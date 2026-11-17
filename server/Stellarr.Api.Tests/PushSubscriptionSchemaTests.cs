using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Data.Schema;

namespace Stellarr.Api.Tests;

public sealed class PushSubscriptionSchemaTests
{
    [Fact]
    public async Task NewDatabaseGetsTheTableFromTheModel()
    {
        await using var connection = new SqliteConnection("Data Source=:memory:");
        await connection.OpenAsync();
        await using var db = new AppDbContext(
            new DbContextOptionsBuilder<AppDbContext>().UseSqlite(connection).Options);

        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);

        Assert.Equal(0, await db.PushSubscriptions.CountAsync());
    }

    [Fact]
    public async Task ExistingDatabaseGainsTheTableAndIndexesIdempotently()
    {
        await using var connection = new SqliteConnection("Data Source=:memory:");
        await connection.OpenAsync();
        await using var db = new AppDbContext(
            new DbContextOptionsBuilder<AppDbContext>().UseSqlite(connection).Options);
        await db.Database.ExecuteSqlRawAsync(
            """
            CREATE TABLE "Users" ("Id" TEXT NOT NULL PRIMARY KEY);
            CREATE TABLE "Existing" ("Id" TEXT NOT NULL PRIMARY KEY);
            INSERT INTO "Users" ("Id") VALUES ('30000000-0000-0000-0000-000000000001');
            """);

        await PushSubscriptionSchema.CreateAsync(db, CancellationToken.None);
        await PushSubscriptionSchema.CreateAsync(db, CancellationToken.None);

        await using (var command = connection.CreateCommand())
        {
            command.CommandText = """PRAGMA table_info("PushSubscriptions");""";
            await using var reader = await command.ExecuteReaderAsync();
            var columns = new List<(string Name, bool Required)>();
            while (await reader.ReadAsync())
            {
                columns.Add((reader.GetString(1), reader.GetInt32(3) == 1));
            }

            Assert.Equal(
                ["Id", "UserId", "SessionId", "Endpoint", "P256dh", "Auth", "CreatedAt"],
                columns.Select(x => x.Name));
            Assert.All(columns.Skip(2), x => Assert.True(x.Required));
        }

        await using (var command = connection.CreateCommand())
        {
            command.CommandText = """PRAGMA index_list("PushSubscriptions");""";
            await using var reader = await command.ExecuteReaderAsync();
            var indexes = new List<string>();
            while (await reader.ReadAsync())
            {
                indexes.Add(reader.GetString(1));
            }

            Assert.Contains("IX_PushSubscriptions_Endpoint", indexes);
            Assert.Contains("IX_PushSubscriptions_UserId_SessionId", indexes);
        }

        await using (var command = connection.CreateCommand())
        {
            command.CommandText = """
                INSERT INTO "PushSubscriptions"
                    ("Id", "UserId", "SessionId", "Endpoint", "P256dh", "Auth", "CreatedAt")
                VALUES
                    ('40000000-0000-0000-0000-000000000001',
                     '30000000-0000-0000-0000-000000000001',
                     '50000000-0000-0000-0000-000000000001',
                     'https://push.example.test/send/upgrade',
                     'key',
                     'secret',
                     '2026-01-01T00:00:00Z');
                """;
            Assert.Equal(1, await command.ExecuteNonQueryAsync());
        }
    }

    [Fact]
    public async Task EndpointIndexRejectsASecondOwnerOfTheSameEndpoint()
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
        await PushSubscriptionSchema.CreateAsync(db, CancellationToken.None);

        await using var command = connection.CreateCommand();
        command.CommandText = """
            INSERT INTO "PushSubscriptions"
                ("Id", "UserId", "SessionId", "Endpoint", "P256dh", "Auth", "CreatedAt")
            VALUES
                ('40000000-0000-0000-0000-000000000001',
                 '30000000-0000-0000-0000-000000000001',
                 '50000000-0000-0000-0000-000000000001',
                 'https://push.example.test/send/one',
                 'key', 'secret', '2026-01-01T00:00:00Z'),
                ('40000000-0000-0000-0000-000000000002',
                 '30000000-0000-0000-0000-000000000001',
                 '50000000-0000-0000-0000-000000000002',
                 'https://push.example.test/send/one',
                 'key', 'secret', '2026-01-01T00:00:00Z');
            """;

        await Assert.ThrowsAsync<SqliteException>(() => command.ExecuteNonQueryAsync());
    }
}
