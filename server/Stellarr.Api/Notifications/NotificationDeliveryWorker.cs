namespace Stellarr.Api.Notifications;

/// <summary>
/// Runs the activity outbox on its own interval. A tick takes a fresh scope,
/// so a transient failure cannot poison the dispatcher, and cancellation is
/// handled without logging it as an error.
/// </summary>
public sealed class NotificationDeliveryWorker(
    IServiceProvider services,
    IConfiguration configuration,
    TimeProvider time,
    ILogger<NotificationDeliveryWorker> logger) : BackgroundService
{
    private static readonly TimeSpan Interval = TimeSpan.FromSeconds(30);

    private static readonly Action<ILogger, Exception?> QueueFailed = LoggerMessage.Define(
        LogLevel.Warning,
        new EventId(1, "ActivityNotificationQueueFailed"),
        "The activity notification queue could not be processed.");

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        // Tests that drive the dispatcher themselves turn the timer off.
        if (!configuration.GetValue("Notifications:WorkerEnabled", true))
        {
            return;
        }

        using var timer = new PeriodicTimer(Interval);
        while (!stoppingToken.IsCancellationRequested)
        {
            try
            {
                await using var scope = services.CreateAsyncScope();
                var dispatcher = scope.ServiceProvider.GetRequiredService<NotificationDeliveryDispatcher>();
                await dispatcher.DispatchAsync(time.GetUtcNow().UtcDateTime, stoppingToken);
            }
            catch (Exception exception) when (exception is not OperationCanceledException)
            {
                QueueFailed(logger, exception);
            }

            try
            {
                await timer.WaitForNextTickAsync(stoppingToken);
            }
            catch (OperationCanceledException)
            {
                return;
            }
        }
    }
}
