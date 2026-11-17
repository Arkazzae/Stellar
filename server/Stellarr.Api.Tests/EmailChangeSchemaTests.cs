using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Data.Schema;

namespace Stellarr.Api.Tests;

public sealed class EmailChangeSchemaTests
{
    [Fact]
    public async Task ExistingPendingLinksGainNoAuthorityAndUpgradeIsRepeatable()
    {
        await using var connection = new SqliteConnection("Data Source=:memory:");
        await connection.OpenAsync();
        await using var db = new AppDbContext(new DbContextOptionsBuilder<AppDbContext>().UseSqlite(connection).Options);
        await db.Database.ExecuteSqlRawAsync("""
            CREATE TABLE "EmailChangeRequests" ("Id" TEXT PRIMARY KEY, "TokenHash" TEXT NOT NULL);
            INSERT INTO "EmailChangeRequests" VALUES ('old-link', 'old-digest');
            """);

        await EmailChangeSchema.CreateAsync(db, CancellationToken.None);
        await EmailChangeSchema.CreateAsync(db, CancellationToken.None);

        await using var command = connection.CreateCommand();
        command.CommandText = "SELECT TokenHash, SessionId, SecurityVersion FROM EmailChangeRequests;";
        await using var reader = await command.ExecuteReaderAsync();
        Assert.True(await reader.ReadAsync());
        Assert.Equal("old-digest", reader.GetString(0));
        Assert.True(reader.IsDBNull(1));
        Assert.True(reader.IsDBNull(2));
        Assert.False(await reader.ReadAsync());
    }
}
