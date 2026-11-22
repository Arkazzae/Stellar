using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Data.Schema;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// The base role, which everybody holds, ranks below every role somebody is
/// given: when a role is made in the role editor, and in a database from
/// before new roles were placed above it.
/// </summary>
public sealed class RoleRankTests
{
    private static readonly Guid NilsId = Guid.Parse("30000000-0000-0000-0000-000000000004");
    private static readonly Guid JoId = Guid.Parse("30000000-0000-0000-0000-000000000005");
    private static readonly string[] ModerateMembers = ["moderateMembers"];

    [Fact]
    public async Task ARoleMadeInTheRoleEditorOutranksThoseWhoWereGivenNone()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var owner = await SignedInAsync(factory, "maja", "Aurora!2026");
        using var guard = await SignedInAsync(factory, "jo", "Corona!2026");

        var roleId = await CreateRoleAsync(owner, "Straż", ModerateMembers);
        var listed = await ListRolesAsync(owner);
        // The list comes highest first, and the base role closes it.
        Assert.Equal(roleId, listed[^2].Id);
        Assert.True(listed[^1] is { IsSystem: true, IsOwner: false });

        Assert.Equal(HttpStatusCode.NoContent,
            (await owner.PatchAsJsonAsync($"/api/admin/members/{JoId}/role", new { roleId })).StatusCode);
        // Nils was given no role, so the holder of a new one may moderate him.
        Assert.Equal(HttpStatusCode.NoContent, (await guard.DeleteAsync($"/api/admin/members/{NilsId}")).StatusCode);
    }

    [Fact]
    public async Task RolesMadeOneAfterAnotherRankInThatOrderAboveTheBaseRole()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var owner = await SignedInAsync(factory, "maja", "Aurora!2026");

        var first = await CreateRoleAsync(owner, "Pierwsza", []);
        var second = await CreateRoleAsync(owner, "Druga", []);

        var listed = await ListRolesAsync(owner);
        Assert.Equal(first, listed[^3].Id);
        Assert.Equal(second, listed[^2].Id);
        Assert.True(listed[^1] is { IsSystem: true, IsOwner: false });
    }

    [Fact]
    public async Task ADatabaseFromBeforeGetsItsBaseRoleLastAndARepeatChangesNothing()
    {
        await using var connection = await OpenAsync();
        await using var db = Context(connection);
        await db.Database.EnsureCreatedAsync();
        // As the role editor used to leave a server: each new role after the base role.
        var before = await SeedAsync(db, "Before",
            Role("Owner", 0, system: true, isOwner: true),
            Role("Member", 1, system: true),
            Role("Guard", 2),
            Role("Helper", 3));
        // A server already in order, with a second system role above the base role.
        var inOrder = await SeedAsync(db, "In order",
            Role("Owner", 0, system: true, isOwner: true),
            Role("Veteran", 1, system: true),
            Role("Guard", 2),
            Role("Member", 3, system: true));

        await RoleRankSchema.RepairAsync(db, CancellationToken.None);
        var once = await PositionsAsync(db, before);
        await RoleRankSchema.RepairAsync(db, CancellationToken.None);

        Assert.Equal(["Guard:2", "Helper:3", "Member:4", "Owner:0"], once);
        Assert.Equal(once, await PositionsAsync(db, before));
        Assert.Equal(["Guard:2", "Member:3", "Owner:0", "Veteran:1"], await PositionsAsync(db, inOrder));
    }

    [Fact]
    public async Task OnlyTheBaseRoleMovesWhenASecondSystemRoleRanksAboveIt()
    {
        await using var connection = await OpenAsync();
        await using var db = Context(connection);
        await db.Database.EnsureCreatedAsync();
        var server = await SeedAsync(db, "Two system roles",
            Role("Owner", 0, system: true, isOwner: true),
            Role("Veteran", 1, system: true),
            Role("Member", 2, system: true),
            Role("Guard", 3));

        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);
        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);

        Assert.Equal(["Guard:3", "Member:4", "Owner:0", "Veteran:1"], await PositionsAsync(db, server));
    }

    private sealed record ListedRole(string Id, bool IsSystem, bool IsOwner);

    private static async Task<HttpClient> SignedInAsync(
        StellarrWebApplicationFactory factory, string username, string password)
    {
        var client = factory.CreateClient(new WebApplicationFactoryClientOptions { AllowAutoRedirect = false, HandleCookies = true });
        (await client.PostAsJsonAsync("/api/auth/login", new { username, password })).EnsureSuccessStatusCode();
        return client;
    }

    private static async Task<string> CreateRoleAsync(HttpClient client, string name, string[] permissions)
    {
        var created = await client.PostAsJsonAsync("/api/admin/roles", new { name, color = "#7c6ff0", permissions });
        Assert.Equal(HttpStatusCode.Created, created.StatusCode);
        using var role = JsonDocument.Parse(await created.Content.ReadAsStringAsync());
        return role.RootElement.GetProperty("id").GetString()!;
    }

    private static async Task<ListedRole[]> ListRolesAsync(HttpClient client)
    {
        using var roles = JsonDocument.Parse(await client.GetStringAsync("/api/admin/roles"));
        return roles.RootElement.EnumerateArray()
            .Select(x => new ListedRole(
                x.GetProperty("id").GetString()!,
                x.GetProperty("isSystem").GetBoolean(),
                x.GetProperty("isOwner").GetBoolean()))
            .ToArray();
    }

    private static async Task<SqliteConnection> OpenAsync()
    {
        var connection = new SqliteConnection("Data Source=:memory:;Foreign Keys=True");
        await connection.OpenAsync();
        return connection;
    }

    private static AppDbContext Context(SqliteConnection connection) =>
        new(new DbContextOptionsBuilder<AppDbContext>().UseSqlite(connection).Options);

    private static ServerRole Role(string name, int position, bool system = false, bool isOwner = false) => new()
    {
        Id = Guid.NewGuid(),
        Name = name,
        Color = "#777777",
        Permissions = isOwner ? ServerPrivileges.All : ServerPrivileges.None,
        Position = position,
        IsSystem = system,
        IsOwner = isOwner,
    };

    private static async Task<Guid> SeedAsync(AppDbContext db, string name, params ServerRole[] roles)
    {
        var server = new StellarrServer { Id = Guid.NewGuid(), Name = name, CreatedAt = DateTime.UtcNow };
        foreach (var role in roles)
        {
            role.ServerId = server.Id;
        }

        db.Servers.Add(server);
        db.ServerRoles.AddRange(roles);
        await db.SaveChangesAsync();
        db.ChangeTracker.Clear();
        return server.Id;
    }

    /// <summary>The server's roles as <c>name:position</c>, by name.</summary>
    private static async Task<string[]> PositionsAsync(AppDbContext db, Guid serverId) =>
        (await db.ServerRoles.AsNoTracking().Where(x => x.ServerId == serverId).ToListAsync())
            .OrderBy(x => x.Name, StringComparer.Ordinal)
            .Select(x => $"{x.Name}:{x.Position}")
            .ToArray();
}
