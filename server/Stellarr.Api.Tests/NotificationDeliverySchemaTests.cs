using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Data.Schema;

namespace Stellarr.Api.Tests;

/// <summary>
/// An installation that predates the activity outbox still gets its table and
/// every index from the schema step, and a second run changes nothing.
/// </summary>
public sealed class NotificationDeliverySchemaTests
{
    [Fact]
    public async Task ExistingDatabaseGainsTheOutboxAndItsUniqueIndexIdempotently()
    {
        await using var connection = new SqliteConnection("Data Source=:memory:");
        await connection.OpenAsync();
        await using var db = new AppDbContext(
            new DbContextOptionsBuilder<AppDbContext>().UseSqlite(connection).Options);
        await db.Database.ExecuteSqlRawAsync(
            """
            CREATE TABLE "Users" ("Id" TEXT NOT NULL PRIMARY KEY);
            INSERT INTO "Users" ("Id") VALUES ('30000000-0000-0000-0000-000000000001');
            """);

        await NotificationDeliverySchema.CreateAsync(db, CancellationToken.None);
        await NotificationDeliverySchema.CreateAsync(db, CancellationToken.None);

        await using (var command = connection.CreateCommand())
        {
            command.CommandText = """PRAGMA table_info("NotificationDeliveries");""";
            await using var reader = await command.ExecuteReaderAsync();
            var columns = new List<(string Name, bool Required)>();
            while (await reader.ReadAsync())
            {
                columns.Add((reader.GetString(1), reader.GetInt32(3) == 1));
            }

            Assert.Equal(
                [
                    "Id", "SourceMessageId", "RecipientId", "EventKind", "Channel",
                    "DestinationKey", "PushSubscriptionId", "CreatedAt", "ExpiresAt",
                    "Attempts", "NextAttemptAt", "LastAttemptAt", "SentAt", "Status",
                    "LeaseId", "LeaseUntil", "LastError",
                ],
                columns.Select(x => x.Name));
            Assert.All(
                columns.Where(x => x.Name is not ("PushSubscriptionId" or "LastAttemptAt" or "SentAt" or "LeaseId" or "LeaseUntil")),
                x => Assert.True(x.Required, $"{x.Name} is required"));
        }

        await using (var command = connection.CreateCommand())
        {
            command.CommandText = """PRAGMA index_list("NotificationDeliveries");""";
            await using var reader = await command.ExecuteReaderAsync();
            var indexes = new List<string>();
            while (await reader.ReadAsync())
            {
                indexes.Add(reader.GetString(1));
            }

            Assert.Contains("IX_NotificationDeliveries_Unique", indexes);
            Assert.Contains("IX_NotificationDeliveries_Status_NextAttemptAt_LeaseUntil", indexes);
            Assert.Contains("IX_NotificationDeliveries_ExpiresAt", indexes);
            Assert.Contains("IX_NotificationDeliveries_RecipientId_Channel", indexes);
            Assert.Contains("IX_NotificationDeliveries_PushSubscriptionId", indexes);
        }

        await using (var command = connection.CreateCommand())
        {
            command.CommandText = """
                INSERT INTO "NotificationDeliveries"
                    ("Id", "SourceMessageId", "RecipientId", "EventKind", "Channel",
                     "DestinationKey", "PushSubscriptionId", "CreatedAt", "ExpiresAt",
                     "Attempts", "NextAttemptAt", "LastAttemptAt", "SentAt", "Status",
                     "LeaseId", "LeaseUntil", "LastError")
                VALUES
                    ('40000000-0000-0000-0000-000000000001',
                     '50000000-0000-0000-0000-000000000001',
                     '30000000-0000-0000-0000-000000000001',
                     'mention', 'email', 'ADA@EXAMPLE.TEST', NULL,
                     '2026-01-01T00:00:00Z', '2026-01-08T00:00:00Z',
                     0, '2026-01-01T00:00:00Z', NULL, NULL, 'pending',
                     NULL, NULL, '');
                """;
            Assert.Equal(1, await command.ExecuteNonQueryAsync());
        }

        await using (var command = connection.CreateCommand())
        {
            // One row per message, recipient, event, channel and destination.
            command.CommandText = """
                INSERT INTO "NotificationDeliveries"
                    ("Id", "SourceMessageId", "RecipientId", "EventKind", "Channel",
                     "DestinationKey", "PushSubscriptionId", "CreatedAt", "ExpiresAt",
                     "Attempts", "NextAttemptAt", "LastAttemptAt", "SentAt", "Status",
                     "LeaseId", "LeaseUntil", "LastError")
                VALUES
                    ('40000000-0000-0000-0000-000000000002',
                     '50000000-0000-0000-0000-000000000001',
                     '30000000-0000-0000-0000-000000000001',
                     'mention', 'email', 'ADA@EXAMPLE.TEST', NULL,
                     '2026-01-01T00:00:00Z', '2026-01-08T00:00:00Z',
                     0, '2026-01-01T00:00:00Z', NULL, NULL, 'pending',
                     NULL, NULL, '');
                """;
            await Assert.ThrowsAsync<SqliteException>(() => command.ExecuteNonQueryAsync());
        }
    }
}
