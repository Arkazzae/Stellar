using System.Globalization;
using System.Net.Http.Headers;

namespace Stellarr.Api.Integrations.Discord;

/// <summary>
/// What Discord said about its rate limits, kept so the next request waits
/// instead of being refused.
/// </summary>
/// <remarks>
/// Discord groups routes into buckets it names in <c>X-RateLimit-Bucket</c>,
/// and counts each bucket separately for every top-level resource (channel or
/// server) in the path. A route's bucket is learnt from its first answer;
/// until then the route counts as its own bucket. The global limit covers
/// every route at once. Times are kept as moments of the given clock, from
/// <c>X-RateLimit-Reset-After</c>, a duration, rather than the epoch in
/// <c>X-RateLimit-Reset</c>, which would trust this machine's clock to agree
/// with Discord's. The reader is meant to be used one request at a time, but
/// a lock keeps the record whole should two tasks share it.
/// </remarks>
internal sealed class DiscordRateLimits
{
    private readonly Lock _gate = new();
    private readonly Dictionary<string, string> _bucketOfRoute = new(StringComparer.Ordinal);
    private readonly Dictionary<string, DateTimeOffset> _exhaustedUntil = new(StringComparer.Ordinal);
    private DateTimeOffset _globalUntil = DateTimeOffset.MinValue;

    /// <summary>How long to wait before sending a request on <paramref name="route"/>.</summary>
    public TimeSpan DelayBefore(DiscordRoute route, DateTimeOffset now)
    {
        lock (_gate)
        {
            var until = _globalUntil;
            if (_exhaustedUntil.TryGetValue(Key(route), out var bucketUntil) && bucketUntil > until)
            {
                until = bucketUntil;
            }

            return until > now ? until - now : TimeSpan.Zero;
        }
    }

    /// <summary>Reads the headers of any answer: which bucket the route is in, and whether it has run out.</summary>
    public void Observe(DiscordRoute route, HttpResponseHeaders headers, DateTimeOffset now)
    {
        var bucket = Header(headers, "X-RateLimit-Bucket");
        var remainingText = Header(headers, "X-RateLimit-Remaining");
        var reset = ResetAfter(headers, now);
        lock (_gate)
        {
            if (bucket is { Length: > 0 })
            {
                _bucketOfRoute[route.Shape] = bucket;
            }

            var key = Key(route);
            if (int.TryParse(remainingText, NumberStyles.Integer, CultureInfo.InvariantCulture, out var remaining))
            {
                if (remaining > 0)
                {
                    _exhaustedUntil.Remove(key);
                }
                else if (reset is { } after)
                {
                    _exhaustedUntil[key] = now + after;
                }
            }
        }
    }

    /// <summary>Records a 429: the bucket, or with <paramref name="global"/> every route, waits <paramref name="retryAfter"/>.</summary>
    public void Limited(DiscordRoute route, TimeSpan retryAfter, bool global, DateTimeOffset now)
    {
        var until = now + retryAfter;
        lock (_gate)
        {
            if (!global)
            {
                _exhaustedUntil[Key(route)] = until;
            }
            else if (until > _globalUntil)
            {
                _globalUntil = until;
            }
        }
    }

    /// <summary>The wait a 429 asks for: its body, then <c>Retry-After</c>, then the bucket's reset.</summary>
    public static TimeSpan? RetryAfter(double? bodySeconds, HttpResponseHeaders headers, DateTimeOffset now)
    {
        if (bodySeconds is { } seconds && double.IsFinite(seconds) && seconds >= 0)
        {
            return Seconds(seconds);
        }

        if (headers.RetryAfter?.Delta is { } delta)
        {
            return delta;
        }

        if (headers.RetryAfter?.Date is { } date)
        {
            return date > now ? date - now : TimeSpan.Zero;
        }

        return ResetAfter(headers, now);
    }

    public static bool IsGlobal(HttpResponseHeaders headers) =>
        string.Equals(Header(headers, "X-RateLimit-Global"), "true", StringComparison.OrdinalIgnoreCase) ||
        string.Equals(Header(headers, "X-RateLimit-Scope"), "global", StringComparison.OrdinalIgnoreCase);

    private string Key(DiscordRoute route) =>
        (_bucketOfRoute.TryGetValue(route.Shape, out var bucket) ? bucket : route.Shape) + "|" + route.Major;

    private static TimeSpan? ResetAfter(HttpResponseHeaders headers, DateTimeOffset now)
    {
        if (Header(headers, "X-RateLimit-Reset-After") is { } after &&
            double.TryParse(after, NumberStyles.Float, CultureInfo.InvariantCulture, out var seconds) &&
            double.IsFinite(seconds) && seconds >= 0)
        {
            return Seconds(seconds);
        }

        if (Header(headers, "X-RateLimit-Reset") is { } reset &&
            double.TryParse(reset, NumberStyles.Float, CultureInfo.InvariantCulture, out var epoch) &&
            double.IsFinite(epoch))
        {
            var at = DateTimeOffset.UnixEpoch + Seconds(Math.Clamp(epoch, 0, 253_402_300_799));
            return at > now ? at - now : TimeSpan.Zero;
        }

        return null;
    }

    /// <summary>Seconds as a duration, capped at a day: no answer Discord gives means a longer wait.</summary>
    private static TimeSpan Seconds(double seconds) => TimeSpan.FromSeconds(Math.Min(seconds, 86_400));

    private static string? Header(HttpResponseHeaders headers, string name) =>
        headers.TryGetValues(name, out var values) ? values.FirstOrDefault()?.Trim() : null;
}

/// <summary>
/// One request's address, relative to the API's base, and what its rate limit
/// is counted by.
/// </summary>
/// <param name="Path">The path and query, such as <c>channels/1/messages?limit=100</c>.</param>
/// <param name="Shape">
/// The route with placeholders for its ids, such as
/// <c>channels/{channel}/messages</c>: Discord's bucket is learnt per shape.
/// </param>
/// <param name="Major">The channel or server id Discord counts the route's limit separately for.</param>
internal readonly record struct DiscordRoute(string Path, string Shape, string Major)
{
    /// <summary>The method and path without the query, as errors and logs name the route.</summary>
    public override string ToString() =>
        "GET " + (Path.IndexOf('?', StringComparison.Ordinal) is var query and >= 0 ? Path[..query] : Path);
}
