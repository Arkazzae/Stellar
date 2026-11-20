using Microsoft.EntityFrameworkCore;

namespace Stellarr.Api.Data.Schema;

/// <summary>
/// Gives an existing audit log the columns of a structured entry. Rows from
/// before keep only their sentence, and every new column is nullable.
/// </summary>
internal static class ServerAuditSchema
{
    private static readonly (string Name, string Type)[] Columns =
    [
        ("Action", "TEXT"),
        ("TargetType", "TEXT"),
        ("TargetId", "TEXT"),
        ("TargetName", "TEXT"),
        ("ChangesJson", "TEXT"),
    ];

    internal static async Task CreateAsync(AppDbContext db, CancellationToken cancellationToken)
    {
        await db.Database.OpenConnectionAsync(cancellationToken);
        try
        {
            var present = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            await using (var command = db.Database.GetDbConnection().CreateCommand())
            {
                command.CommandText = "PRAGMA table_info(\"ServerAuditEntries\");";
                await using var reader = await command.ExecuteReaderAsync(cancellationToken);
                while (await reader.ReadAsync(cancellationToken))
                {
                    present.Add(reader.GetString(1));
                }
            }

            foreach (var (name, type) in Columns)
            {
                if (!present.Contains(name))
                {
#pragma warning disable EF1002 // The names come from the list above, never from input.
                    await db.Database.ExecuteSqlRawAsync(
                        $"ALTER TABLE \"ServerAuditEntries\" ADD COLUMN \"{name}\" {type} NULL;", cancellationToken);
#pragma warning restore EF1002
                }
            }
        }
        finally
        {
            await db.Database.CloseConnectionAsync();
        }
    }
}
