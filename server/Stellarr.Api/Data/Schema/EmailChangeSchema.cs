using Microsoft.EntityFrameworkCore;

namespace Stellarr.Api.Data.Schema;

/// <summary>Upgrades existing databases without authorizing their unbound pending links.</summary>
internal static class EmailChangeSchema
{
    internal static async Task CreateAsync(AppDbContext db, CancellationToken cancellationToken)
    {
        await db.Database.OpenConnectionAsync(cancellationToken);
        try
        {
            var columns = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            await using (var command = db.Database.GetDbConnection().CreateCommand())
            {
                command.CommandText = "PRAGMA table_info(\"EmailChangeRequests\");";
                await using var reader = await command.ExecuteReaderAsync(cancellationToken);
                while (await reader.ReadAsync(cancellationToken))
                {
                    columns.Add(reader.GetString(1));
                }
            }

            if (!columns.Contains("SessionId"))
            {
                await db.Database.ExecuteSqlRawAsync(
                    "ALTER TABLE \"EmailChangeRequests\" ADD COLUMN \"SessionId\" TEXT NULL;", cancellationToken);
            }

            if (!columns.Contains("SecurityVersion"))
            {
                await db.Database.ExecuteSqlRawAsync(
                    "ALTER TABLE \"EmailChangeRequests\" ADD COLUMN \"SecurityVersion\" INTEGER NULL;", cancellationToken);
            }
        }
        finally
        {
            await db.Database.CloseConnectionAsync();
        }
    }
}
