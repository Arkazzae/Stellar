using System.Net;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Stellarr.Api.Domain;
using Stellarr.Api.Embeds;
using WebPush;
using WebPush.Model;

namespace Stellarr.Api.Notifications;

/// <summary>
/// Sends Web Push notifications through the vetted <c>WebPush</c> library.
/// Encryption and VAPID signing are the library's, not this class's.
/// </summary>
/// <remarks>
/// The outbox that owns retries only needs to know whether one attempt was
/// accepted, is worth retrying, or should drop the subscription. An endpoint is
/// resolved and connected through <see cref="CreateHandler"/>, so a name that
/// points into the local network is refused at the socket rather than after the
/// request has been encrypted for it.
/// </remarks>
public sealed class WebPushDelivery : IWebPushDelivery
{
    internal const string HttpClientName = "web-push";
    internal const int MaxPayloadBytes = 3_072;
    internal const int TimeToLiveSeconds = 300;

    private static readonly JsonSerializerOptions PayloadJsonOptions = new(JsonSerializerDefaults.Web)
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
    };

    private static readonly Action<ILogger, Guid, int, Exception?> DeliveryRefused = LoggerMessage.Define<Guid, int>(
        LogLevel.Warning,
        new EventId(1, "WebPushDeliveryRefused"),
        "A web push delivery for subscription {SubscriptionId} was refused with status {StatusCode}.");

    private static readonly Action<ILogger, Guid, Exception?> DeliveryFailed = LoggerMessage.Define<Guid>(
        LogLevel.Warning,
        new EventId(2, "WebPushDeliveryFailed"),
        "A web push delivery for subscription {SubscriptionId} could not reach its push service.");

    private static readonly Action<ILogger, Guid, Exception?> DeliverySkipped = LoggerMessage.Define<Guid>(
        LogLevel.Warning,
        new EventId(3, "WebPushDeliverySkipped"),
        "A web push delivery for subscription {SubscriptionId} was skipped because Web Push is not configured.");

    private readonly HttpClient _httpClient;
    private readonly ILogger<WebPushDelivery> _logger;
    private readonly PushVapidConfiguration? _vapid;
    private int _unavailableLogged;

    internal WebPushDelivery(HttpClient httpClient, PushOptions options, ILogger<WebPushDelivery> logger)
    {
        _httpClient = httpClient;
        _logger = logger;
        _vapid = PushVapidConfiguration.From(options);
    }

    public async Task<PushDeliveryResult> SendAsync(
        PushSubscriptionRecord subscription,
        NotificationPushPayload payload,
        CancellationToken cancellationToken)
    {
        if (_vapid is null)
        {
            if (Interlocked.Exchange(ref _unavailableLogged, 1) == 0)
            {
                DeliverySkipped(_logger, subscription.Id, null);
            }

            return PushDeliveryResult.Retry;
        }

        // Checked again at delivery, not only when the row was written: an old
        // or altered row must not make this process fetch an internal address.
        if (!PushSubscriptionValidation.TryValidate(
                subscription.Endpoint,
                subscription.P256dh,
                subscription.Auth,
                out var validated))
        {
            return PushDeliveryResult.Gone;
        }

        byte[] body;
        try
        {
            var json = JsonSerializer.Serialize(payload, PayloadJsonOptions);
            body = Encoding.UTF8.GetBytes(json);
        }
        catch (NotSupportedException)
        {
            return PushDeliveryResult.Gone;
        }

        if (body.Length > MaxPayloadBytes)
        {
            DeliveryRefused(_logger, subscription.Id, StatusCodes.Status413PayloadTooLarge, null);
            // The subscription is still healthy; a payload that is too large is
            // a problem for this notification, not a reason to drop the browser.
            return PushDeliveryResult.Retry;
        }

        try
        {
            var client = new WebPushClient(_httpClient);
            var options = new Dictionary<string, object>
            {
                ["TTL"] = TimeToLiveSeconds,
                ["vapidDetails"] = _vapid.Details,
            };
            using var request = client.GenerateRequestDetails(
                new WebPush.PushSubscription(validated.Endpoint, validated.P256dh, validated.Auth),
                Encoding.UTF8.GetString(body),
                options);

            using var deadline = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            deadline.CancelAfter(TimeSpan.FromSeconds(10));
            using var response = await _httpClient.SendAsync(
                request,
                HttpCompletionOption.ResponseHeadersRead,
                deadline.Token);
            return MapStatus(subscription.Id, response.StatusCode);
        }
        catch (InvalidEncryptionDetailsException)
        {
            // The browser's keys are unusable. Retrying the same row can never
            // succeed, so it is retired instead of tried forever.
            return PushDeliveryResult.Gone;
        }
        catch (Exception exception) when (
            exception is FormatException or ArgumentException or CryptographicException)
        {
            return PushDeliveryResult.Gone;
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            throw;
        }
        catch (Exception exception) when (exception is HttpRequestException or IOException or OperationCanceledException)
        {
            DeliveryFailed(_logger, subscription.Id, null);
            return PushDeliveryResult.Retry;
        }
    }

    private PushDeliveryResult MapStatus(Guid subscriptionId, HttpStatusCode statusCode)
    {
        var status = (int)statusCode;
        if (status is >= 200 and <= 299)
        {
            return PushDeliveryResult.Delivered;
        }

        if (status is 404 or 410)
        {
            DeliveryRefused(_logger, subscriptionId, status, null);
            return PushDeliveryResult.Gone;
        }

        // Every other refusal leaves the subscription usable. A VAPID problem
        // (401, 403), a malformed payload (400, 413), a redirect we refuse to
        // follow (3xx), a throttle or a provider outage is retried by the
        // outbox, which bounds its attempts instead of deleting the browser.
        DeliveryRefused(_logger, subscriptionId, status, null);
        return PushDeliveryResult.Retry;
    }

    /// <summary>
    /// The handler used for every delivery: no redirects, cookies or proxy,
    /// and a socket opened only to an address that passed the public check.
    /// </summary>
    public static SocketsHttpHandler CreateHandler() => new()
    {
        AllowAutoRedirect = false,
        UseCookies = false,
        UseProxy = false,
        AutomaticDecompression = DecompressionMethods.None,
        ConnectTimeout = TimeSpan.FromSeconds(5),
        PooledConnectionLifetime = TimeSpan.FromMinutes(2),
        MaxConnectionsPerServer = 4,
        ConnectCallback = ConnectAsync,
    };

    private static async ValueTask<Stream> ConnectAsync(
        SocketsHttpConnectionContext context,
        CancellationToken cancellationToken)
    {
        var host = context.DnsEndPoint.Host;
        var resolved = await Dns.GetHostAddressesAsync(host, cancellationToken);
        var allowed = resolved.Where(PublicAddress.IsPublic).ToArray();
        if (allowed.Length == 0)
        {
            throw new HttpRequestException($"Refusing to connect to {host}: it has no public address.");
        }

        var socket = new Socket(SocketType.Stream, ProtocolType.Tcp) { NoDelay = true };
        try
        {
            await socket.ConnectAsync(allowed, context.DnsEndPoint.Port, cancellationToken);
            return new NetworkStream(socket, ownsSocket: true);
        }
        catch
        {
            socket.Dispose();
            throw;
        }
    }
}
