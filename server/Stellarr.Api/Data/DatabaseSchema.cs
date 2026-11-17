using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data.Schema;

namespace Stellarr.Api.Data;

/// <summary>
/// Creates the SQLite schema of the instance: every table, column and index
/// comes from the model, and the steps add what the model cannot express.
/// </summary>
public static class DatabaseSchema
{
    /// <summary>
    /// The triggers and the virtual table on top of the model. Each step is
    /// idempotent and runs on every start, so a database whose first start was
    /// cut short between the model and these still gets them.
    /// </summary>
    private static readonly Func<AppDbContext, CancellationToken, Task>[] Steps =
    [
        EmailChangeSchema.CreateAsync,
        MessageJournalSchema.CreateAsync,
        MessageIndexSchema.CreateAsync,
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
    }
}
