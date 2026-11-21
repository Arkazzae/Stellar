using Microsoft.EntityFrameworkCore;

namespace Stellarr.Api.Data.Schema;

/// <summary>
/// Drops the audio profile a channel used to store. Nothing read it, and the
/// model no longer has it, so only a database created before carries the column.
/// </summary>
internal static class ChannelAudioProfileSchema
{
    internal static async Task DropAsync(AppDbContext db, CancellationToken cancellationToken)
    {
        await db.Database.OpenConnectionAsync(cancellationToken);
        try
        {
            var present = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            await using (var command = db.Database.GetDbConnection().CreateCommand())
            {
                command.CommandText = "PRAGMA table_info(\"Channels\");";
                await using var reader = await command.ExecuteReaderAsync(cancellationToken);
                while (await reader.ReadAsync(cancellationToken))
                {
                    present.Add(reader.GetString(1));
                }
            }

            if (present.Contains("AudioProfile"))
            {
                // DROP COLUMN needs SQLite 3.35; the bundled e_sqlite3 is newer.
                await db.Database.ExecuteSqlRawAsync(
                    "ALTER TABLE \"Channels\" DROP COLUMN \"AudioProfile\";", cancellationToken);
            }
        }
        finally
        {
            await db.Database.CloseConnectionAsync();
        }
    }
}
