namespace Stellarr.Api.Logging;

public static partial class StartupLog
{
    [LoggerMessage(
        EventId = 1001,
        Level = LogLevel.Warning,
        Message = "The object store is not ready yet ({Attempt}/{MaxAttempts}): {ErrorMessage}")]
    public static partial void StorageNotReady(ILogger logger, int attempt, int maxAttempts, string errorMessage);

    // The setup code is a warning so that it still shows when the API's log
    // is turned down to warnings: without it nobody can create the owner
    // account. Each is one line, which `docker logs` shows as it is.
    [LoggerMessage(
        EventId = 1002,
        Level = LogLevel.Warning,
        Message = "Setup code: {Code}. Open {SetupUrl} to create the owner account.")]
    public static partial void SetupCodeIssued(ILogger logger, string code, string setupUrl);

    /// <summary>The same line for an instance that does not know its public address.</summary>
    [LoggerMessage(
        EventId = 1003,
        Level = LogLevel.Warning,
        Message = "Setup code: {Code}. Open /setup at the address of this instance to create the owner account.")]
    public static partial void SetupCodeIssuedWithoutAddress(ILogger logger, string code);
}
