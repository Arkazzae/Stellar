using System.Net;
using System.Net.Http.Json;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// A server from before <c>viewChannel</c> gets it once for its base role, and
/// the new everything for its owner role, so nobody there stops seeing a
/// channel. The databases here already ran the step before it, with the masks
/// of that release, which is what an instance being upgraded has.
/// </summary>
public sealed class ViewChannelDefaultsTests
{
    /// <summary>Every permission of the release before, the owner role's mask then.</summary>
    private const ServerPrivileges OldEverything = (ServerPrivileges)((1 << 14) - 1);

    /// <summary>The base role's defaults of the release before.</summary>
    private const ServerPrivileges OldDefaults =
        ServerPrivileges.SendMessages | ServerPrivileges.AttachFiles | ServerPrivileges.AddReactions | ServerPrivileges.Connect;

    private static readonly string[] SendOnly = ["sendMessages"];

    [Fact]
    public async Task ADatabaseFromBeforeGetsViewForItsBaseRoleOnItsFirstStart()
    {
        await using var connection = await OpenAsync();
        await using var db = Context(connection);
        await PrepareReleaseBeforeAsync(db);
        var server = await SeedAsync(db);

        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);

        Assert.Equal(new Dictionary<string, ServerPrivileges>
        {
            ["Owner"] = ServerPrivileges.All,
            ["Admins"] = ServerPrivileges.ManageServer | ServerPrivileges.CreateInvite,
            ["Mods"] = ServerPrivileges.ManageMessages,
            // A second system role above the base role is not the base role.
            ["Veteran"] = ServerPrivileges.None,
            ["Member"] = ServerRole.BaseRoleDefaults,
        }, await PermissionsAsync(db, server));
        Assert.Equal(["ordinary-permissions", "view-channel"], await StepsAsync(connection));
    }

    [Fact]
    public async Task ABaseRoleTheAdministratorTookViewFromKeepsItTakenAcrossRestarts()
    {
        await using var connection = await OpenAsync();
        await using var db = Context(connection);
        await PrepareReleaseBeforeAsync(db);
        var server = await SeedAsync(db);
        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);

        await db.ServerRoles.Where(x => x.ServerId == server && x.Name == "Member")
            .ExecuteUpdateAsync(s => s.SetProperty(x => x.Permissions, ServerPrivileges.SendMessages));
        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);
        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);

        Assert.Equal(ServerPrivileges.SendMessages, (await PermissionsAsync(db, server))["Member"]);
    }

    [Fact]
    public async Task ADatabaseThatHadTheStepIsLeftAsItIs()
    {
        await using var connection = await OpenAsync();
        await using var db = Context(connection);
        await db.Database.EnsureCreatedAsync();
        // A new database runs every step on no data at all, and it is done.
        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);
        var server = await SeedAsync(db);

        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);

        Assert.Equal(OldDefaults, (await PermissionsAsync(db, server))["Member"]);
        Assert.Equal(OldEverything, (await PermissionsAsync(db, server))["Owner"]);
    }

    [Fact]
    public async Task AnUpgradedInstanceShowsEveryMemberTheChannelsTheySawBefore()
    {
        var databasePath = Path.Combine(Path.GetTempPath(), $"stellarr-view-{Guid.NewGuid():N}.db");
        var dataRoot = Path.Combine(Path.GetTempPath(), $"stellarr-view-{Guid.NewGuid():N}");
        StellarrWebApplicationFactory Start() => new(
            loadTestData: true, databasePath: databasePath, dataRoot: dataRoot, deleteStorageOnDispose: false);
        var memberRoleId = Guid.Parse("60000000-0000-0000-0000-000000000003");
        try
        {
            using (var before = Start())
            {
                // Back to how the release before left it: the step before
                // this one run, and the masks of that time.
                await using var scope = before.OpenScope(out var db);
                await db.Database.ExecuteSqlRawAsync("""DELETE FROM "OneTimeSteps" WHERE "Name" = 'view-channel';""");
                await db.ServerRoles.Where(x => x.Id == memberRoleId)
                    .ExecuteUpdateAsync(s => s.SetProperty(x => x.Permissions, OldDefaults));
                await db.ServerRoles.Where(x => x.IsOwner)
                    .ExecuteUpdateAsync(s => s.SetProperty(x => x.Permissions, OldEverything));
            }

            using (var upgraded = Start())
            {
                using var jo = await SignedInAsync(upgraded, "jo", "Corona!2026");
                using var owner = await SignedInAsync(upgraded, "maja", "Aurora!2026");
                Assert.Contains("viewChannel", await PermissionNamesAsync(jo));
                Assert.Contains("viewChannel", await PermissionNamesAsync(owner));

                Assert.Equal(HttpStatusCode.OK, (await owner.PatchAsJsonAsync(
                    $"/api/admin/roles/{memberRoleId}", new { permissions = SendOnly })).StatusCode);
                Assert.Equal(SendOnly, await PermissionNamesAsync(jo));
            }

            using var restarted = Start();
            using var again = await SignedInAsync(restarted, "jo", "Corona!2026");
            Assert.Equal(SendOnly, await PermissionNamesAsync(again));
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

    /// <summary>The caller's server-wide permissions, as the bootstrap names them.</summary>
    private static async Task<string?[]> PermissionNamesAsync(HttpClient client)
    {
        var bootstrap = await client.GetFromJsonAsync<System.Text.Json.JsonElement>("/api/bootstrap");
        return bootstrap.GetProperty("permissions").EnumerateArray().Select(x => x.GetString()).ToArray();
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

    /// <summary>
    /// The schema, and the step table as the release before left it: the
    /// step before this one claimed, this one not.
    /// </summary>
    private static async Task PrepareReleaseBeforeAsync(AppDbContext db)
    {
        await db.Database.EnsureCreatedAsync();
        await db.Database.ExecuteSqlRawAsync(
            """
            CREATE TABLE "OneTimeSteps" (
                "Name" TEXT NOT NULL CONSTRAINT "PK_OneTimeSteps" PRIMARY KEY,
                "AppliedAt" TEXT NOT NULL
            );
            INSERT INTO "OneTimeSteps" ("Name", "AppliedAt") VALUES ('ordinary-permissions', '2026-11-23 12:00:00');
            """);
    }

    /// <summary>A server with the roles of the release before, stored as it stored them.</summary>
    private static async Task<Guid> SeedAsync(AppDbContext db)
    {
        var server = new StellarrServer { Id = Guid.NewGuid(), Name = "Before", CreatedAt = DateTime.UtcNow };
        db.Servers.Add(server);
        db.ServerRoles.AddRange(
            Role(server.Id, "Owner", 0, OldEverything, system: true, isOwner: true),
            Role(server.Id, "Admins", 1, ServerPrivileges.ManageServer | ServerPrivileges.CreateInvite),
            Role(server.Id, "Mods", 2, ServerPrivileges.ManageMessages),
            Role(server.Id, "Veteran", 3, ServerPrivileges.None, system: true),
            Role(server.Id, "Member", 4, OldDefaults, system: true));
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
