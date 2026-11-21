using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// A database created while channels still stored an audio profile loses the
/// column on the next start and keeps its channels, and a second start changes
/// nothing.
/// </summary>
public sealed class ChannelAudioProfileSchemaTests
{
    [Fact]
    public async Task AnOldDatabaseLosesTheColumnAndKeepsItsChannels()
    {
        await using var connection = new SqliteConnection("Data Source=:memory:");
        await connection.OpenAsync();
        await using var db = new AppDbContext(
            new DbContextOptionsBuilder<AppDbContext>().UseSqlite(connection).Options);
        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);
        // The column as the model used to create it, on the full schema: DROP
        // COLUMN checks the triggers and indexes on the table again.
        await db.Database.ExecuteSqlRawAsync(
            """ALTER TABLE "Channels" ADD COLUMN "AudioProfile" TEXT NOT NULL DEFAULT 'premium';""");
        var server = new StellarrServer { Id = Guid.NewGuid(), Name = "Orbit", CreatedAt = DateTime.UtcNow };
        var channel = new Channel
        {
            Id = Guid.NewGuid(),
            ServerId = server.Id,
            Name = "salon",
            Kind = ChannelKind.Voice,
            UserLimit = 8,
        };
        db.Servers.Add(server);
        db.Channels.Add(channel);
        await db.SaveChangesAsync();
        db.ChangeTracker.Clear();

        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);
        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);

        await using (var command = connection.CreateCommand())
        {
            command.CommandText = """PRAGMA table_info("Channels");""";
            await using var reader = await command.ExecuteReaderAsync();
            var columns = new List<string>();
            while (await reader.ReadAsync())
            {
                columns.Add(reader.GetString(1));
            }

            Assert.DoesNotContain("AudioProfile", columns);
        }

        var kept = await db.Channels.SingleAsync(x => x.Id == channel.Id);
        Assert.Equal(("salon", ChannelKind.Voice, 8), (kept.Name, kept.Kind, kept.UserLimit));
    }
}
