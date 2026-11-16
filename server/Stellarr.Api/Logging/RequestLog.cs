using System.Diagnostics;

namespace Stellarr.Api.Logging;

/// <summary>
/// One line for every request the API serves: its method, path, status and
/// duration, at information level under <see cref="Category"/>. It takes the
/// place of the six lines ASP.NET Core writes for a request at that level, the
/// request starting and finished, the endpoint executing and executed, and the
/// result's status and body, which appsettings.json logs only from Warning. The
/// query string is left out, as it can carry a token. The health probes write
/// nothing, see <see cref="HealthProbeLogging"/>.
/// </summary>
public static partial class RequestLog
{
    /// <summary>The category of the request lines, which a log level can be set for.</summary>
    public const string Category = "Stellarr.Api.Requests";

    /// <summary>
    /// Logs each request once it has an answer. Placed outside the exception
    /// handler, so that a request that failed is logged with the 500 it got.
    /// </summary>
    public static IApplicationBuilder UseRequestLog(this IApplicationBuilder app)
    {
        var logger = app.ApplicationServices.GetRequiredService<ILoggerFactory>().CreateLogger(Category);
        return app.Use(async (context, next) =>
        {
            var started = Stopwatch.GetTimestamp();
            try
            {
                await next(context);
            }
            finally
            {
                // Off for a health probe, and when the category is set to
                // Warning or above.
                if (logger.IsEnabled(LogLevel.Information))
                {
                    var path = context.Request.PathBase.Add(context.Request.Path);
                    var milliseconds = (long)Stopwatch.GetElapsedTime(started).TotalMilliseconds;
                    Served(logger, context.Request.Method, path, context.Response.StatusCode, milliseconds);
                }
            }
        });
    }

    [LoggerMessage(EventId = 1, Level = LogLevel.Information, Message = "{Method} {Path} {StatusCode} in {Milliseconds} ms")]
    private static partial void Served(ILogger logger, string method, PathString path, int statusCode, long milliseconds);
}
