using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Data.Schema;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// A database made before the Discord import: no column that marks a
/// placeholder and none of the import's bookkeeping. The step must give it
/// both exactly as a new database has them, and keep the accounts it has.
/// </summary>
public sealed class DiscordImportSchemaTests
{
    private static readonly string[] NewTables =
    [
        "DiscordImports",
        "DiscordImportAccounts",
        "DiscordImportSections",
        "DiscordImportChannels",
        "DiscordImportMessages",
    ];

    [Fact]
    public async Task UpgradedDatabaseGetsWhatANewOneHasAndARepeatChangesNothing()
    {
        await using var fresh = await OpenAsync();
        await using var freshDb = Context(fresh);
        await freshDb.Database.EnsureCreatedAsync();

        await using var upgraded = await OpenAsync();
        await using var upgradedDb = Context(upgraded);
        await CreateBeforeTheChangeAsync(upgradedDb);
        await DiscordImportSchema.CreateAsync(upgradedDb, CancellationToken.None);
        await DiscordImportSchema.CreateAsync(upgradedDb, CancellationToken.None);

        foreach (var table in NewTables.Append("Users"))
        {
            Assert.Equal(await ShapeAsync(fresh, table), await ShapeAsync(upgraded, table));
        }
    }

    [Fact]
    public async Task AnOlderDatabaseStartsAndItsAccountsArePeople()
    {
        await using var connection = await OpenAsync();
        await using var db = Context(connection);
        await CreateBeforeTheChangeAsync(db);
        var id = Guid.NewGuid();
        await db.Database.ExecuteSqlInterpolatedAsync(
            $"""
            INSERT INTO "Users" ("Id", "Username", "NormalizedUsername", "DisplayName", "IsDisabled",
                "SessionVersion", "SecurityRecoveryRequired", "AvatarColor", "CreatedAt")
            VALUES ({id}, 'ada', 'ADA', 'Ada', 0, 0, 0, '#777777', {DateTime.UtcNow});
            """);

        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);

        var user = await db.Users.AsNoTracking().SingleAsync(x => x.Id == id);
        Assert.Null(user.Origin);
        Assert.Equal([id], await db.Users.People().Select(x => x.Id).ToListAsync());
    }

    private static async Task<SqliteConnection> OpenAsync()
    {
        var connection = new SqliteConnection("Data Source=:memory:;Foreign Keys=True");
        await connection.OpenAsync();
        return connection;
    }

    private static AppDbContext Context(SqliteConnection connection) =>
        new(new DbContextOptionsBuilder<AppDbContext>().UseSqlite(connection).Options);

    /// <summary>The model's schema without what this change added.</summary>
    private static async Task CreateBeforeTheChangeAsync(AppDbContext db)
    {
        await db.Database.EnsureCreatedAsync();
        // Children first: the tables point at each other.
        foreach (var table in NewTables.Reverse())
        {
#pragma warning disable EF1002 // The names come from the list above.
            await db.Database.ExecuteSqlRawAsync($"DROP TABLE \"{table}\";");
#pragma warning restore EF1002
        }

        await db.Database.ExecuteSqlRawAsync("ALTER TABLE \"Users\" DROP COLUMN \"Origin\";");
    }

    /// <summary>The columns, foreign keys and indexes of a table, in a form two databases can be compared by.</summary>
    private static async Task<List<string>> ShapeAsync(SqliteConnection connection, string table)
    {
        var shape = new List<string>();
        shape.AddRange((await RowsAsync(connection, $"PRAGMA table_info(\"{table}\");")).Select(x => $"column {x}"));
        // The first two values are the foreign key's ordinal and the column's place in it.
        shape.AddRange((await RowsAsync(connection, $"PRAGMA foreign_key_list(\"{table}\");"))
            .Select(x => $"foreign key {x[(x.IndexOf('|', x.IndexOf('|') + 1) + 1)..]}").Order(StringComparer.Ordinal));
        foreach (var index in (await RowsAsync(connection, $"PRAGMA index_list(\"{table}\");"))
                     .Select(x => x[(x.IndexOf('|') + 1)..]).Order(StringComparer.Ordinal))
        {
            shape.Add($"index {index}");
            var name = index[..index.IndexOf('|')];
            shape.AddRange((await RowsAsync(connection, $"PRAGMA index_info(\"{name}\");")).Select(x => $"  {x}"));
        }

        return shape;
    }

    private static async Task<List<string>> RowsAsync(SqliteConnection connection, string sql)
    {
        await using var command = connection.CreateCommand();
        command.CommandText = sql;
        await using var reader = await command.ExecuteReaderAsync();
        var rows = new List<string>();
        while (await reader.ReadAsync())
        {
            rows.Add(string.Join('|', Enumerable.Range(0, reader.FieldCount)
                .Select(i => reader.IsDBNull(i) ? "null" : Convert.ToString(reader.GetValue(i), System.Globalization.CultureInfo.InvariantCulture))));
        }

        return rows;
    }
}
