using System.Text.Json;

namespace Stellarr.Api.Realtime;

/// <summary>
/// Restores the presence a development seed wrote for accounts it cannot keep
/// connected. Presence normally lives only in this process and follows live hub
/// connections; a seed that inserts 28 accounts has no way to open one for each,
/// so it leaves a small document on the data volume instead.
/// </summary>
/// <remarks>
/// Development only: a production instance must never invent a status for
/// someone who is not connected. Connecting as a seeded account promotes it to
/// a normal live session and drops the seeded fallback.
/// </remarks>
public static partial class DevelopmentPresenceSeed
{
    public const string FileName = "seed-presence.json";

    private static readonly JsonSerializerOptions JsonOptions = new(JsonSerializerDefaults.Web);

    [LoggerMessage(
        EventId = 4101,
        Level = LogLevel.Information,
        Message = "Restored {Count} seeded presence entries from {Path}.")]
    private static partial void SeededPresenceRestored(ILogger logger, int count, string path);

    [LoggerMessage(
        EventId = 4102,
        Level = LogLevel.Warning,
        Message = "Could not read the seeded presence from {Path}.")]
    private static partial void SeededPresenceUnreadable(ILogger logger, Exception exception, string path);

    public static void Apply(IServiceProvider services)
    {
        if (!services.GetRequiredService<IHostEnvironment>().IsDevelopment())
        {
            return;
        }

        var dataRoot = services.GetRequiredService<IConfiguration>()["DataRoot"] ?? "data";
        var path = Path.Combine(dataRoot, FileName);
        if (!File.Exists(path))
        {
            return;
        }

        var logger = services.GetRequiredService<ILoggerFactory>()
            .CreateLogger(typeof(DevelopmentPresenceSeed));
        try
        {
            var document = JsonSerializer.Deserialize<Document>(File.ReadAllText(path), JsonOptions);
            var presences = new List<SeedPresence>();
            foreach (var entry in document?.Users ?? [])
            {
                if (Guid.TryParse(entry.Id, out var userId) &&
                    PresenceTracker.IsSupportedPresence(entry.Presence))
                {
                    presences.Add(new SeedPresence(userId, entry.Presence));
                }
            }

            services.GetRequiredService<PresenceTracker>().SeedPresence(presences);
            SeededPresenceRestored(logger, presences.Count, path);
        }
        catch (Exception exception) when (exception is IOException or UnauthorizedAccessException or JsonException)
        {
            SeededPresenceUnreadable(logger, exception, path);
        }
    }

    private sealed record Document(SeededUser[] Users);

    private sealed record SeededUser(string Id, string Presence);
}
