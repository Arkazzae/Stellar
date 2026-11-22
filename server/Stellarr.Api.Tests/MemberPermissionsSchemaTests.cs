using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Data;
using Stellarr.Api.Data.Schema;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// A database made before members could hold several roles: every table but
/// the two new ones, as the model creates them. The step must give it those
/// two exactly as a new database has them, and hold each member's old role.
/// </summary>
public sealed class MemberPermissionsSchemaTests
{
    private static readonly string[] NewTables = ["ServerMemberRoles", "MemberPermissionOverrides"];

    [Fact]
    public async Task UpgradedDatabaseGetsTheTablesANewOneHasAndARepeatChangesNothing()
    {
        await using var fresh = await OpenAsync();
        await using var freshDb = Context(fresh);
        await freshDb.Database.EnsureCreatedAsync();

        await using var upgraded = await OpenAsync();
        await using var upgradedDb = Context(upgraded);
        await CreateBeforeTheChangeAsync(upgradedDb);
        await MemberPermissionsSchema.CreateAsync(upgradedDb, CancellationToken.None);
        await MemberPermissionsSchema.CreateAsync(upgradedDb, CancellationToken.None);

        foreach (var table in NewTables)
        {
            Assert.Equal(await ShapeAsync(fresh, table), await ShapeAsync(upgraded, table));
        }
    }

    [Fact]
    public async Task EveryMemberKeepsTheirOldRoleExceptTheBaseRoleWhichNobodyStores()
    {
        await using var connection = await OpenAsync();
        await using var db = Context(connection);
        await CreateBeforeTheChangeAsync(db);
        var seeded = await SeedAsync(db);

        await MemberPermissionsSchema.CreateAsync(db, CancellationToken.None);
        await MemberPermissionsSchema.CreateAsync(db, CancellationToken.None);

        var held = await db.ServerMemberRoles.AsNoTracking()
            .Select(x => new { x.UserId, x.RoleId }).ToListAsync();
        Assert.Equal(3, held.Count);
        Assert.Contains(held, x => x.UserId == seeded.Owner && x.RoleId == seeded.OwnerRole);
        Assert.Contains(held, x => x.UserId == seeded.Moderator && x.RoleId == seeded.ModeratorRole);
        // A second system role that ranks above the base role is an ordinary role to hold.
        Assert.Contains(held, x => x.UserId == seeded.Veteran && x.RoleId == seeded.VeteranRole);
        Assert.DoesNotContain(held, x => x.UserId == seeded.Plain);
    }

    [Fact]
    public async Task ABackupFromBeforeTheChangeStartsAndItsMembersKeepTheirPowers()
    {
        await using var connection = await OpenAsync();
        await using var db = Context(connection);
        await CreateBeforeTheChangeAsync(db);
        var seeded = await SeedAsync(db);

        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);
        db.ChangeTracker.Clear();

        Assert.Equal(ServerPrivileges.All, await ServerAccess.GetAsync(db, seeded.Owner, seeded.Server));
        Assert.Equal(ServerPrivileges.ManageMessages, await ServerAccess.GetAsync(db, seeded.Moderator, seeded.Server));
        Assert.Equal(ServerPrivileges.None, await ServerAccess.GetAsync(db, seeded.Plain, seeded.Server));
    }

    private static async Task<SqliteConnection> OpenAsync()
    {
        var connection = new SqliteConnection("Data Source=:memory:;Foreign Keys=True");
        await connection.OpenAsync();
        return connection;
    }

    private static AppDbContext Context(SqliteConnection connection) =>
        new(new DbContextOptionsBuilder<AppDbContext>().UseSqlite(connection).Options);

    /// <summary>The model's schema without the tables this change added.</summary>
    private static async Task CreateBeforeTheChangeAsync(AppDbContext db)
    {
        await db.Database.EnsureCreatedAsync();
        foreach (var table in NewTables)
        {
#pragma warning disable EF1002 // The names come from the list above.
            await db.Database.ExecuteSqlRawAsync($"DROP TABLE \"{table}\";");
#pragma warning restore EF1002
        }
    }

    private sealed record Seeded(
        Guid Server, Guid Owner, Guid Moderator, Guid Veteran, Guid Plain,
        Guid OwnerRole, Guid ModeratorRole, Guid VeteranRole);

    /// <summary>Members as the release before stored them, each with one role and nothing else.</summary>
    private static async Task<Seeded> SeedAsync(AppDbContext db)
    {
        var server = new StellarrServer { Id = Guid.NewGuid(), Name = "Before", CreatedAt = DateTime.UtcNow };
        var owner = Role(server.Id, "Owner", 0, ServerPrivileges.All, system: true, isOwner: true);
        var moderator = Role(server.Id, "Moderator", 1, ServerPrivileges.ManageMessages);
        var veteran = Role(server.Id, "Veteran", 2, ServerPrivileges.None, system: true);
        var member = Role(server.Id, "Member", 3, ServerPrivileges.None, system: true);
        var users = Enumerable.Range(0, 4).Select(index => new User
        {
            Id = Guid.NewGuid(),
            Username = $"user{index}",
            NormalizedUsername = $"USER{index}",
            DisplayName = $"User {index}",
            PasswordHash = "hash",
            AvatarColor = "#777777",
            CreatedAt = DateTime.UtcNow,
        }).ToArray();
        db.Servers.Add(server);
        db.ServerRoles.AddRange(owner, moderator, veteran, member);
        db.Users.AddRange(users);
        db.ServerMembers.AddRange(
            Member(server.Id, users[0].Id, owner.Id, ownerSlot: 1),
            Member(server.Id, users[1].Id, moderator.Id),
            Member(server.Id, users[2].Id, veteran.Id),
            Member(server.Id, users[3].Id, member.Id));
        await db.SaveChangesAsync();
        db.ChangeTracker.Clear();
        return new Seeded(server.Id, users[0].Id, users[1].Id, users[2].Id, users[3].Id, owner.Id, moderator.Id, veteran.Id);
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

    private static ServerMember Member(Guid serverId, Guid userId, Guid roleId, int? ownerSlot = null) => new()
    {
        ServerId = serverId,
        UserId = userId,
        RoleId = roleId,
        OwnerSlot = ownerSlot,
        JoinedAt = DateTime.UtcNow,
    };

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
