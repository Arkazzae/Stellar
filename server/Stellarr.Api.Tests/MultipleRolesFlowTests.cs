using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Http.Connections;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// Members who hold several roles: what they may do, who may change what they
/// hold, and every way of escalating through a second role that must fail.
/// Each case has a server state of its own.
/// </summary>
public sealed class MultipleRolesFlowTests
{
    private static readonly Guid MajaId = Guid.Parse("30000000-0000-0000-0000-000000000001");
    private static readonly Guid AdaId = Guid.Parse("30000000-0000-0000-0000-000000000002");
    private static readonly Guid JoId = Guid.Parse("30000000-0000-0000-0000-000000000005");
    private static readonly Guid OwnerRoleId = Guid.Parse("60000000-0000-0000-0000-000000000001");
    private static readonly Guid MemberRoleId = Guid.Parse("60000000-0000-0000-0000-000000000003");
    private static readonly string[] ManageMessages = ["manageMessages"];
    private static readonly string[] ManageBans = ["manageBans"];
    private static readonly string[] Nothing = [];
    private static readonly string[] ViewAuditLog = ["viewAuditLog"];

    [Fact]
    public async Task PermissionsUniteEveryHeldRoleWhileTheTopRoleRanks()
    {
        using var state = await Scenario.CreateAsync();

        var permissions = JsonDocument.Parse(await state.Actor.GetStringAsync("/api/bootstrap")).RootElement
            .GetProperty("permissions").EnumerateArray().Select(x => x.GetString()).ToArray();
        Assert.Contains("manageRoles", permissions);
        // Only the Helper role, below the actor's top role, grants this one.
        Assert.Contains("moderateMembers", permissions);
        Assert.Equal(HttpStatusCode.NoContent, (await state.Actor.DeleteAsync($"/api/admin/members/{JoId}")).StatusCode);
    }

    [Fact]
    public async Task TheListGivesAndTakesRolesAndTheMemberRanksByTheHighest()
    {
        using var state = await Scenario.CreateAsync();

        Assert.Equal(HttpStatusCode.NoContent, (await Scenario.SetRolesAsync(state.Actor, JoId, state.Lower, state.Gift)).StatusCode);
        var jo = await state.MemberAsync(JoId);
        Assert.Equal(state.Lower.ToString(), jo.GetProperty("roleId").GetString());
        Assert.Equal([state.Lower.ToString(), state.Gift.ToString()], RoleIds(jo));

        // The base role needs no naming; naming it changes nothing.
        Assert.Equal(HttpStatusCode.NoContent, (await Scenario.SetRolesAsync(state.Actor, JoId, state.Gift, MemberRoleId)).StatusCode);
        jo = await state.MemberAsync(JoId);
        Assert.Equal(state.Gift.ToString(), jo.GetProperty("roleId").GetString());
        Assert.Equal([state.Gift.ToString()], RoleIds(jo));

        Assert.Equal(HttpStatusCode.NoContent, (await Scenario.SetRolesAsync(state.Actor, JoId)).StatusCode);
        jo = await state.MemberAsync(JoId);
        Assert.Equal(MemberRoleId.ToString(), jo.GetProperty("roleId").GetString());
        Assert.Empty(RoleIds(jo));

        await state.WithDbAsync(async db =>
        {
            var entries = await db.ServerAuditEntries.AsNoTracking()
                .Where(x => x.Action == "member.role" && x.TargetId == JoId).ToListAsync();
            Assert.Equal(3, entries.Count);
            Assert.All(entries, x => Assert.Contains("\"field\":\"roles\"", x.ChangesJson, StringComparison.Ordinal));
        });
    }

    [Theory]
    [InlineData("peer", HttpStatusCode.Forbidden)]
    [InlineData("higher", HttpStatusCode.Forbidden)]
    [InlineData("power-the-actor-lacks", HttpStatusCode.Forbidden)]
    [InlineData("hidden-grant", HttpStatusCode.Forbidden)]
    [InlineData("owner-role", HttpStatusCode.Conflict)]
    [InlineData("unknown-role", HttpStatusCode.NotFound)]
    [InlineData("gift", HttpStatusCode.NoContent)]
    public async Task GivingARoleNeedsItBelowTheActorAndEveryPowerItCouldGrant(string role, HttpStatusCode expected)
    {
        using var state = await Scenario.CreateAsync();
        var given = role switch
        {
            "peer" => state.Peer,
            "higher" => state.Higher,
            "power-the-actor-lacks" => state.Banner,
            "hidden-grant" => state.Hidden,
            "owner-role" => OwnerRoleId,
            "unknown-role" => Guid.NewGuid(),
            _ => state.Gift,
        };

        Assert.Equal(expected, (await Scenario.SetRolesAsync(state.Actor, JoId, state.Lower, given)).StatusCode);
        Assert.Equal(expected, (await state.Actor.PatchAsJsonAsync($"/api/admin/members/{JoId}/role", new { roleId = given })).StatusCode);
        await state.WithDbAsync(async db => Assert.Equal(
            expected == HttpStatusCode.NoContent,
            await db.ServerMemberRoles.AnyAsync(x => x.UserId == JoId && x.RoleId == given)));
    }

    [Theory]
    [InlineData("power-the-actor-lacks")]
    [InlineData("hidden-grant")]
    public async Task TakingARoleAwayNeedsEveryPowerItGrantsAsWell(string role)
    {
        using var state = await Scenario.CreateAsync();
        var held = role == "hidden-grant" ? state.Hidden : state.Banner;
        await state.HoldAsync(JoId, state.Lower, held);

        Assert.Equal(HttpStatusCode.Forbidden, (await Scenario.SetRolesAsync(state.Actor, JoId, state.Lower)).StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden,
            (await state.Actor.PatchAsJsonAsync($"/api/admin/members/{JoId}/role", new { roleId = state.Lower })).StatusCode);
        // A change that leaves that role alone is the actor's to make.
        Assert.Equal(HttpStatusCode.NoContent, (await Scenario.SetRolesAsync(state.Actor, JoId, state.Lower, held, state.Gift)).StatusCode);
    }

    [Fact]
    public async Task AMemberWhoseHighestRoleMatchesTheActorsIsOutOfReachWhateverElseTheyHold()
    {
        using var state = await Scenario.CreateAsync();
        await state.HoldAsync(JoId, state.Lower, state.Peer);

        Assert.Equal(HttpStatusCode.Forbidden, (await Scenario.SetRolesAsync(state.Actor, JoId, state.Peer)).StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden, (await state.Actor.DeleteAsync($"/api/admin/members/{JoId}")).StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden,
            (await Scenario.SetMemberOverrideAsync(state.Actor, JoId, ManageMessages, Nothing)).StatusCode);
    }

    [Fact]
    public async Task NobodyChangesTheirOwnRolesOrThoseOfTheOwner()
    {
        using var state = await Scenario.CreateAsync();

        Assert.Equal(HttpStatusCode.Forbidden, (await Scenario.SetRolesAsync(state.Actor, AdaId, state.Lead, state.Helper, state.Gift)).StatusCode);
        Assert.Equal(HttpStatusCode.Conflict, (await Scenario.SetRolesAsync(state.Actor, MajaId)).StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden, (await Scenario.SetRolesAsync(state.Owner, MajaId, OwnerRoleId, state.Gift)).StatusCode);
    }

    [Fact]
    public async Task ThePatchOfOneRoleLeavesThatRoleAlone()
    {
        using var state = await Scenario.CreateAsync();
        await state.HoldAsync(JoId, state.Lower, state.Gift);

        Assert.Equal(HttpStatusCode.NoContent,
            (await state.Actor.PatchAsJsonAsync($"/api/admin/members/{JoId}/role", new { roleId = state.Gift })).StatusCode);
        var jo = await state.MemberAsync(JoId);
        Assert.Equal(state.Gift.ToString(), jo.GetProperty("roleId").GetString());
        Assert.Equal([state.Gift.ToString()], RoleIds(jo));

        Assert.Equal(HttpStatusCode.NoContent,
            (await state.Actor.PatchAsJsonAsync($"/api/admin/members/{JoId}/role", new { roleId = MemberRoleId })).StatusCode);
        jo = await state.MemberAsync(JoId);
        Assert.Equal(MemberRoleId.ToString(), jo.GetProperty("roleId").GetString());
        Assert.Empty(RoleIds(jo));
    }

    [Fact]
    public async Task DeletingTheTopRoleRanksItsHoldersByTheirNextRole()
    {
        using var state = await Scenario.CreateAsync();
        await state.HoldAsync(JoId, state.Lower, state.Gift);

        Assert.Equal(HttpStatusCode.NoContent, (await state.Owner.DeleteAsync($"/api/admin/roles/{state.Lower}")).StatusCode);

        var jo = await state.MemberAsync(JoId);
        Assert.Equal(state.Gift.ToString(), jo.GetProperty("roleId").GetString());
        Assert.Equal([state.Gift.ToString()], RoleIds(jo));
    }

    [Fact]
    public async Task RoleCountsCountHoldersAndTheBaseRoleCountsEverybody()
    {
        using var state = await Scenario.CreateAsync();
        await state.HoldAsync(JoId, state.Lower, state.Helper);

        var roles = JsonDocument.Parse(await state.Owner.GetStringAsync("/api/admin/roles")).RootElement.EnumerateArray()
            .ToDictionary(x => x.GetProperty("id").GetString()!, x => x.GetProperty("memberCount").GetInt32());
        Assert.Equal(5, roles[MemberRoleId.ToString()]);
        Assert.Equal(2, roles[state.Helper.ToString()]);
        Assert.Equal(1, roles[state.Lead.ToString()]);
        Assert.Equal(1, roles[OwnerRoleId.ToString()]);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task RemovedMembersLoseTheirRolesAndOverrides(bool ban)
    {
        using var state = await Scenario.CreateAsync();
        await state.HoldAsync(JoId, state.Lower, state.Gift);
        Assert.Equal(HttpStatusCode.OK, (await Scenario.SetMemberOverrideAsync(state.Owner, JoId, ManageMessages, Nothing)).StatusCode);

        using var removed = ban
            ? await state.Owner.PostAsJsonAsync($"/api/admin/members/{JoId}/ban", new { reason = "" })
            : await state.Owner.DeleteAsync($"/api/admin/members/{JoId}");
        Assert.True(removed.IsSuccessStatusCode);

        await state.WithDbAsync(async db =>
        {
            Assert.False(await db.ServerMemberRoles.AnyAsync(x => x.UserId == JoId));
            Assert.False(await db.MemberPermissionOverrides.AnyAsync(x => x.UserId == JoId));
        });
    }

    [Fact]
    public async Task HandingTheServerOverMovesTheOwnerRoleAndLeavesOtherRolesWhereTheyAre()
    {
        using var state = await Scenario.CreateAsync();
        await state.HoldAsync(JoId, state.Gift);
        Assert.Equal(HttpStatusCode.OK, (await Scenario.SetMemberOverrideAsync(state.Owner, JoId, Nothing, ManageMessages)).StatusCode);

        Assert.Equal(HttpStatusCode.NoContent,
            (await state.Owner.PostAsJsonAsync("/api/admin/server/transfer", new { userId = JoId })).StatusCode);

        var jo = await state.MemberAsync(JoId);
        Assert.Equal(OwnerRoleId.ToString(), jo.GetProperty("roleId").GetString());
        Assert.Equal([OwnerRoleId.ToString(), state.Gift.ToString()], RoleIds(jo));
        var maja = await state.MemberAsync(MajaId);
        Assert.Equal(MemberRoleId.ToString(), maja.GetProperty("roleId").GetString());
        Assert.Empty(RoleIds(maja));
        await state.WithDbAsync(async db => Assert.False(await db.MemberPermissionOverrides.AnyAsync(x => x.UserId == JoId)));
    }

    [Fact]
    public async Task InAChannelAnAllowOfOneRoleBeatsADenyOfAnotherAndTheMemberHasTheLastWord()
    {
        using var state = await Scenario.CreateAsync();
        await state.HoldAsync(JoId, state.Lower, state.Gift);
        await state.RoleOverrideAsync(state.Lower, Nothing, ManageMessages);
        Assert.False(await state.JoMayAsync(ServerPrivileges.ManageMessages));

        await state.RoleOverrideAsync(state.Gift, ManageMessages, Nothing);
        Assert.True(await state.JoMayAsync(ServerPrivileges.ManageMessages));

        Assert.Equal(HttpStatusCode.OK, (await Scenario.SetMemberOverrideAsync(state.Owner, JoId, Nothing, ManageMessages)).StatusCode);
        Assert.False(await state.JoMayAsync(ServerPrivileges.ManageMessages));
        Assert.Equal(HttpStatusCode.OK, (await Scenario.SetMemberOverrideAsync(state.Owner, JoId, ManageBans, Nothing)).StatusCode);
        Assert.True(await state.JoMayAsync(ServerPrivileges.ManageBans));
    }

    [Fact]
    public async Task TheBaseRolesOverridesReachMembersWhoHoldOtherRolesUnlessOneOfThemAllows()
    {
        using var state = await Scenario.CreateAsync();
        await state.HoldAsync(JoId, state.Gift);
        await state.RoleOverrideAsync(MemberRoleId, Nothing, ManageMessages);
        Assert.False(await state.JoMayAsync(ServerPrivileges.ManageMessages));

        await state.RoleOverrideAsync(state.Gift, ManageMessages, Nothing);
        Assert.True(await state.JoMayAsync(ServerPrivileges.ManageMessages));
    }

    [Fact]
    public async Task MemberOverridesAreListedApartFromRoleOverrides()
    {
        using var state = await Scenario.CreateAsync();

        using var set = await Scenario.SetMemberOverrideAsync(state.Actor, JoId, ManageMessages, Nothing);
        Assert.Equal(HttpStatusCode.OK, set.StatusCode);
        var body = JsonDocument.Parse(await set.Content.ReadAsStringAsync()).RootElement;
        Assert.Equal(JoId.ToString(), body.GetProperty("userId").GetString());
        Assert.Equal(ManageMessages, body.GetProperty("allow").EnumerateArray().Select(x => x.GetString()));

        Assert.Empty(JsonDocument.Parse(await state.Actor.GetStringAsync(Scenario.OverridesUrl)).RootElement.EnumerateArray());
        var listed = Assert.Single(JsonDocument.Parse(
            await state.Actor.GetStringAsync($"{Scenario.OverridesUrl}/members")).RootElement.EnumerateArray());
        Assert.Equal(JoId.ToString(), listed.GetProperty("userId").GetString());
        await state.WithDbAsync(async db => Assert.True(await db.ServerAuditEntries.AnyAsync(
            x => x.Action == "channel.permissions.member" && x.TargetId == TestData.GeneralChannelId)));

        Assert.Equal(HttpStatusCode.NoContent, (await state.Actor.DeleteAsync($"{Scenario.OverridesUrl}/members/{JoId}")).StatusCode);
        Assert.Empty(JsonDocument.Parse(await state.Actor.GetStringAsync($"{Scenario.OverridesUrl}/members")).RootElement.EnumerateArray());
    }

    [Theory]
    [InlineData("grant-a-power-the-actor-lacks", HttpStatusCode.Forbidden)]
    [InlineData("deny-a-power-the-actor-lacks", HttpStatusCode.Forbidden)]
    [InlineData("lift-a-deny-the-actor-could-not-set", HttpStatusCode.Forbidden)]
    [InlineData("self", HttpStatusCode.Forbidden)]
    [InlineData("owner", HttpStatusCode.Conflict)]
    [InlineData("scoped-manage-roles-deny", HttpStatusCode.Forbidden)]
    [InlineData("no-manage-roles", HttpStatusCode.Forbidden)]
    [InlineData("not-a-member", HttpStatusCode.NotFound)]
    [InlineData("allowed", HttpStatusCode.OK)]
    public async Task AMemberOverrideObeysTheSameBoundariesAsARolesOverride(string attempt, HttpStatusCode expected)
    {
        using var state = await Scenario.CreateAsync();
        var target = attempt switch
        {
            "self" => AdaId,
            "owner" => MajaId,
            "not-a-member" => Guid.NewGuid(),
            _ => JoId,
        };
        if (attempt == "lift-a-deny-the-actor-could-not-set")
        {
            Assert.Equal(HttpStatusCode.OK, (await Scenario.SetMemberOverrideAsync(state.Owner, JoId, Nothing, ManageBans)).StatusCode);
        }
        if (attempt == "scoped-manage-roles-deny")
        {
            await state.RoleOverrideAsync(state.Helper, Nothing, ["manageRoles"]);
        }
        if (attempt == "no-manage-roles")
        {
            await state.HoldAsync(AdaId, state.Helper);
        }

        using var response = attempt switch
        {
            "grant-a-power-the-actor-lacks" => await Scenario.SetMemberOverrideAsync(state.Actor, target, ManageBans, Nothing),
            "deny-a-power-the-actor-lacks" => await Scenario.SetMemberOverrideAsync(state.Actor, target, Nothing, ManageBans),
            "lift-a-deny-the-actor-could-not-set" => await state.Actor.DeleteAsync($"{Scenario.OverridesUrl}/members/{target}"),
            _ => await Scenario.SetMemberOverrideAsync(state.Actor, target, ManageMessages, Nothing),
        };
        Assert.Equal(expected, response.StatusCode);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task MovingAChannelBetweenSectionsWhereAMembersOverridesDifferIsAPermissionChange(bool managesRoles)
    {
        using var state = await Scenario.CreateAsync();
        var source = Guid.NewGuid();
        var destination = Guid.NewGuid();
        await state.WithDbAsync(async db =>
        {
            db.Sections.AddRange(
                new Section { Id = source, ServerId = TestData.ServerId, Name = "Source", Position = 40 },
                new Section { Id = destination, ServerId = TestData.ServerId, Name = "Destination", Position = 41 });
            db.MemberPermissionOverrides.Add(new MemberPermissionOverride
            {
                Id = Guid.NewGuid(), ServerId = TestData.ServerId, UserId = JoId,
                TargetType = PermissionTargetType.Section, TargetId = destination, DenyMask = ServerPrivileges.ManageMessages,
            });
            await db.SaveChangesAsync();
            await db.Channels.Where(x => x.Id == TestData.GeneralChannelId)
                .ExecuteUpdateAsync(s => s.SetProperty(x => x.SectionId, source));
            await db.ServerRoles.Where(x => x.Id == state.Lead).ExecuteUpdateAsync(s => s.SetProperty(x => x.Permissions,
                ServerPrivileges.ManageChannels | ServerPrivileges.ManageMessages |
                (managesRoles ? ServerPrivileges.ManageRoles : ServerPrivileges.None)));
        });

        using var response = await ChannelSections.FileAsync(state.Actor, TestData.GeneralChannelId, destination);
        Assert.Equal(managesRoles ? HttpStatusCode.NoContent : HttpStatusCode.Forbidden, response.StatusCode);
    }

    [Fact]
    public async Task EveryoneWhoseRolesChangeHearsTheirOwnPermissions()
    {
        using var state = await Scenario.CreateAsync();
        await using var connection = await ListenAsync(state.Factory, "jo", "Corona!2026");

        Assert.Equal(HttpStatusCode.NoContent, (await Scenario.SetRolesAsync(state.Owner, JoId, state.Gift, state.Banner)).StatusCode);
        Assert.Equal(["manageMessages", "manageBans"], Permissions(await connection.NextAsync()));

        // A change to the base role reaches members who hold other roles too.
        Assert.Equal(HttpStatusCode.OK, (await state.Owner.PatchAsJsonAsync(
            $"/api/admin/roles/{MemberRoleId}", new { permissions = ViewAuditLog })).StatusCode);
        Assert.Equal(["manageMessages", "manageBans", "viewAuditLog"], Permissions(await connection.NextAsync()));

        Assert.Equal(HttpStatusCode.NoContent, (await state.Owner.DeleteAsync($"/api/admin/roles/{state.Banner}")).StatusCode);
        Assert.Equal(["manageMessages", "viewAuditLog"], Permissions(await connection.NextAsync()));
    }

    private static string[] RoleIds(JsonElement member) =>
        member.GetProperty("roleIds").EnumerateArray().Select(x => x.GetString()!).ToArray();

    private static string[] Permissions(JsonElement changed) =>
        changed.GetProperty("permissions").EnumerateArray().Select(x => x.GetString()!).ToArray();

    private static async Task<HubListener> ListenAsync(StellarrWebApplicationFactory factory, string username, string password)
    {
        using var client = factory.CreateClient(new WebApplicationFactoryClientOptions { HandleCookies = false });
        using var login = await client.PostAsJsonAsync("/api/auth/login", new { username, password });
        login.EnsureSuccessStatusCode();
        var cookie = string.Join("; ", login.Headers.GetValues("Set-Cookie").Select(x => x.Split(';', 2)[0]));
        var connection = new HubConnectionBuilder()
            .WithUrl("https://localhost/hubs/app", options =>
            {
                options.Headers["Cookie"] = cookie;
                options.Transports = HttpTransportType.LongPolling;
                options.HttpMessageHandlerFactory = _ => factory.Server.CreateHandler();
            })
            .Build();
        var listener = new HubListener(connection);
        // The hub speaks first once the connection has joined its groups.
        var joined = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        using var subscription = connection.On<JsonElement>("PresenceChanged", _ => joined.TrySetResult());
        await connection.StartAsync();
        await joined.Task.WaitAsync(TimeSpan.FromSeconds(10));
        return listener;
    }

    private sealed class HubListener : IAsyncDisposable
    {
        private readonly HubConnection _connection;
        private readonly System.Threading.Channels.Channel<JsonElement> _events =
            System.Threading.Channels.Channel.CreateUnbounded<JsonElement>();

        public HubListener(HubConnection connection)
        {
            _connection = connection;
            connection.On<JsonElement>("PermissionsChanged", payload => _events.Writer.TryWrite(payload));
        }

        public async Task<JsonElement> NextAsync() =>
            await _events.Reader.ReadAsync().AsTask().WaitAsync(TimeSpan.FromSeconds(10));

        public ValueTask DisposeAsync() => _connection.DisposeAsync();
    }

    /// <summary>
    /// Ada acts. She holds Lead, her top role, which manages roles, members
    /// and messages, and Helper below it, which alone lets her moderate. Jo
    /// holds Lower. Peer ranks with Lead and Higher above it; Gift, Banner and
    /// Hidden rank below Lower, Banner with a power Ada lacks and Hidden with
    /// that power granted only through a channel override. The base role
    /// ranks last.
    /// </summary>
    private sealed class Scenario : IDisposable
    {
        internal StellarrWebApplicationFactory Factory { get; } = new();
        internal HttpClient Actor { get; private set; } = null!;
        internal HttpClient Owner { get; private set; } = null!;
        internal Guid Higher { get; } = Guid.NewGuid();
        internal Guid Lead { get; } = Guid.NewGuid();
        internal Guid Peer { get; } = Guid.NewGuid();
        internal Guid Helper { get; } = Guid.NewGuid();
        internal Guid Lower { get; } = Guid.NewGuid();
        internal Guid Gift { get; } = Guid.NewGuid();
        internal Guid Banner { get; } = Guid.NewGuid();
        internal Guid Hidden { get; } = Guid.NewGuid();

        internal static string OverridesUrl => $"/api/admin/channels/{TestData.GeneralChannelId}/permissions";

        internal static async Task<Scenario> CreateAsync()
        {
            var state = new Scenario();
            await state.WithDbAsync(async db =>
            {
                db.ServerRoles.AddRange(
                    Role(state.Higher, "Higher", 5, ServerPrivileges.None),
                    Role(state.Lead, "Lead", 10, ServerPrivileges.ManageRoles | ServerPrivileges.ManageMembers | ServerPrivileges.ManageMessages),
                    Role(state.Peer, "Peer", 10, ServerPrivileges.None),
                    Role(state.Helper, "Helper", 15, ServerPrivileges.ModerateMembers),
                    Role(state.Lower, "Lower", 20, ServerPrivileges.None),
                    Role(state.Gift, "Gift", 21, ServerPrivileges.ManageMessages),
                    Role(state.Banner, "Banner", 22, ServerPrivileges.ManageBans),
                    Role(state.Hidden, "Hidden", 23, ServerPrivileges.None));
                db.PermissionOverrides.Add(new PermissionOverride
                {
                    Id = Guid.NewGuid(), ServerId = TestData.ServerId, RoleId = state.Hidden,
                    TargetType = PermissionTargetType.Channel, TargetId = TestData.ProjectsChannelId,
                    AllowMask = ServerPrivileges.ManageBans,
                });
                await db.SaveChangesAsync();
                await db.ServerRoles.Where(x => x.Id == MemberRoleId).ExecuteUpdateAsync(s => s.SetProperty(x => x.Position, 30));
            });
            await state.HoldAsync(AdaId, state.Lead, state.Helper);
            await state.HoldAsync(JoId, state.Lower);
            state.Actor = await SignedInAsync(state.Factory, "ada", "Quasar!2026");
            state.Owner = await SignedInAsync(state.Factory, "maja", "Aurora!2026");
            return state;
        }

        /// <summary>Stores what the member holds as the writers do: a row for each role and the highest as their top.</summary>
        internal Task HoldAsync(Guid userId, params Guid[] roleIds) => WithDbAsync(async db =>
        {
            await db.ServerMemberRoles.Where(x => x.ServerId == TestData.ServerId && x.UserId == userId).ExecuteDeleteAsync();
            db.ServerMemberRoles.AddRange(roleIds.Select(id => new ServerMemberRole
            {
                ServerId = TestData.ServerId, UserId = userId, RoleId = id,
            }));
            await db.SaveChangesAsync();
            var roles = await db.ServerRoles.AsNoTracking().Where(x => roleIds.Contains(x.Id)).ToListAsync();
            var baseRole = await db.ServerRoles.AsNoTracking().SingleAsync(x => x.Id == MemberRoleId);
            var top = HeldRoles.Top(roles, baseRole).Id;
            await db.ServerMembers.Where(x => x.ServerId == TestData.ServerId && x.UserId == userId)
                .ExecuteUpdateAsync(s => s.SetProperty(x => x.RoleId, top));
        });

        internal Task RoleOverrideAsync(Guid roleId, string[] allow, string[] deny) => WithDbAsync(async db =>
        {
            Assert.True(ServerAccess.TryParse(allow, out var allowMask));
            Assert.True(ServerAccess.TryParse(deny, out var denyMask));
            db.PermissionOverrides.Add(new PermissionOverride
            {
                Id = Guid.NewGuid(), ServerId = TestData.ServerId, RoleId = roleId,
                TargetType = PermissionTargetType.Channel, TargetId = TestData.GeneralChannelId,
                AllowMask = allowMask, DenyMask = denyMask,
            });
            await db.SaveChangesAsync();
        });

        internal static Task<HttpResponseMessage> SetRolesAsync(HttpClient client, Guid userId, params Guid[] roleIds) =>
            client.PutAsJsonAsync($"/api/admin/members/{userId}/roles", new { roleIds });

        internal static Task<HttpResponseMessage> SetMemberOverrideAsync(HttpClient client, Guid userId, string[] allow, string[] deny) =>
            client.PutAsJsonAsync($"{OverridesUrl}/members/{userId}", new { allow, deny });

        internal async Task<JsonElement> MemberAsync(Guid userId)
        {
            var members = JsonDocument.Parse(await Owner.GetStringAsync("/api/admin/members")).RootElement;
            return members.EnumerateArray().Single(x => x.GetProperty("user").GetProperty("id").GetString() == userId.ToString());
        }

        internal async Task<bool> JoMayAsync(ServerPrivileges privilege)
        {
            await using var scope = Factory.OpenScope(out var db);
            return await ServerAccess.HasAsync(db, JoId, TestData.ServerId, TestData.GeneralChannelId, privilege);
        }

        internal async Task WithDbAsync(Func<AppDbContext, Task> action)
        {
            await using var scope = Factory.OpenScope(out var db);
            await action(db);
        }

        private static async Task<HttpClient> SignedInAsync(StellarrWebApplicationFactory factory, string username, string password)
        {
            var client = factory.CreateClient(new WebApplicationFactoryClientOptions { AllowAutoRedirect = false, HandleCookies = true });
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
