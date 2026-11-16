using Microsoft.Extensions.DependencyInjection.Extensions;

namespace Stellarr.Api.Logging;

/// <summary>
/// Keeps the health probes out of the log. The container's process manager asks
/// <c>/health/ready</c> every ten seconds, and each request would otherwise write
/// its line from <see cref="RequestLog"/> to the console and to Cockpit's
/// buffer, and ASP.NET Core's six more where its level is raised to information.
/// While a probe is served, nothing below a warning is logged, whatever its
/// category; a warning or an error still is, and a probe that fails is reported
/// by the process manager itself. Every other request logs as before.
/// </summary>
public static class HealthProbeLogging
{
    /// <summary>Whether <paramref name="path"/> is one of the health endpoints.</summary>
    public static bool IsProbe(PathString path) =>
        path.Equals("/health/ready", StringComparison.OrdinalIgnoreCase)
        || path.Equals("/health/live", StringComparison.OrdinalIgnoreCase);

    /// <summary>
    /// Puts a filter in front of every logger the host creates. The logger
    /// factory stays the framework's, with its providers and configured levels;
    /// only what it would pass on while a probe is served is dropped.
    /// </summary>
    public static IServiceCollection AddQuietHealthProbes(this IServiceCollection services)
    {
        services.AddHttpContextAccessor();
        services.RemoveAll<ILoggerFactory>();
        services.TryAddSingleton<LoggerFactory>();
        services.AddSingleton<ILoggerFactory>(provider => new QuietLoggerFactory(
            provider.GetRequiredService<LoggerFactory>(),
            provider.GetRequiredService<IHttpContextAccessor>()));
        return services;
    }

    /// <summary>Whether an entry at <paramref name="level"/> is dropped now.</summary>
    internal static bool Silences(IHttpContextAccessor accessor, LogLevel level) =>
        level < LogLevel.Warning
        && accessor.HttpContext is { } context
        && IsProbe(context.Request.Path);

    /// <summary>
    /// The framework's factory, behind a filter. Disposing it is left to the
    /// container, which owns the factory it wraps.
    /// </summary>
    internal sealed class QuietLoggerFactory(ILoggerFactory inner, IHttpContextAccessor accessor) : ILoggerFactory
    {
        public ILogger CreateLogger(string categoryName) =>
            new QuietLogger(inner.CreateLogger(categoryName), accessor);

        public void AddProvider(ILoggerProvider provider) => inner.AddProvider(provider);

        public void Dispose()
        {
        }
    }

    private sealed class QuietLogger(ILogger inner, IHttpContextAccessor accessor) : ILogger
    {
        public IDisposable? BeginScope<TState>(TState state)
            where TState : notnull => inner.BeginScope(state);

        public bool IsEnabled(LogLevel logLevel) =>
            inner.IsEnabled(logLevel) && !Silences(accessor, logLevel);

        public void Log<TState>(
            LogLevel logLevel,
            EventId eventId,
            TState state,
            Exception? exception,
            Func<TState, Exception?, string> formatter)
        {
            if (!Silences(accessor, logLevel))
            {
                inner.Log(logLevel, eventId, state, exception, formatter);
            }
        }
    }
}
