using System.Net;
using System.Text;
using Microsoft.Extensions.Logging;
using Stellarr.Api.Integrations.Discord;

namespace Stellarr.Api.Tests;

/// <summary>
/// Discord's API as a function of the request, so tests run without a
/// network. Every request is recorded as it was sent, headers included.
/// </summary>
internal sealed class DiscordFakeApi : HttpMessageHandler
{
    private readonly Func<SentRequest, HttpResponseMessage> _respond;
    private readonly List<SentRequest> _requests = [];

    public DiscordFakeApi(Func<SentRequest, HttpResponseMessage> respond) => _respond = respond;

    /// <summary>Answers requests in turn from a list, whatever they ask.</summary>
    public DiscordFakeApi(params HttpResponseMessage[] answers)
    {
        var queue = new Queue<HttpResponseMessage>(answers);
        _respond = _ => queue.Count > 0 ? queue.Dequeue() : throw new InvalidOperationException("No more answers.");
    }

    public IReadOnlyList<SentRequest> Requests
    {
        get
        {
            lock (_requests) return [.. _requests];
        }
    }

    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
    {
        var sent = new SentRequest(
            request.Method,
            request.RequestUri!,
            request.Headers.Authorization?.ToString(),
            request.Headers.UserAgent.ToString());
        lock (_requests) _requests.Add(sent);
        return Task.FromResult(_respond(sent));
    }

    public static HttpResponseMessage Json(string json, HttpStatusCode status = HttpStatusCode.OK, params (string Name, string Value)[] headers)
    {
        var response = new HttpResponseMessage(status) { Content = new StringContent(json, Encoding.UTF8, "application/json") };
        foreach (var (name, value) in headers)
        {
            response.Headers.TryAddWithoutValidation(name, value);
        }

        return response;
    }

    public static HttpResponseMessage Error(HttpStatusCode status, int code, string message) =>
        Json($$"""{"message":"{{message}}","code":{{code}}}""", status);

    public static HttpResponseMessage TooMany(double retryAfter, bool global = false, params (string Name, string Value)[] headers) =>
        Json($$"""{"message":"You are being rate limited.","retry_after":{{retryAfter.ToString(System.Globalization.CultureInfo.InvariantCulture)}},"global":{{(global ? "true" : "false")}}}""",
            HttpStatusCode.TooManyRequests, headers);
}

internal sealed record SentRequest(HttpMethod Method, Uri Uri, string? Authorization, string UserAgent)
{
    public string PathAndQuery => Uri.PathAndQuery;

    public string? Query(string name) =>
        System.Web.HttpUtility.ParseQueryString(Uri.Query)[name];
}

/// <summary>
/// A clock for code that waits through <c>Task.Delay(delay, timeProvider)</c>:
/// every timer fires at once and moves the clock on by its delay, so a test
/// sees each wait without sitting through it. Held, timers never fire.
/// </summary>
internal sealed class DiscordTestClock : TimeProvider
{
    private readonly object _gate = new();
    private readonly List<TimeSpan> _waits = [];
    private DateTimeOffset _now = new(2026, 11, 23, 12, 0, 0, TimeSpan.Zero);

    /// <summary>When set, a timer records its wait and never fires, so the waiter stays waiting.</summary>
    public bool Hold { get; set; }

    public IReadOnlyList<TimeSpan> Waits
    {
        get
        {
            lock (_gate) return [.. _waits];
        }
    }

    public override DateTimeOffset GetUtcNow()
    {
        lock (_gate) return _now;
    }

    public void Advance(TimeSpan by)
    {
        lock (_gate) _now += by;
    }

    public override ITimer CreateTimer(TimerCallback callback, object? state, TimeSpan dueTime, TimeSpan period)
    {
        lock (_gate)
        {
            _waits.Add(dueTime);
            if (!Hold)
            {
                _now += dueTime;
            }
        }

        if (!Hold)
        {
            // Not on this thread: the caller has not stored the timer yet.
            ThreadPool.QueueUserWorkItem(_ => callback(state));
        }

        return new SpentTimer();
    }

    private sealed class SpentTimer : ITimer
    {
        public bool Change(TimeSpan dueTime, TimeSpan period) => false;

        public void Dispose()
        {
        }

        public ValueTask DisposeAsync() => ValueTask.CompletedTask;
    }
}

/// <summary>Keeps every line logged, with its exception, so a test can look for what must not be there.</summary>
internal sealed class DiscordCapturingLogger : ILogger<DiscordClient>
{
    private readonly List<string> _lines = [];

    public IReadOnlyList<string> Lines
    {
        get
        {
            lock (_lines) return [.. _lines];
        }
    }

    public IDisposable? BeginScope<TState>(TState state) where TState : notnull => null;

    public bool IsEnabled(LogLevel logLevel) => true;

    public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter)
    {
        lock (_lines) _lines.Add($"{logLevel} {formatter(state, exception)} {exception}");
    }
}
