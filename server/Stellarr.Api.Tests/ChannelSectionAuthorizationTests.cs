using System.Net;
using System.Net.Http.Json;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>Moving a channel may reorganize it, but must not bypass delegation rules.</summary>
public sealed class ChannelSectionAuthorizationTests
{
    private const ServerPrivileges ChannelManager = ServerPrivileges.ManageChannels;
    private const ServerPrivileges Delegator = ChannelManager | ServerPrivileges.ManageRoles | ServerPrivileges.ManageMessages;

    [Theory]
    [InlineData("section")]
    [InlineData("none")]
    [InlineData("same")]
    public async Task OrganizationWithoutEffectivePermissionChangesDoesNotRequireManageRoles(string destination)
    {
        using var state = await MoveScenario.CreateAsync(ChannelManager);
        var target = destination == "none" ? (Guid?)null : destination == "same" ? state.Source : state.Destination;
        await state.ExpectMoveAsync(target, HttpStatusCode.NoContent);
    }

    [Theory]
    [InlineData("own")]
    [InlineData("equal")]
    [InlineData("higher")]
    [InlineData("lower")]
    public async Task ReparentingCannotChangeOwnPeerOrHigherRolePermissions(string relationship)
    {
        using var state = await MoveScenario.CreateAsync(Delegator);
        var role = relationship == "own" ? state.ActorRole : state.LowerRole;
        if (relationship is "equal" or "higher")
        {
            await state.WithDbAsync(db => db.ServerRoles.Where(x => x.Id == role)
                .ExecuteUpdateAsync(s => s.SetProperty(x => x.Position, relationship == "equal" ? 10 : 5)));
        }
        await state.OverrideAsync(role, state.Destination, PermissionTargetType.Section,
            deny: ServerPrivileges.ManageMessages);
        await state.ExpectMoveAsync(state.Destination,
            relationship == "lower" ? HttpStatusCode.NoContent : HttpStatusCode.Forbidden);
    }

    [Theory]
    [InlineData(false, false)]
    [InlineData(false, true)]
    [InlineData(true, false)]
    [InlineData(true, true)]
    public async Task ChangingLowerRolePermissionsNeedsManageRolesIncludingUngrouping(bool managesRoles, bool ungroup)
    {
        using var state = await MoveScenario.CreateAsync(managesRoles ? Delegator : ChannelManager | ServerPrivileges.ManageMessages);
        await state.OverrideAsync(state.LowerRole, ungroup ? state.Source : state.Destination,
            PermissionTargetType.Section, deny: ServerPrivileges.ManageMessages);
        await state.ExpectMoveAsync(ungroup ? null : state.Destination,
            managesRoles ? HttpStatusCode.NoContent : HttpStatusCode.Forbidden);
    }

    [Theory]
    [InlineData("source")]
    [InlineData("destination")]
    [InlineData("both")]
    public async Task TheActorMustHoldChangedBitsInBothContexts(string deniedContext)
    {
        using var state = await MoveScenario.CreateAsync(Delegator);
        await state.OverrideAsync(state.LowerRole, state.Destination, PermissionTargetType.Section,
            deny: ServerPrivileges.ManageMessages);
        if (deniedContext is "source" or "both")
            await state.OverrideAsync(state.ActorRole, state.Source, PermissionTargetType.Section, deny: ServerPrivileges.ManageMessages);
        if (deniedContext is "destination" or "both")
            await state.OverrideAsync(state.ActorRole, state.Destination, PermissionTargetType.Section, deny: ServerPrivileges.ManageMessages);
        await state.ExpectMoveAsync(state.Destination, HttpStatusCode.Forbidden);
    }

    [Fact]
    public async Task LocalGrantsOnBothSectionsCannotExpandTheActorsServerDelegationCeiling()
    {
        using var state = await MoveScenario.CreateAsync(Delegator);
        await state.OverrideAsync(state.ActorRole, state.Source, PermissionTargetType.Section, allow: ServerPrivileges.ManageBans);
        await state.OverrideAsync(state.ActorRole, state.Destination, PermissionTargetType.Section, allow: ServerPrivileges.ManageBans);
        await state.OverrideAsync(state.LowerRole, state.Destination, PermissionTargetType.Section, allow: ServerPrivileges.ManageBans);
        await state.ExpectMoveAsync(state.Destination, HttpStatusCode.Forbidden);
    }

    [Theory]
    [InlineData("source-channel")]
    [InlineData("source-section")]
    [InlineData("destination-section")]
    [InlineData("destination-section-with-channel-grant")]
    [InlineData("server-after-ungroup")]
    public async Task MovingRequiresManageChannelsAtTheSourceAndDestination(string missing)
    {
        using var state = await MoveScenario.CreateAsync(missing == "server-after-ungroup" ? ServerPrivileges.None : Delegator);
        switch (missing)
        {
            case "source-channel":
                await state.OverrideAsync(state.ActorRole, TestData.GeneralChannelId, PermissionTargetType.Channel, deny: ChannelManager);
                break;
            case "source-section":
                await state.OverrideAsync(state.ActorRole, state.Source, PermissionTargetType.Section, deny: ChannelManager);
                break;
            case "destination-section":
            case "destination-section-with-channel-grant":
                await state.OverrideAsync(state.ActorRole, state.Destination, PermissionTargetType.Section, deny: ChannelManager);
                if (missing.EndsWith("channel-grant", StringComparison.Ordinal))
                    await state.OverrideAsync(state.ActorRole, TestData.GeneralChannelId, PermissionTargetType.Channel, allow: ChannelManager);
                break;
            case "server-after-ungroup":
                await state.OverrideAsync(state.ActorRole, state.Source, PermissionTargetType.Section, allow: ChannelManager);
                break;
        }
        await state.ExpectMoveAsync(missing == "server-after-ungroup" ? null : state.Destination, HttpStatusCode.Forbidden);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task ChannelOverridesThatPreserveEffectiveRightsAllowOrdinaryOrganization(bool allow)
    {
        using var state = await MoveScenario.CreateAsync(ChannelManager);
        var grant = allow ? ServerPrivileges.ManageMessages : ServerPrivileges.None;
        var deny = allow ? ServerPrivileges.None : ServerPrivileges.ManageMessages;
        // The sections differ, but the channel override decides both before and after.
        await state.OverrideAsync(state.LowerRole, state.Source, PermissionTargetType.Section,
            deny: ServerPrivileges.ManageMessages);
        await state.OverrideAsync(state.LowerRole, TestData.GeneralChannelId, PermissionTargetType.Channel, grant, deny);
        await state.ExpectMoveAsync(state.Destination, HttpStatusCode.NoContent);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task OwnerCanMoveChannelsWhenLowerRolesGainOrLoseInheritedPermissions(bool ungroup)
    {
        using var state = await MoveScenario.CreateAsync(Delegator);
        await state.OverrideAsync(state.LowerRole, ungroup ? state.Source : state.Destination,
            PermissionTargetType.Section, deny: ServerPrivileges.ManageMessages);
        using var owner = state.Factory.CreateClient();
        (await owner.PostAsJsonAsync("/api/auth/login", new { username = "maja", password = "Aurora!2026" })).EnsureSuccessStatusCode();
        await state.ExpectMoveAsync(ungroup ? null : state.Destination, HttpStatusCode.NoContent, owner);
    }

    [Fact]
    public async Task MissingDestinationIsRejectedWithoutMovingTheChannel()
    {
        using var state = await MoveScenario.CreateAsync(Delegator);
        await state.ExpectMoveAsync(Guid.NewGuid(), HttpStatusCode.NotFound);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task AMoveSentWithTheChannelOrderObeysTheSameDelegationRules(bool managesRoles)
    {
        using var state = await MoveScenario.CreateAsync(managesRoles ? Delegator : ChannelManager | ServerPrivileges.ManageMessages);
        await state.OverrideAsync(state.LowerRole, state.Destination, PermissionTargetType.Section,
            deny: ServerPrivileges.ManageMessages);
        await state.ExpectOrderedMoveAsync(state.Destination,
            managesRoles ? HttpStatusCode.NoContent : HttpStatusCode.Forbidden);
    }

    private sealed class MoveScenario : IDisposable
    {
        internal StellarrWebApplicationFactory Factory { get; } = new();
        internal HttpClient Client { get; private set; } = null!;
        internal Guid ActorRole { get; } = Guid.NewGuid();
        internal Guid LowerRole { get; } = Guid.NewGuid();
        internal Guid Source { get; } = Guid.NewGuid();
        internal Guid Destination { get; } = Guid.NewGuid();

        internal static async Task<MoveScenario> CreateAsync(ServerPrivileges actorPermissions)
        {
            var state = new MoveScenario();
            state.Client = state.Factory.CreateClient();
            await state.WithDbAsync(async db =>
            {
                db.ServerRoles.AddRange(
                    new ServerRole { Id = state.ActorRole, ServerId = TestData.ServerId, Name = "Channel manager", Color = "#123456", Position = 10, Permissions = actorPermissions },
                    new ServerRole { Id = state.LowerRole, ServerId = TestData.ServerId, Name = "Lower role", Color = "#123456", Position = 20, Permissions = ServerPrivileges.ManageMessages });
                db.Sections.AddRange(
                    new Section { Id = state.Source, ServerId = TestData.ServerId, Name = "Source", Position = 40 },
                    new Section { Id = state.Destination, ServerId = TestData.ServerId, Name = "Destination", Position = 41 });
                await db.SaveChangesAsync();
                var actorId = await db.Users.Where(x => x.Username == "ada").Select(x => x.Id).SingleAsync();
                await db.ServerMembers.Where(x => x.ServerId == TestData.ServerId && x.UserId == actorId)
                    .ExecuteUpdateAsync(s => s.SetProperty(x => x.RoleId, state.ActorRole));
                await db.Channels.Where(x => x.Id == TestData.GeneralChannelId)
                    .ExecuteUpdateAsync(s => s.SetProperty(x => x.SectionId, state.Source));
            });
            (await state.Client.PostAsJsonAsync("/api/auth/login", new { username = "ada", password = "Quasar!2026" })).EnsureSuccessStatusCode();
            return state;
        }

        internal async Task ExpectMoveAsync(Guid? destination, HttpStatusCode expected, HttpClient? client = null)
        {
            using var response = await ChannelSections.FileAsync(client ?? Client, TestData.GeneralChannelId, destination);
            Assert.Equal(expected, response.StatusCode);
            await WithDbAsync(async db => Assert.Equal(expected == HttpStatusCode.NoContent ? destination : Source,
                (await db.Channels.SingleAsync(x => x.Id == TestData.GeneralChannelId)).SectionId));
        }

        /// <summary>The same move sent with the whole channel order, reversed; a refusal keeps both.</summary>
        internal async Task ExpectOrderedMoveAsync(Guid? destination, HttpStatusCode expected)
        {
            var before = await OrderAsync();
            var reversed = before.AsEnumerable().Reverse().ToArray();
            using var response = await Client.PutAsJsonAsync("/api/admin/channels/order", new
            {
                channelIds = reversed,
                move = new { channelId = TestData.GeneralChannelId, sectionId = destination },
            });
            Assert.Equal(expected, response.StatusCode);
            var applied = expected == HttpStatusCode.NoContent;
            Assert.Equal(applied ? reversed : before, await OrderAsync());
            await WithDbAsync(async db => Assert.Equal(applied ? destination : Source,
                (await db.Channels.SingleAsync(x => x.Id == TestData.GeneralChannelId)).SectionId));
        }

        private async Task<Guid[]> OrderAsync()
        {
            Guid[] order = [];
            await WithDbAsync(async db => order = await db.Channels.Where(x => x.ServerId == TestData.ServerId)
                .OrderBy(x => x.Position).Select(x => x.Id).ToArrayAsync());
            return order;
        }

        internal async Task WithDbAsync(Func<AppDbContext, Task> action)
        {
            await using var scope = Factory.OpenScope(out var db);
            await action(db);
        }

        internal Task OverrideAsync(Guid role, Guid target, PermissionTargetType type,
            ServerPrivileges allow = ServerPrivileges.None, ServerPrivileges deny = ServerPrivileges.None) =>
            WithDbAsync(async db =>
            {
                db.PermissionOverrides.Add(new PermissionOverride
                {
                    Id = Guid.NewGuid(), ServerId = TestData.ServerId, RoleId = role,
                    TargetId = target, TargetType = type, AllowMask = allow, DenyMask = deny,
                });
                await db.SaveChangesAsync();
            });

        public void Dispose() { Client.Dispose(); Factory.Dispose(); }
    }
}
