using System.Net;
using System.Text;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Extensions.Options;
using Stellarr.Api.Domain;
using Stellarr.Api.Notifications;

namespace Stellarr.Api.Tests;

public sealed class WebPushDeliveryTests
{
    private static readonly NotificationPushPayload Payload = new(
        "delivery-1",
        "Visible title",
        "Visible body",
        "https://chat.example.test/",
        "delivery-1");

    [Fact]
    public void ServiceExtensionRegistersOptionsAndTheTransport()
    {
        var configuration = new ConfigurationBuilder()
            .AddInMemoryCollection(new Dictionary<string, string?>
            {
                ["Push:Enabled"] = "true",
                ["Push:Subject"] = "mailto:push@example.test",
                ["Push:PublicKey"] = "public",
                ["Push:PrivateKey"] = "private",
            })
            .Build();
        var services = new ServiceCollection().AddLogging();
        services.AddWebPushNotifications(configuration);

        using var provider = services.BuildServiceProvider();
        var options = provider.GetRequiredService<IOptions<PushOptions>>().Value;
        Assert.True(options.Enabled);
        Assert.Equal("mailto:push@example.test", options.Subject);
        Assert.IsType<WebPushDelivery>(provider.GetRequiredService<IWebPushDelivery>());
    }

    [Fact]
    public async Task SendsEncryptedCamelCasePayloadWithVapidHeaders()
    {
        var vapid = PushTestKeys.Vapid();
        var subscription = PushTestKeys.Subscription();
        var handler = new CapturingHandler((_, _) =>
            Task.FromResult(new HttpResponseMessage(HttpStatusCode.Created)));
        using var client = new HttpClient(handler);
        var delivery = CreateDelivery(client, vapid, subscription);

        var result = await delivery.SendAsync(Record(subscription), Payload, CancellationToken.None);

        Assert.Equal(PushDeliveryResult.Delivered, result);
        var request = Assert.IsType<CapturedRequest>(handler.Request);
        Assert.Equal(HttpMethod.Post, request.Method);
        Assert.Equal("push.example.test", request.Uri.Host);
        Assert.Equal("300", request.Headers["TTL"]);
        Assert.StartsWith("WebPush ", request.Headers["Authorization"], StringComparison.Ordinal);
        Assert.Contains("dh=", request.Headers["Crypto-Key"], StringComparison.Ordinal);
        Assert.Contains(
            "p256ecdsa=" + vapid.PublicKey,
            request.Headers["Crypto-Key"],
            StringComparison.Ordinal);
        Assert.StartsWith("salt=", request.Headers["Encryption"], StringComparison.Ordinal);
        Assert.Equal("aesgcm", request.ContentHeaders["Content-Encoding"]);
        Assert.Equal("application/octet-stream", request.ContentHeaders["Content-Type"]);
        Assert.True(request.Body.Length > 32);
        Assert.DoesNotContain("Visible title", Encoding.UTF8.GetString(request.Body), StringComparison.Ordinal);
        Assert.DoesNotContain("Visible body", Encoding.UTF8.GetString(request.Body), StringComparison.Ordinal);
    }

    [Theory]
    [InlineData(200, PushDeliveryResult.Delivered)]
    [InlineData(201, PushDeliveryResult.Delivered)]
    [InlineData(204, PushDeliveryResult.Delivered)]
    [InlineData(400, PushDeliveryResult.Gone)]
    [InlineData(302, PushDeliveryResult.Gone)]
    [InlineData(404, PushDeliveryResult.Gone)]
    [InlineData(410, PushDeliveryResult.Gone)]
    [InlineData(413, PushDeliveryResult.Gone)]
    [InlineData(401, PushDeliveryResult.Retry)]
    [InlineData(403, PushDeliveryResult.Retry)]
    [InlineData(408, PushDeliveryResult.Retry)]
    [InlineData(429, PushDeliveryResult.Retry)]
    [InlineData(500, PushDeliveryResult.Retry)]
    [InlineData(503, PushDeliveryResult.Retry)]
    public async Task MapsProviderStatusToDeliveryResult(int status, PushDeliveryResult expected)
    {
        var vapid = PushTestKeys.Vapid();
        var subscription = PushTestKeys.Subscription();
        var handler = new CapturingHandler((_, _) =>
            Task.FromResult(new HttpResponseMessage((HttpStatusCode)status)));
        using var client = new HttpClient(handler);
        var delivery = CreateDelivery(client, vapid, subscription);

        var result = await delivery.SendAsync(Record(subscription), Payload, CancellationToken.None);

        Assert.Equal(expected, result);
    }

    [Fact]
    public async Task NetworkFailureIsRetryable()
    {
        var vapid = PushTestKeys.Vapid();
        var subscription = PushTestKeys.Subscription();
        var handler = new CapturingHandler((_, _) => throw new HttpRequestException("offline"));
        using var client = new HttpClient(handler);
        var delivery = CreateDelivery(client, vapid, subscription);

        Assert.Equal(
            PushDeliveryResult.Retry,
            await delivery.SendAsync(Record(subscription), Payload, CancellationToken.None));
    }

    [Fact]
    public async Task HttpTimeoutIsRetryable()
    {
        var vapid = PushTestKeys.Vapid();
        var subscription = PushTestKeys.Subscription();
        var handler = new CapturingHandler(async (_, cancellationToken) =>
        {
            await Task.Delay(Timeout.Infinite, cancellationToken);
            return new HttpResponseMessage(HttpStatusCode.OK);
        });
        using var client = new HttpClient(handler) { Timeout = TimeSpan.FromMilliseconds(50) };
        var delivery = CreateDelivery(client, vapid, subscription);

        Assert.Equal(
            PushDeliveryResult.Retry,
            await delivery.SendAsync(Record(subscription), Payload, CancellationToken.None));
    }

    [Fact]
    public async Task CallerCancellationIsNotTurnedIntoRetry()
    {
        var vapid = PushTestKeys.Vapid();
        var subscription = PushTestKeys.Subscription();
        var entered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var handler = new CapturingHandler(async (_, cancellationToken) =>
        {
            entered.TrySetResult();
            await Task.Delay(Timeout.Infinite, cancellationToken);
            return new HttpResponseMessage(HttpStatusCode.OK);
        });
        using var client = new HttpClient(handler);
        var delivery = CreateDelivery(client, vapid, subscription);
        using var cancellation = new CancellationTokenSource();

        var pending = delivery.SendAsync(Record(subscription), Payload, cancellation.Token);
        await entered.Task.WaitAsync(TimeSpan.FromSeconds(5));
        cancellation.Cancel();

        await Assert.ThrowsAnyAsync<OperationCanceledException>(
            () => pending.WaitAsync(TimeSpan.FromSeconds(5)));
    }

    [Fact]
    public async Task InvalidSubscriptionKeysAreGoneWithoutAnAttempt()
    {
        var vapid = PushTestKeys.Vapid();
        var valid = PushTestKeys.Subscription();
        var handler = new CapturingHandler((_, _) =>
            Task.FromResult(new HttpResponseMessage(HttpStatusCode.Created)));
        using var client = new HttpClient(handler);
        var delivery = CreateDelivery(client, vapid, valid);
        var broken = new PushSubscriptionRecord
        {
            Id = Guid.NewGuid(),
            UserId = Guid.NewGuid(),
            SessionId = Guid.NewGuid(),
            Endpoint = "https://push.example.test/send/keys",
            P256dh = "not-a-public-key",
            Auth = valid.Auth,
            CreatedAt = DateTime.UtcNow,
        };

        Assert.Equal(
            PushDeliveryResult.Gone,
            await delivery.SendAsync(broken, Payload, CancellationToken.None));
        Assert.Null(handler.Request);
    }

    [Fact]
    public async Task PrivateOrMalformedEndpointIsGoneWithoutAnAttempt()
    {
        var vapid = PushTestKeys.Vapid();
        var subscription = PushTestKeys.Subscription();
        var handler = new CapturingHandler((_, _) =>
            Task.FromResult(new HttpResponseMessage(HttpStatusCode.Created)));
        using var client = new HttpClient(handler);
        var delivery = CreateDelivery(client, vapid, subscription);
        var record = Record(subscription);
        record.Endpoint = "https://127.0.0.1/send/private";

        Assert.Equal(
            PushDeliveryResult.Gone,
            await delivery.SendAsync(record, Payload, CancellationToken.None));
        Assert.Null(handler.Request);
    }

    [Fact]
    public async Task OversizedPayloadIsNotSent()
    {
        var vapid = PushTestKeys.Vapid();
        var subscription = PushTestKeys.Subscription();
        var handler = new CapturingHandler((_, _) =>
            Task.FromResult(new HttpResponseMessage(HttpStatusCode.Created)));
        using var client = new HttpClient(handler);
        var delivery = CreateDelivery(client, vapid, subscription);
        var payload = Payload with { Body = new string('x', WebPushDelivery.MaxPayloadBytes * 2) };

        Assert.Equal(
            PushDeliveryResult.Gone,
            await delivery.SendAsync(Record(subscription), payload, CancellationToken.None));
        Assert.Null(handler.Request);
    }

    [Fact]
    public async Task MissingConfigurationRetriesWithoutAnAttempt()
    {
        var subscription = PushTestKeys.Subscription();
        var handler = new CapturingHandler((_, _) =>
            Task.FromResult(new HttpResponseMessage(HttpStatusCode.Created)));
        using var client = new HttpClient(handler);
        var delivery = new WebPushDelivery(
            client,
            new PushOptions { Enabled = false },
            NullLogger<WebPushDelivery>.Instance);

        Assert.Equal(
            PushDeliveryResult.Retry,
            await delivery.SendAsync(Record(subscription), Payload, CancellationToken.None));
        Assert.Null(handler.Request);
    }

    [Theory]
    [InlineData("https://127.0.0.1/send")]
    [InlineData("https://[::1]/send")]
    [InlineData("https://localhost/send")]
    [InlineData("http://push.example.test/send")]
    [InlineData("https://push.example.test:8443/send")]
    public void EndpointValidationRefusesLocalOrNonDefaultEndpoints(string endpoint)
    {
        var subscription = PushTestKeys.Subscription();
        Assert.False(PushSubscriptionValidation.TryValidate(
            endpoint,
            subscription.P256dh,
            subscription.Auth,
            out _));
    }

    [Theory]
    [InlineData("https://127.0.0.1:9/send")]
    [InlineData("https://[::1]:9/send")]
    [InlineData("https://localhost:9/send")]
    public async Task ConnectionHandlerRefusesPrivateAddressAtConnect(string endpoint)
    {
        using var invoker = new HttpMessageInvoker(WebPushDelivery.CreateHandler());

        await Assert.ThrowsAsync<HttpRequestException>(
            () => invoker.SendAsync(new HttpRequestMessage(HttpMethod.Post, endpoint), CancellationToken.None));
    }

    private static WebPushDelivery CreateDelivery(
        HttpClient client,
        (string PublicKey, string PrivateKey) vapid,
        (string P256dh, string Auth, byte[] AuthBytes) subscription)
    {
        _ = subscription;
        return new WebPushDelivery(
            client,
            new PushOptions
            {
                Enabled = true,
                Subject = "mailto:push@example.test",
                PublicKey = vapid.PublicKey,
                PrivateKey = vapid.PrivateKey,
            },
            NullLogger<WebPushDelivery>.Instance);
    }

    private static PushSubscriptionRecord Record(
        (string P256dh, string Auth, byte[] AuthBytes) subscription) => new()
    {
        Id = Guid.NewGuid(),
        UserId = Guid.NewGuid(),
        SessionId = Guid.NewGuid(),
        Endpoint = "https://push.example.test/send/subscription",
        P256dh = subscription.P256dh,
        Auth = subscription.Auth,
        CreatedAt = DateTime.UtcNow,
    };

    private sealed record CapturedRequest(
        HttpMethod Method,
        Uri Uri,
        IReadOnlyDictionary<string, string> Headers,
        IReadOnlyDictionary<string, string> ContentHeaders,
        byte[] Body);

    private sealed class CapturingHandler(
        Func<HttpRequestMessage, CancellationToken, Task<HttpResponseMessage>> respond) : HttpMessageHandler
    {
        public CapturedRequest? Request { get; private set; }

        protected override async Task<HttpResponseMessage> SendAsync(
            HttpRequestMessage request,
            CancellationToken cancellationToken)
        {
            var body = request.Content is null
                ? []
                : await request.Content.ReadAsByteArrayAsync(cancellationToken);
            Request = new CapturedRequest(
                request.Method,
                request.RequestUri!,
                request.Headers.ToDictionary(
                    header => header.Key,
                    header => string.Join(", ", header.Value),
                    StringComparer.OrdinalIgnoreCase),
                request.Content?.Headers.ToDictionary(
                    header => header.Key,
                    header => string.Join(", ", header.Value),
                    StringComparer.OrdinalIgnoreCase) ?? new Dictionary<string, string>(),
                body);
            return await respond(request, cancellationToken);
        }
    }
}
