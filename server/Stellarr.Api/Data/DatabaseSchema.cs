using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data.OneTime;
using Stellarr.Api.Data.Schema;

namespace Stellarr.Api.Data;

/// <summary>
/// Creates the SQLite schema of the instance: every table, column and index
/// comes from the model, and the steps add what the model cannot express and
/// remove what it no longer has.
/// </summary>
public static class DatabaseSchema
{
    /// <summary>
    /// The triggers and the virtual table on top of the model, and the drop of
    /// columns an older database still has. Each step is idempotent and runs on
    /// every start, so a database whose first start was cut short between the
    /// model and these still gets them.
    /// </summary>
    private static readonly Func<AppDbContext, CancellationToken, Task>[] Steps =
    [
        EmailChangeSchema.CreateAsync,
        ChannelPostTimeSchema.CreateAsync,
        MessageJournalSchema.CreateAsync,
        MessageIndexSchema.CreateAsync,
        ServerContentSchema.CreateAsync,
        // The push table is referenced by the activity outbox, so it comes
        // first even though the outbox only stores its id.
        PushSubscriptionSchema.CreateAsync,
        NotificationDeliverySchema.CreateAsync,
        ServerAuditSchema.CreateAsync,
        ChannelAudioProfileSchema.DropAsync,
        MemberPermissionsSchema.CreateAsync,
        RoleRankSchema.RepairAsync,
        DiscordImportSchema.CreateAsync,
        // After every trigger: it deletes through them.
        OrphanContentSchema.RemoveAsync,
    ];

    public static async Task PrepareAsync(AppDbContext db, CancellationToken cancellationToken)
    {
        await db.Database.EnsureCreatedAsync(cancellationToken);
        // The journal mode is stored in the database file, so once holds for
        // every connection. What belongs to a connection is in the connection
        // string, see DatabaseConnection.
        await db.Database.ExecuteSqlRawAsync("PRAGMA journal_mode=WAL;", cancellationToken);

        foreach (var step in Steps)
        {
            await step(db, cancellationToken);
        }

        // Last: a one-time step reads the data as the steps above leave it.
        await OneTimeSteps.RunAsync(db, cancellationToken);
    }
}
