using Microsoft.EntityFrameworkCore;

namespace Stellarr.Api.Data.OneTime;

/// <summary>
/// Changes to the data that happen once in the life of a database, where the
/// schema steps run on every start. A step that has run is named in the
/// <c>OneTimeSteps</c> table, written in the same transaction as the step's
/// own writes, so it never runs again: not after a restart, and not after an
/// administrator undid what it did.
/// </summary>
/// <remarks>
/// The table lives in the database file, so a backup carries it. A backup
/// from before a step gets the step on its first start after a restore, and
/// one from after it does not. A new database runs every step on no data at
/// all and is marked, which is why whatever writes data later, a new server,
/// the test fixture or the development seed, writes what the steps would
/// have. A name is never changed nor reused. The CLI reads none of this: it
/// never runs schema steps, and it neither reads nor writes what they change.
/// </remarks>
internal static class OneTimeSteps
{
    /// <summary>Every step, in the order they run, by the name they are stored under.</summary>
    private static readonly (string Name, Func<AppDbContext, CancellationToken, Task> Apply)[] Steps =
    [
        ("ordinary-permissions", OrdinaryPermissionDefaults.ApplyAsync),
        ("view-channel", ViewChannelDefaults.ApplyAsync),
    ];

    /// <summary>Runs, each in a transaction of its own, the steps this database has not run yet.</summary>
    internal static async Task RunAsync(AppDbContext db, CancellationToken cancellationToken)
    {
        await db.Database.ExecuteSqlRawAsync(
            """
            CREATE TABLE IF NOT EXISTS "OneTimeSteps" (
                "Name" TEXT NOT NULL CONSTRAINT "PK_OneTimeSteps" PRIMARY KEY,
                "AppliedAt" TEXT NOT NULL
            );
            """,
            cancellationToken);

        foreach (var (name, apply) in Steps)
        {
            await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
            // Claiming the name first decides whether the step is due, in the
            // transaction that will hold its writes: both commit or neither.
            var claimed = await db.Database.ExecuteSqlInterpolatedAsync(
                $"""INSERT OR IGNORE INTO "OneTimeSteps" ("Name", "AppliedAt") VALUES ({name}, {DateTime.UtcNow})""",
                cancellationToken);
            if (claimed == 0)
            {
                continue;
            }

            await apply(db, cancellationToken);
            await transaction.CommitAsync(cancellationToken);
        }
    }
}
