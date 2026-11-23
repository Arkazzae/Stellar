using System.Net;
using System.Net.Http.Json;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// A server from before the ordinary permissions gets, once, what a new
/// server starts with, so nobody there stops being able to post or join
/// voice. Once means once: what an administrator later takes away stays away
/// across restarts, and a database that already had the step is left alone.
/// </summary>
public sealed class OrdinaryPermissionDefaultsTests
{
    private const ServerPrivileges OldEverything = (ServerPrivileges)255;
    private static readonly string[] ConnectOnly = ["connect"];

    [Fact]
    public async Task ADatabaseFromBeforeGetsTheDefaultsOnItsFirstStart()
    {
        await using var connection = await OpenAsync();
        await using var db = Context(connection);
        await db.Database.EnsureCreatedAsync();
        var server = await SeedAsync(db);

        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);

        Assert.Equal(new Dictionary<string, ServerPrivileges>
        {
            ["Owner"] = ServerPrivileges.All,
            ["Admins"] = ServerPrivileges.ManageServer | ServerPrivileges.ManageRoles | ServerPrivileges.CreateInvite,
            ["Mods"] = ServerPrivileges.ManageMessages,
            // A second system role above the base role is not the base role.
            ["Veteran"] = ServerPrivileges.None,
            ["Member"] = ServerRole.BaseRoleDefaults,
        }, await PermissionsAsync(db, server));
    }

    [Fact]
    public async Task WhatAnAdministratorTakesAwayAfterwardsStaysAwayAcrossRestarts()
    {
        await using var connection = await OpenAsync();
        await using var db = Context(connection);
        await db.Database.EnsureCreatedAsync();
        var server = await SeedAsync(db);
        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);

        await db.ServerRoles.Where(x => x.ServerId == server && x.Name == "Member")
            .ExecuteUpdateAsync(s => s.SetProperty(x => x.Permissions, ServerPrivileges.Connect));
        await db.ServerRoles.Where(x => x.ServerId == server && x.Name == "Admins")
            .ExecuteUpdateAsync(s => s.SetProperty(x => x.Permissions, ServerPrivileges.ManageServer));
        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);
        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);

        var permissions = await PermissionsAsync(db, server);
        Assert.Equal(ServerPrivileges.Connect, permissions["Member"]);
        Assert.Equal(ServerPrivileges.ManageServer, permissions["Admins"]);
        Assert.Equal(["ordinary-permissions"], await StepsAsync(connection));
    }

    [Fact]
    public async Task ADatabaseThatHadTheStepIsLeftAsItIs()
    {
        await using var connection = await OpenAsync();
        await using var db = Context(connection);
        await db.Database.EnsureCreatedAsync();
        // A new database runs the step on no data at all, and it is done.
        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);
        var server = await SeedAsync(db);

        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);

        Assert.Equal(new Dictionary<string, ServerPrivileges>
        {
            ["Owner"] = OldEverything,
            ["Admins"] = ServerPrivileges.ManageServer | ServerPrivileges.ManageRoles,
            ["Mods"] = ServerPrivileges.ManageMessages,
            ["Veteran"] = ServerPrivileges.None,
            ["Member"] = ServerPrivileges.None,
        }, await PermissionsAsync(db, server));
    }

    [Fact]
    public async Task AnInstanceRestartedOnItsOwnDatabaseKeepsWhatTheOwnerTookAway()
    {
        var databasePath = Path.Combine(Path.GetTempPath(), $"stellarr-defaults-{Guid.NewGuid():N}.db");
        var dataRoot = Path.Combine(Path.GetTempPath(), $"stellarr-defaults-{Guid.NewGuid():N}");
        StellarrWebApplicationFactory Start() => new(
            loadTestData: true, databasePath: databasePath, dataRoot: dataRoot, deleteStorageOnDispose: false);
        var memberRoleId = Guid.Parse("60000000-0000-0000-0000-000000000003");
        var message = new { content = "cześć" };
        var channel = $"/api/channels/{TestData.GeneralChannelId}/messages";
        try
        {
            using (var before = Start())
            {
                // Back to how the release before left it: no step run, and the
                // masks of that time.
                await using var scope = before.OpenScope(out var db);
                await db.Database.ExecuteSqlRawAsync("""DROP TABLE "OneTimeSteps";""");
                await db.ServerRoles.Where(x => x.Id == memberRoleId)
                    .ExecuteUpdateAsync(s => s.SetProperty(x => x.Permissions, ServerPrivileges.None));
                await db.ServerRoles.Where(x => x.IsOwner)
                    .ExecuteUpdateAsync(s => s.SetProperty(x => x.Permissions, OldEverything));
            }

            using (var upgraded = Start())
            {
                using var jo = await SignedInAsync(upgraded, "jo", "Corona!2026");
                using var owner = await SignedInAsync(upgraded, "maja", "Aurora!2026");
                Assert.Equal(HttpStatusCode.Created, (await jo.PostAsJsonAsync(channel, message)).StatusCode);

                Assert.Equal(HttpStatusCode.OK, (await owner.PatchAsJsonAsync(
                    $"/api/admin/roles/{memberRoleId}", new { permissions = ConnectOnly })).StatusCode);
                Assert.Equal(HttpStatusCode.Forbidden, (await jo.PostAsJsonAsync(channel, message)).StatusCode);
            }

            using var restarted = Start();
            using var again = await SignedInAsync(restarted, "jo", "Corona!2026");
            Assert.Equal(HttpStatusCode.Forbidden, (await again.PostAsJsonAsync(channel, message)).StatusCode);
        }
        finally
        {
            using (var connection = new SqliteConnection(DatabaseConnection.From($"Data Source={databasePath}")))
            {
                SqliteConnection.ClearPool(connection);
            }

            foreach (var path in new[] { databasePath, databasePath + "-wal", databasePath + "-shm" })
            {
                File.Delete(path);
            }

            if (Directory.Exists(dataRoot)) Directory.Delete(dataRoot, recursive: true);
        }
    }

    private static async Task<HttpClient> SignedInAsync(StellarrWebApplicationFactory factory, string username, string password)
    {
        var client = factory.CreateClient(new WebApplicationFactoryClientOptions { AllowAutoRedirect = false, HandleCookies = true });
        (await client.PostAsJsonAsync("/api/auth/login", new { username, password })).EnsureSuccessStatusCode();
        return client;
    }

    private static async Task<SqliteConnection> OpenAsync()
    {
        var connection = new SqliteConnection("Data Source=:memory:;Foreign Keys=True");
        await connection.OpenAsync();
        return connection;
    }

    private static AppDbContext Context(SqliteConnection connection) =>
        new(new DbContextOptionsBuilder<AppDbContext>().UseSqlite(connection).Options);

    /// <summary>A server with the roles of the release before, stored as it stored them.</summary>
    private static async Task<Guid> SeedAsync(AppDbContext db)
    {
        var server = new StellarrServer { Id = Guid.NewGuid(), Name = "Before", CreatedAt = DateTime.UtcNow };
        db.Servers.Add(server);
        db.ServerRoles.AddRange(
            Role(server.Id, "Owner", 0, OldEverything, system: true, isOwner: true),
            Role(server.Id, "Admins", 1, ServerPrivileges.ManageServer | ServerPrivileges.ManageRoles),
            Role(server.Id, "Mods", 2, ServerPrivileges.ManageMessages),
            Role(server.Id, "Veteran", 3, ServerPrivileges.None, system: true),
            Role(server.Id, "Member", 4, ServerPrivileges.None, system: true));
        await db.SaveChangesAsync();
        db.ChangeTracker.Clear();
        return server.Id;
    }

    private static ServerRole Role(
        Guid serverId, string name, int position, ServerPrivileges permissions, bool system = false, bool isOwner = false) => new()
    {
        Id = Guid.NewGuid(),
        ServerId = serverId,
        Name = name,
        Color = "#777777",
        Permissions = permissions,
        Position = position,
        IsSystem = system,
        IsOwner = isOwner,
    };

    private static async Task<Dictionary<string, ServerPrivileges>> PermissionsAsync(AppDbContext db, Guid serverId) =>
        await db.ServerRoles.AsNoTracking().Where(x => x.ServerId == serverId)
            .ToDictionaryAsync(x => x.Name, x => x.Permissions);

    private static async Task<List<string>> StepsAsync(SqliteConnection connection)
    {
        await using var command = connection.CreateCommand();
        command.CommandText = """SELECT "Name" FROM "OneTimeSteps" ORDER BY "Name";""";
        await using var reader = await command.ExecuteReaderAsync();
        var names = new List<string>();
        while (await reader.ReadAsync())
        {
            names.Add(reader.GetString(0));
        }

        return names;
    }
}
