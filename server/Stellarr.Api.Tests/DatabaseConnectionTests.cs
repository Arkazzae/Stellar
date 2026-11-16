using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// What the API's connections to the database open with, read from the
/// connections themselves: the pool hands out many, and each one must have it.
/// </summary>
public sealed class DatabaseConnectionTests
{
    /// <summary>SQLITE_CONSTRAINT_FOREIGNKEY.</summary>
    private const int ForeignKeyViolation = 787;

    [Fact]
    public async Task EveryConnectionThePoolHandsOutEnforcesForeignKeys()
    {
        using var factory = new StellarrWebApplicationFactory(loadTestData: false);
        string settings;
        await using (var scope = factory.OpenScope(out var app))
        {
            settings = app.Database.GetConnectionString()!;
        }

        Assert.Equal(factory.DatabasePath, new SqliteConnectionStringBuilder(settings).DataSource);

        // The API's own settings on a file nothing else opens, so that no
        // background service takes a connection from this pool between steps.
        var path = Path.Combine(Path.GetTempPath(), $"stellarr-pool-{Guid.NewGuid():N}.db");
        var connectionString = new SqliteConnectionStringBuilder(settings) { DataSource = path }.ToString();
        try
        {
            await using (var setup = Context(connectionString))
            {
                await setup.Database.EnsureCreatedAsync();
            }

            await using var first = Context(connectionString);
            await first.Database.OpenConnectionAsync();
            await AssertForeignKeysEnforcedAsync(first);

            SQLitePCL.sqlite3? reused;
            await using (var second = Context(connectionString))
            {
                await second.Database.OpenConnectionAsync();
                reused = Handle(second);
                Assert.NotSame(Handle(first), reused);
                await AssertForeignKeysEnforcedAsync(second);

                // Back to the pool switched off, as a connection would be if
                // nothing but the native library's default stood behind it.
                await second.Database.ExecuteSqlRawAsync("PRAGMA foreign_keys=OFF;");
            }

            await using var third = Context(connectionString);
            await third.Database.OpenConnectionAsync();
            Assert.Same(reused, Handle(third));
            await AssertForeignKeysEnforcedAsync(third);
        }
        finally
        {
            await using (var connection = new SqliteConnection(connectionString))
            {
                SqliteConnection.ClearPool(connection);
            }

            foreach (var file in new[] { path, path + "-wal", path + "-shm" })
            {
                File.Delete(file);
            }
        }
    }

    [Fact]
    public async Task TheDatabaseIsInWriteAheadLoggingOnEveryConnection()
    {
        using var factory = new StellarrWebApplicationFactory(loadTestData: false);
        await using var firstScope = factory.OpenScope(out var first);
        await using var secondScope = factory.OpenScope(out var second);
        await first.Database.OpenConnectionAsync();
        await second.Database.OpenConnectionAsync();
        Assert.NotSame(Handle(first), Handle(second));

        Assert.Equal("wal", await ScalarAsync(first, "PRAGMA journal_mode;"));
        Assert.Equal("wal", await ScalarAsync(second, "PRAGMA journal_mode;"));

        // Stored in the file: a connection from outside the API finds it too.
        await using var outside = new SqliteConnection($"Data Source={factory.DatabasePath};Pooling=False");
        await outside.OpenAsync();
        await using var command = outside.CreateCommand();
        command.CommandText = "PRAGMA journal_mode;";
        Assert.Equal("wal", await command.ExecuteScalarAsync());
    }

    [Fact]
    public async Task WritersInSeparateContextsWaitForEachOtherInsteadOfFailing()
    {
        const int Writers = 4;
        using var factory = new StellarrWebApplicationFactory(loadTestData: false);
        var names = Enumerable.Range(0, Writers + 1).Select(_ => DataModelSupport.UniqueName()).ToArray();
        var waiting = Enumerable.Range(0, Writers)
            .Select(_ => new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously))
            .ToArray();

        Task[] writers;
        await using (var holderScope = factory.OpenScope(out var holder))
        await using (var transaction = await holder.Database.BeginTransactionAsync())
        {
            // The transaction began with BEGIN IMMEDIATE, so this context
            // holds the write lock until it commits.
            holder.Users.Add(NewUser(names[0]));
            await holder.SaveChangesAsync();

            writers = Enumerable.Range(1, Writers).Select(index => Task.Run(async () =>
            {
                await using var scope = factory.OpenScope(out var db);
                db.Users.Add(NewUser(names[index]));
                waiting[index - 1].SetResult();
                await db.SaveChangesAsync();
            })).ToArray();

            // Each writer is about to save; give them time to meet the lock.
            await Task.WhenAll(waiting.Select(x => x.Task)).WaitAsync(TimeSpan.FromSeconds(30));
            await Task.Delay(TimeSpan.FromMilliseconds(250));
            await transaction.CommitAsync();
        }

        // A busy or locked database that reached a writer would surface here.
        await Task.WhenAll(writers);

        await using var checkScope = factory.OpenScope(out var check);
        Assert.Equal(Writers + 1, await check.Users.CountAsync(x => names.Contains(x.Username)));
    }

    private static AppDbContext Context(string connectionString) =>
        new(new DbContextOptionsBuilder<AppDbContext>().UseSqlite(connectionString).Options);

    /// <summary>The native connection under a context's open connection.</summary>
    private static SQLitePCL.sqlite3? Handle(AppDbContext db) => ((SqliteConnection)db.Database.GetDbConnection()).Handle;

    private static async Task<object?> ScalarAsync(AppDbContext db, string sql)
    {
        await using var command = db.Database.GetDbConnection().CreateCommand();
        command.CommandText = sql;
        return await command.ExecuteScalarAsync();
    }

    /// <summary>Checks the setting, then has SQLite refuse a member of a server that does not exist.</summary>
    private static async Task AssertForeignKeysEnforcedAsync(AppDbContext db)
    {
        Assert.Equal(1L, await ScalarAsync(db, "PRAGMA foreign_keys;"));
        db.ServerMembers.Add(new ServerMember
        {
            ServerId = Guid.NewGuid(),
            UserId = Guid.NewGuid(),
            RoleId = Guid.NewGuid(),
            JoinedAt = DateTime.UtcNow,
        });

        var error = await Assert.ThrowsAsync<DbUpdateException>(() => db.SaveChangesAsync());
        Assert.Equal(ForeignKeyViolation, Assert.IsType<SqliteException>(error.InnerException).SqliteExtendedErrorCode);
        db.ChangeTracker.Clear();
    }

    private static User NewUser(string name) => new()
    {
        Id = Guid.NewGuid(),
        Username = name,
        NormalizedUsername = name.ToUpperInvariant(),
        DisplayName = name,
        PasswordHash = "hash",
        AvatarColor = "#777777",
        CreatedAt = DateTime.UtcNow,
    };
}
