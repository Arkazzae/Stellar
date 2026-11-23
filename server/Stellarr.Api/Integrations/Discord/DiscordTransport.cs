using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Text;
using System.Text.Json;
using Stellarr.Api.Embeds;

namespace Stellarr.Api.Integrations.Discord;

/// <summary>
/// Sends one GET request at a time to Discord's API and turns the answer into
/// a value or a <see cref="DiscordApiException"/>, waiting out rate limits in
/// between.
/// </summary>
/// <remarks>
/// One transport serves one token (or none, for templates), because Discord
/// counts limits per bot. Requests are sequential: an import reads one page
/// after another, and Discord's limits are per bot anyway, so a parallel
/// reader would only queue up behind the same buckets.
/// </remarks>
internal sealed partial class DiscordTransport(
    HttpClient http,
    TimeProvider time,
    ILogger logger,
    DiscordRequestPolicy policy,
    DiscordBotToken? token)
{
    /// <summary>Version 10 of the API, always on this host: no address in a request comes from outside.</summary>
    internal static readonly Uri ApiBase = new("https://discord.com/api/v10/");

    /// <summary>The form Discord requires, without which it may refuse the request.</summary>
    internal const string UserAgent = "DiscordBot (https://github.com/arkazz/stellarr, 2.0)";

    /// <summary>More than any error body Discord sends; a longer one is cut, not read whole.</summary>
    private const int MaxErrorBytes = 64 * 1024;

    private readonly DiscordRateLimits _limits = new();

    public async Task<T> GetAsync<T>(DiscordRoute route, Func<int?, bool>? missingIntent, CancellationToken cancellationToken)
    {
        var rateLimited = 0;
        var serverErrors = 0;
        while (true)
        {
            var wait = _limits.DelayBefore(route, time.GetUtcNow());
            if (wait > TimeSpan.Zero)
            {
                if (wait > policy.MaxWait)
                {
                    throw RateLimited(route, wait);
                }

                LogWaiting(logger, wait.TotalSeconds, route);
                await Task.Delay(wait, time, cancellationToken);
            }

            using var request = new HttpRequestMessage(HttpMethod.Get, new Uri(ApiBase, route.Path));
            request.Headers.UserAgent.ParseAdd(UserAgent);
            request.Headers.Accept.ParseAdd("application/json");
            if (token is not null)
            {
                request.Headers.Authorization = new AuthenticationHeaderValue("Bot", token.Value);
            }

            HttpResponseMessage response;
            try
            {
                response = await http.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, cancellationToken);
            }
            catch (HttpRequestException exception)
            {
                throw Failure(DiscordFailureKind.Unreachable, route, $"could not reach Discord ({exception.HttpRequestError})", exception);
            }
            catch (TaskCanceledException exception) when (!cancellationToken.IsCancellationRequested)
            {
                // HttpClient reports its own timeout as a cancellation.
                throw Failure(DiscordFailureKind.Unreachable, route, "Discord did not answer in time", exception);
            }

            using (response)
            {
                var now = time.GetUtcNow();
                _limits.Observe(route, response.Headers, now);
                var status = (int)response.StatusCode;

                if (response.StatusCode == HttpStatusCode.TooManyRequests)
                {
                    var error = await ReadErrorAsync(response, cancellationToken);
                    var retryAfter = DiscordRateLimits.RetryAfter(error?.RetryAfter, response.Headers, now) ??
                        policy.UnstatedRetryAfter;
                    var global = error?.Global == true || DiscordRateLimits.IsGlobal(response.Headers);
                    _limits.Limited(route, retryAfter, global, now);
                    rateLimited++;
                    if (rateLimited > policy.MaxRateLimitRetries || retryAfter > policy.MaxWait)
                    {
                        throw RateLimited(route, retryAfter);
                    }

                    LogLimited(logger, route, retryAfter.TotalSeconds, global);
                    continue;
                }

                if (status >= 500)
                {
                    serverErrors++;
                    if (serverErrors > policy.MaxServerErrorRetries)
                    {
                        throw Failure(DiscordFailureKind.Unreachable, route, $"Discord failed with HTTP {status}", statusCode: status);
                    }

                    await Task.Delay(policy.ServerErrorPause * serverErrors, time, cancellationToken);
                    continue;
                }

                if (response.IsSuccessStatusCode)
                {
                    return await ReadAsync<T>(route, response, cancellationToken);
                }

                throw Refused(route, status, await ReadErrorAsync(response, cancellationToken), missingIntent);
            }
        }
    }

    private async Task<T> ReadAsync<T>(DiscordRoute route, HttpResponseMessage response, CancellationToken cancellationToken)
    {
        try
        {
            return await response.Content.ReadFromJsonAsync<T>(DiscordJson.Options, cancellationToken) ??
                throw Failure(DiscordFailureKind.UnexpectedResponse, route, "Discord answered with an empty body", statusCode: (int)response.StatusCode);
        }
        catch (JsonException exception)
        {
            throw Failure(DiscordFailureKind.UnexpectedResponse, route, "Discord answered with JSON of another shape",
                exception, (int)response.StatusCode);
        }
        catch (NotSupportedException exception)
        {
            // ReadFromJsonAsync's way of saying the content type is not JSON.
            throw Failure(DiscordFailureKind.UnexpectedResponse, route, "Discord answered with something other than JSON",
                exception, (int)response.StatusCode);
        }
        catch (Exception exception) when (exception is HttpRequestException or IOException ||
            (exception is OperationCanceledException && !cancellationToken.IsCancellationRequested))
        {
            throw Failure(DiscordFailureKind.Unreachable, route, "the answer from Discord broke off", exception, (int)response.StatusCode);
        }
    }

    private DiscordApiException Refused(DiscordRoute route, int status, WireError? error, Func<int?, bool>? missingIntent)
    {
        var kind = status switch
        {
            401 => DiscordFailureKind.InvalidToken,
            403 when missingIntent?.Invoke(error?.Code) == true => DiscordFailureKind.MissingIntent,
            403 => DiscordFailureKind.MissingAccess,
            404 => DiscordFailureKind.UnknownResource,
            _ => DiscordFailureKind.UnexpectedResponse,
        };
        var said = error?.Message is { Length: > 0 } message ? $": {message}" : "";
        var code = error?.Code is { } number ? $" (code {number})" : "";
        return new DiscordApiException(
            kind,
            Redact($"Discord refused {route}: HTTP {status}{code}{said}"),
            route.ToString(),
            status,
            error?.Code);
    }

    private static DiscordApiException RateLimited(DiscordRoute route, TimeSpan retryAfter) => new(
        DiscordFailureKind.RateLimited,
        $"Discord asked to wait {retryAfter.TotalSeconds:0.###} s before {route}, longer than the import waits",
        route.ToString(),
        429,
        retryAfter: retryAfter);

    private DiscordApiException Failure(
        DiscordFailureKind kind, DiscordRoute route, string what, Exception? inner = null, int? statusCode = null) =>
        new(kind, Redact($"{route} failed: {what}"), route.ToString(), statusCode, innerException: inner);

    /// <summary>
    /// Reads Discord's error body, if it is one. A 429 from Discord's edge
    /// rather than its API may be HTML; that reads as no body.
    /// </summary>
    private static async Task<WireError?> ReadErrorAsync(HttpResponseMessage response, CancellationToken cancellationToken)
    {
        try
        {
            var bytes = await CappedBody.ReadAsync(response.Content, MaxErrorBytes, allowTruncated: true, cancellationToken);
            return bytes is { Length: > 0 } ? JsonSerializer.Deserialize<WireError>(bytes, DiscordJson.Options) : null;
        }
        catch (Exception exception) when (exception is JsonException or HttpRequestException or IOException)
        {
            return null;
        }
    }

    /// <summary>Discord's own text goes into messages; it should never hold the token, and this makes sure.</summary>
    private string Redact(string text) =>
        token is null || token.Value.Length == 0
            ? text
            : new StringBuilder(text).Replace(token.Value, "[token]").ToString();

    [LoggerMessage(Level = LogLevel.Information, Message = "Waiting {Seconds} s for Discord's rate limit before {Route}")]
    private static partial void LogWaiting(ILogger logger, double seconds, DiscordRoute route);

    [LoggerMessage(Level = LogLevel.Information, Message = "Discord rate-limited {Route} for {Seconds} s (global: {Global})")]
    private static partial void LogLimited(ILogger logger, DiscordRoute route, double seconds, bool global);
}

/// <summary>How long the reader waits for Discord before it gives up and says so.</summary>
internal sealed record DiscordRequestPolicy
{
    public static readonly DiscordRequestPolicy Default = new();

    /// <summary>
    /// The longest single wait. Discord can ask for twenty minutes on a shared
    /// resource; an import job would rather hear that and decide than sit in
    /// one request for it.
    /// </summary>
    public TimeSpan MaxWait { get; init; } = TimeSpan.FromMinutes(1);

    /// <summary>How many 429s in a row one request may get before it fails.</summary>
    public int MaxRateLimitRetries { get; init; } = 5;

    /// <summary>The wait after a 429 that says nothing about how long, as an edge's HTML page may.</summary>
    public TimeSpan UnstatedRetryAfter { get; init; } = TimeSpan.FromSeconds(5);

    /// <summary>How many times a 5xx is tried again, after one pause, then two.</summary>
    public int MaxServerErrorRetries { get; init; } = 2;

    public TimeSpan ServerErrorPause { get; init; } = TimeSpan.FromSeconds(1);
}
