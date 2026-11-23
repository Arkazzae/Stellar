using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// Setting the order of the roles between the owner role and the base role,
/// which is their rank, and every way of climbing with it that must fail.
/// Ada holds Lead, which manages roles; Peer ranks with Lead and Higher above
/// it; Jo holds Low and Lower, which rank below Lead with Lowest. Each case
/// has a server of its own.
/// </summary>
public sealed class RoleOrderFlowTests
{
    private static readonly Guid AdaId = Guid.Parse("30000000-0000-0000-0000-000000000002");
    private static readonly Guid JoId = Guid.Parse("30000000-0000-0000-0000-000000000005");
    private static readonly Guid OwnerRoleId = Guid.Parse("60000000-0000-0000-0000-000000000001");
    private static readonly Guid ModeratorRoleId = Guid.Parse("60000000-0000-0000-0000-000000000002");
    private static readonly Guid MemberRoleId = Guid.Parse("60000000-0000-0000-0000-000000000003");
    private static readonly Guid Higher = Guid.Parse("70000000-0000-0000-0000-000000000001");
    private static readonly Guid Lead = Guid.Parse("70000000-0000-0000-0000-000000000002");
    private static readonly Guid Peer = Guid.Parse("70000000-0000-0000-0000-000000000003");
    private static readonly Guid Low = Guid.Parse("70000000-0000-0000-0000-000000000004");
    private static readonly Guid Lower = Guid.Parse("70000000-0000-0000-0000-000000000005");
    private static readonly Guid Lowest = Guid.Parse("70000000-0000-0000-0000-000000000006");

    /// <summary>The order the scenario starts in, highest first; Lead and Peer tie and their ids order them.</summary>
    private static readonly Guid[] Start = [ModeratorRoleId, Higher, Lead, Peer, Low, Lower, Lowest];

    [Fact]
    public async Task TheOwnerSetsAnyOrderAndEveryoneRanksByItAtOnce()
    {
        using var state = await Scenario.CreateAsync();
        Guid[] wanted = [Lowest, ModeratorRoleId, Higher, Lead, Peer, Lower, Low];

        Assert.Equal(HttpStatusCode.NoContent, (await Scenario.ReorderAsync(state.Owner, wanted)).StatusCode);

        Assert.Equal(wanted, await state.OrderAsync());
        await state.WithDbAsync(async db =>
        {
            var roles = await db.ServerRoles.AsNoTracking().Where(x => x.ServerId == TestData.ServerId).ToListAsync();
            Assert.Equal(OwnerRoleId, HeldRoles.Ranked(roles).First().Id);
            Assert.Equal(MemberRoleId, HeldRoles.Ranked(roles).Last().Id);
            Assert.True(roles.Single(x => x.Id == MemberRoleId).Position > roles.Where(x => !x.IsSystem).Max(x => x.Position));
            // Jo holds Low and Lower; Lower ranks higher now, and ranks him.
            Assert.Equal(Lower, (await db.ServerMembers.SingleAsync(x => x.ServerId == TestData.ServerId && x.UserId == JoId)).RoleId);
            Assert.Equal(1, await db.ServerAuditEntries.CountAsync(x => x.Action == "role.reorder"));
        });
    }

    [Fact]
    public async Task AnOrderThatChangesNothingWritesNothing()
    {
        using var state = await Scenario.CreateAsync();

        Assert.Equal(HttpStatusCode.NoContent, (await Scenario.ReorderAsync(state.Owner, Start)).StatusCode);
        await state.WithDbAsync(async db => Assert.False(await db.ServerAuditEntries.AnyAsync(x => x.Action == "role.reorder")));
    }

    [Theory]
    [InlineData("one-missing")]
    [InlineData("one-twice")]
    [InlineData("owner-role-named")]
    [InlineData("base-role-named")]
    [InlineData("unknown-role")]
    [InlineData("none")]
    public async Task TheOrderNamesEveryRoleBetweenTheOwnersAndTheBaseRoleOnce(string mistake)
    {
        using var state = await Scenario.CreateAsync();
        Guid[] wanted = mistake switch
        {
            "one-missing" => Start[..^1],
            "one-twice" => [.. Start[..^1], Start[0]],
            "owner-role-named" => [OwnerRoleId, .. Start],
            "base-role-named" => [.. Start, MemberRoleId],
            "unknown-role" => [.. Start[..^1], Guid.NewGuid()],
            _ => [],
        };

        using var response = await Scenario.ReorderAsync(state.Owner, wanted);

        Assert.Equal(HttpStatusCode.BadRequest, response.StatusCode);
        var problem = await response.Content.ReadFromJsonAsync<JsonElement>();
        Assert.Equal(
            "Lista musi zawierać dokładnie raz każdą rolę między rolą właściciela a rolą domyślną.",
            problem.GetProperty("errors").GetProperty("roleIds")[0].GetString());
        Assert.Equal(Start, await state.OrderAsync());
    }

    [Fact]
    public async Task ReorderingNeedsManagingRoles()
    {
        using var state = await Scenario.CreateAsync();
        using var jo = await state.SignedInAsync("jo", "Corona!2026");

        Assert.Equal(HttpStatusCode.Forbidden, (await Scenario.ReorderAsync(jo, [.. Start.Reverse()])).StatusCode);
        Assert.Equal(Start, await state.OrderAsync());
    }

    [Fact]
    public async Task AnActorMovesTheRolesBelowTheirOwnAndTiesAboveStayTies()
    {
        using var state = await Scenario.CreateAsync();
        Guid[] wanted = [ModeratorRoleId, Higher, Lead, Peer, Lowest, Low, Lower];

        Assert.Equal(HttpStatusCode.NoContent, (await Scenario.ReorderAsync(state.Actor, wanted)).StatusCode);

        Assert.Equal(wanted, await state.OrderAsync());
        await state.WithDbAsync(async db =>
        {
            var positions = await db.ServerRoles.AsNoTracking().Where(x => x.ServerId == TestData.ServerId)
                .ToDictionaryAsync(x => x.Id, x => x.Position);
            // The roles Ada may not move keep their positions, so Peer still ties with Lead.
            Assert.Equal(1, positions[ModeratorRoleId]);
            Assert.Equal(5, positions[Higher]);
            Assert.Equal(10, positions[Lead]);
            Assert.Equal(10, positions[Peer]);
            Assert.True(positions[Lowest] > positions[Peer]);
        });
    }

    [Theory]
    [InlineData("own-role-down")]
    [InlineData("peer-down")]
    [InlineData("lower-role-above-own")]
    [InlineData("lower-role-beside-own")]
    [InlineData("lower-role-to-the-top")]
    [InlineData("higher-role-down")]
    public async Task NobodyClimbsByReordering(string attempt)
    {
        using var state = await Scenario.CreateAsync();
        Guid[] wanted = attempt switch
        {
            "own-role-down" => [ModeratorRoleId, Higher, Peer, Low, Lead, Lower, Lowest],
            "peer-down" => [ModeratorRoleId, Higher, Lead, Low, Peer, Lower, Lowest],
            "lower-role-above-own" => [ModeratorRoleId, Higher, Low, Lead, Peer, Lower, Lowest],
            "lower-role-beside-own" => [ModeratorRoleId, Higher, Lead, Low, Peer, Lower, Lowest],
            "lower-role-to-the-top" => [Low, ModeratorRoleId, Higher, Lead, Peer, Lower, Lowest],
            _ => [ModeratorRoleId, Lead, Peer, Higher, Low, Lower, Lowest],
        };

        Assert.Equal(HttpStatusCode.Forbidden, (await Scenario.ReorderAsync(state.Actor, wanted)).StatusCode);
        Assert.Equal(Start, await state.OrderAsync());
    }

    private sealed class Scenario : IDisposable
    {
        internal StellarrWebApplicationFactory Factory { get; } = new();
        internal HttpClient Actor { get; private set; } = null!;
        internal HttpClient Owner { get; private set; } = null!;

        internal static async Task<Scenario> CreateAsync()
        {
            var state = new Scenario();
            await state.WithDbAsync(async db =>
            {
                db.ServerRoles.AddRange(
                    Role(Higher, "Higher", 5, ServerPrivileges.None),
                    Role(Lead, "Lead", 10, ServerPrivileges.ManageRoles),
                    Role(Peer, "Peer", 10, ServerPrivileges.None),
                    Role(Low, "Low", 20, ServerPrivileges.None),
                    Role(Lower, "Lower", 21, ServerPrivileges.None),
                    Role(Lowest, "Lowest", 22, ServerPrivileges.None));
                db.ServerMemberRoles.AddRange(
                    new ServerMemberRole { ServerId = TestData.ServerId, UserId = AdaId, RoleId = Lead },
                    new ServerMemberRole { ServerId = TestData.ServerId, UserId = JoId, RoleId = Low },
                    new ServerMemberRole { ServerId = TestData.ServerId, UserId = JoId, RoleId = Lower });
                await db.SaveChangesAsync();
                // Ada holds Lead in place of Moderacja, which the fixture gives her without a row.
                await db.ServerMembers.Where(x => x.UserId == AdaId).ExecuteUpdateAsync(s => s.SetProperty(x => x.RoleId, Lead));
                await db.ServerMembers.Where(x => x.UserId == JoId).ExecuteUpdateAsync(s => s.SetProperty(x => x.RoleId, Low));
                await db.ServerRoles.Where(x => x.Id == MemberRoleId).ExecuteUpdateAsync(s => s.SetProperty(x => x.Position, 30));
            });
            state.Actor = await state.SignedInAsync("ada", "Quasar!2026");
            state.Owner = await state.SignedInAsync("maja", "Aurora!2026");
            return state;
        }

        internal static Task<HttpResponseMessage> ReorderAsync(HttpClient client, Guid[] roleIds) =>
            client.PutAsJsonAsync("/api/admin/roles/order", new { roleIds });

        /// <summary>The roles between the owner role and the base role, highest first, as the server ranks them.</summary>
        internal async Task<Guid[]> OrderAsync()
        {
            await using var scope = Factory.OpenScope(out var db);
            var roles = await db.ServerRoles.AsNoTracking().Where(x => x.ServerId == TestData.ServerId).ToListAsync();
            return HeldRoles.Ranked(roles.Where(x => !x.IsOwner && x.Id != MemberRoleId)).Select(x => x.Id).ToArray();
        }

        internal async Task WithDbAsync(Func<AppDbContext, Task> action)
        {
            await using var scope = Factory.OpenScope(out var db);
            await action(db);
        }

        internal async Task<HttpClient> SignedInAsync(string username, string password)
        {
            var client = Factory.CreateClient(new WebApplicationFactoryClientOptions { AllowAutoRedirect = false, HandleCookies = true });
            (await client.PostAsJsonAsync("/api/auth/login", new { username, password })).EnsureSuccessStatusCode();
            return client;
        }

        private static ServerRole Role(Guid id, string name, int position, ServerPrivileges permissions) => new()
        {
            Id = id, ServerId = TestData.ServerId, Name = name, Color = "#123456", Position = position, Permissions = permissions,
        };

        public void Dispose()
        {
            Actor.Dispose();
            Owner.Dispose();
            Factory.Dispose();
        }
    }
}
