using System.Net;
using System.Net.Http.Json;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>HTTP delegation boundaries, with one independent server state per case.</summary>
public sealed class ServerAuthorizationFlowTests
{
    private static readonly string[] ManageBans = ["manageBans"];
    private static readonly string[] ManageMessages = ["manageMessages"];
    [Theory]
    [InlineData("own", false, false)]
    [InlineData("equal", false, false)]
    [InlineData("higher", false, false)]
    [InlineData("lower", false, true)]
    [InlineData("own", true, false)]
    [InlineData("equal", true, false)]
    [InlineData("higher", true, false)]
    [InlineData("lower", true, true)]
    public async Task EditingAndDeletingRolesRequiresStrictlyLowerRank(string relationship, bool delete, bool allowed)
    {
        using var state = await Scenario.CreateAsync();
        var roleId = relationship == "own" ? state.ActorRole : state.LowerRole;
        if (relationship is "equal" or "higher")
        {
            await state.WithDbAsync(db => db.ServerRoles.Where(x => x.Id == roleId)
                .ExecuteUpdateAsync(s => s.SetProperty(x => x.Position, relationship == "equal" ? 10 : 5)));
        }
        using var response = delete
            ? await state.Client.DeleteAsync($"/api/admin/roles/{roleId}")
            : await state.Client.PatchAsJsonAsync($"/api/admin/roles/{roleId}", new { name = "Renamed" });
        Assert.Equal(allowed ? (delete ? HttpStatusCode.NoContent : HttpStatusCode.OK) : HttpStatusCode.Forbidden, response.StatusCode);
        await state.WithDbAsync(async db =>
        {
            var role = await db.ServerRoles.FindAsync(roleId);
            if (!allowed) Assert.NotNull(role);
            if (!delete) Assert.Equal(allowed ? "Renamed" : relationship == "own" ? "Actor" : "Lower", role!.Name);
        });
    }

    [Theory]
    [InlineData("replace-base")]
    [InlineData("rename-base")]
    [InlineData("delete-base")]
    [InlineData("rename-hidden")]
    [InlineData("delete-hidden")]
    public async Task ExistingRolePowersCannotBeRemovedWithoutHoldingThem(string operation)
    {
        using var state = await Scenario.CreateAsync();
        if (operation.EndsWith("hidden", StringComparison.Ordinal))
        {
            await state.OverrideAsync(state.LowerRole, false, ServerPrivileges.ManageBans, ServerPrivileges.None);
        }
        else
        {
            await state.WithDbAsync(db => db.ServerRoles.Where(x => x.Id == state.LowerRole)
                .ExecuteUpdateAsync(s => s.SetProperty(x => x.Permissions, ServerPrivileges.ManageBans)));
        }
        using var response = operation.StartsWith("delete", StringComparison.Ordinal)
            ? await state.Client.DeleteAsync($"/api/admin/roles/{state.LowerRole}")
            : operation == "replace-base"
                ? await state.Client.PatchAsJsonAsync($"/api/admin/roles/{state.LowerRole}", new { permissions = Array.Empty<string>() })
                : await state.Client.PatchAsJsonAsync($"/api/admin/roles/{state.LowerRole}", new { name = "Unauthorized rename" });
        Assert.Equal(HttpStatusCode.Forbidden, response.StatusCode);
    }

    [Fact]
    public async Task CreatingAndUpdatingRolesCanDelegateOnlyTheActorsPowers()
    {
        using var state = await Scenario.CreateAsync();
        using var deniedCreate = await state.Client.PostAsJsonAsync("/api/admin/roles", new
        {
            name = "Too powerful", color = "#123456", permissions = ManageBans,
        });
        Assert.Equal(HttpStatusCode.Forbidden, deniedCreate.StatusCode);
        using var allowedCreate = await state.Client.PostAsJsonAsync("/api/admin/roles", new
        {
            name = "Delegated", color = "#123456", permissions = ManageMessages,
        });
        Assert.Equal(HttpStatusCode.Created, allowedCreate.StatusCode);
        using var deniedEdit = await state.Client.PatchAsJsonAsync($"/api/admin/roles/{state.LowerRole}",
            new { permissions = ManageBans });
        Assert.Equal(HttpStatusCode.Forbidden, deniedEdit.StatusCode);
        using var allowedEdit = await state.Client.PatchAsJsonAsync($"/api/admin/roles/{state.LowerRole}",
            new { permissions = ManageMessages });
        Assert.Equal(HttpStatusCode.OK, allowedEdit.StatusCode);
    }

    [Theory]
    [InlineData(false, false)]
    [InlineData(false, true)]
    [InlineData(true, false)]
    [InlineData(true, true)]
    public async Task RemovingADenyThroughEmptyPutOrDeleteStillRequiresThatPower(bool section, bool delete)
    {
        using var state = await Scenario.CreateAsync();
        await state.OverrideAsync(state.LowerRole, section, ServerPrivileges.None, ServerPrivileges.ManageBans);
        using var response = delete
            ? await state.Client.DeleteAsync(state.OverrideUrl(section, state.LowerRole))
            : await state.Client.PutAsJsonAsync(state.OverrideUrl(section, state.LowerRole), new { allow = Array.Empty<string>(), deny = Array.Empty<string>() });
        Assert.Equal(HttpStatusCode.Forbidden, response.StatusCode);
        await state.WithDbAsync(async db => Assert.Equal(ServerPrivileges.ManageBans,
            (await db.PermissionOverrides.SingleAsync(x => x.RoleId == state.LowerRole)).DenyMask));
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task OverrideGrantAndDenyBothRespectTheServerRoleCeiling(bool section)
    {
        using var state = await Scenario.CreateAsync();
        foreach (var allow in new[] { true, false })
        {
            using var denied = await state.Client.PutAsJsonAsync(state.OverrideUrl(section, state.LowerRole), new
            {
                allow = allow ? ManageBans : [],
                deny = allow ? [] : ManageBans,
            });
            Assert.Equal(HttpStatusCode.Forbidden, denied.StatusCode);
        }
        using var allowed = await state.Client.PutAsJsonAsync(state.OverrideUrl(section, state.LowerRole),
            new { allow = ManageMessages, deny = Array.Empty<string>() });
        Assert.Equal(HttpStatusCode.OK, allowed.StatusCode);
        using var cleared = await state.Client.DeleteAsync(state.OverrideUrl(section, state.LowerRole));
        Assert.Equal(HttpStatusCode.NoContent, cleared.StatusCode);
    }

    [Theory]
    [InlineData("own")]
    [InlineData("equal")]
    [InlineData("higher")]
    [InlineData("lower")]
    public async Task OverrideChangesAlsoRespectRoleHierarchy(string relationship)
    {
        using var state = await Scenario.CreateAsync();
        var role = relationship == "own" ? state.ActorRole : state.LowerRole;
        if (relationship is "equal" or "higher")
        {
            await state.WithDbAsync(db => db.ServerRoles.Where(x => x.Id == role)
                .ExecuteUpdateAsync(s => s.SetProperty(x => x.Position, relationship == "equal" ? 10 : 5)));
        }
        using var response = await state.Client.PutAsJsonAsync(state.OverrideUrl(false, role),
            new { allow = ManageMessages, deny = Array.Empty<string>() });
        Assert.Equal(relationship == "lower" ? HttpStatusCode.OK : HttpStatusCode.Forbidden, response.StatusCode);
    }

    [Theory]
    [InlineData(false, false)]
    [InlineData(true, false)]
    [InlineData(true, true)]
    public async Task ScopedManageRolesDenyPreventsEditingOverrides(bool denyAtSection, bool editSection)
    {
        using var state = await Scenario.CreateAsync();
        await state.OverrideAsync(state.ActorRole, denyAtSection, ServerPrivileges.None, ServerPrivileges.ManageRoles);
        using var response = await state.Client.PutAsJsonAsync(state.OverrideUrl(editSection, state.LowerRole),
            new { allow = ManageMessages, deny = Array.Empty<string>() });
        Assert.Equal(HttpStatusCode.Forbidden, response.StatusCode);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task ALocalGrantCannotExpandDelegationAndALocalDenyRestrictsIt(bool grantBeyondBase)
    {
        using var state = await Scenario.CreateAsync();
        await state.OverrideAsync(state.ActorRole, false,
            grantBeyondBase ? ServerPrivileges.ManageBans : ServerPrivileges.None,
            grantBeyondBase ? ServerPrivileges.None : ServerPrivileges.ManageMessages);
        using var response = await state.Client.PutAsJsonAsync(state.OverrideUrl(false, state.LowerRole),
            new { allow = new[] { grantBeyondBase ? "manageBans" : "manageMessages" }, deny = Array.Empty<string>() });
        Assert.Equal(HttpStatusCode.Forbidden, response.StatusCode);
    }

    [Theory]
    [InlineData("self")]
    [InlineData("target-equal")]
    [InlineData("target-higher")]
    [InlineData("role-equal")]
    [InlineData("role-higher")]
    [InlineData("lower")]
    public async Task RoleAssignmentsRequireLowerTargetAndLowerDestination(string relationship)
    {
        using var state = await Scenario.CreateAsync();
        var target = relationship == "self" ? Scenario.ActorId : Scenario.TargetId;
        if (relationship.StartsWith("target-", StringComparison.Ordinal))
        {
            await state.WithDbAsync(db => db.ServerRoles.Where(x => x.Id == state.LowerRole)
                .ExecuteUpdateAsync(s => s.SetProperty(x => x.Position, relationship == "target-equal" ? 10 : 5)));
        }
        if (relationship.StartsWith("role-", StringComparison.Ordinal))
        {
            await state.WithDbAsync(db => db.ServerRoles.Where(x => x.Id == state.DestinationRole)
                .ExecuteUpdateAsync(s => s.SetProperty(x => x.Position, relationship == "role-equal" ? 10 : 5)));
        }
        using var response = await state.Client.PatchAsJsonAsync($"/api/admin/members/{target}/role", new { roleId = state.DestinationRole });
        Assert.Equal(relationship == "lower" ? HttpStatusCode.NoContent : HttpStatusCode.Forbidden, response.StatusCode);
        await state.WithDbAsync(async db => Assert.Equal(
            relationship == "lower" ? state.DestinationRole : relationship == "self" ? state.ActorRole : state.LowerRole,
            (await db.ServerMembers.SingleAsync(x => x.ServerId == TestData.ServerId && x.UserId == target)).RoleId));
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task RoleAssignmentChecksHiddenGrantsOnBothOldAndNewRoles(bool oldRole)
    {
        using var state = await Scenario.CreateAsync();
        await state.OverrideAsync(oldRole ? state.LowerRole : state.DestinationRole,
            true, ServerPrivileges.ManageBans, ServerPrivileges.None);
        using var response = await state.Client.PatchAsJsonAsync($"/api/admin/members/{Scenario.TargetId}/role",
            new { roleId = state.DestinationRole });
        Assert.Equal(HttpStatusCode.Forbidden, response.StatusCode);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task RoleAssignmentChecksBasePowersOnBothOldAndNewRoles(bool oldRole)
    {
        using var state = await Scenario.CreateAsync();
        var role = oldRole ? state.LowerRole : state.DestinationRole;
        await state.WithDbAsync(db => db.ServerRoles.Where(x => x.Id == role)
            .ExecuteUpdateAsync(s => s.SetProperty(x => x.Permissions, ServerPrivileges.ManageBans)));
        using var response = await state.Client.PatchAsJsonAsync($"/api/admin/members/{Scenario.TargetId}/role",
            new { roleId = state.DestinationRole });
        Assert.Equal(HttpStatusCode.Forbidden, response.StatusCode);
    }

    [Fact]
    public async Task DeletingARoleCannotGrantHiddenPowersThroughItsFallback()
    {
        using var state = await Scenario.CreateAsync();
        Guid fallback = default;
        await state.WithDbAsync(async db => fallback = await db.ServerRoles
            .Where(x => x.ServerId == TestData.ServerId && x.IsSystem && !x.IsOwner).Select(x => x.Id).SingleAsync());
        await state.OverrideAsync(fallback, false, ServerPrivileges.ManageBans, ServerPrivileges.None);
        using var response = await state.Client.DeleteAsync($"/api/admin/roles/{state.LowerRole}");
        Assert.Equal(HttpStatusCode.Forbidden, response.StatusCode);
        await state.WithDbAsync(async db => Assert.Equal(state.LowerRole,
            (await db.ServerMembers.SingleAsync(x => x.ServerId == TestData.ServerId && x.UserId == Scenario.TargetId)).RoleId));
    }

    [Theory]
    [InlineData(false, false)]
    [InlineData(false, true)]
    [InlineData(true, false)]
    [InlineData(true, true)]
    public async Task OwnerCanClearTheirOwnLegacyDeniesButOtherActorsCannot(bool section, bool delete)
    {
        using var state = await Scenario.CreateAsync();
        Guid ownerRole = default;
        await state.WithDbAsync(async db => ownerRole = await db.ServerRoles
            .Where(x => x.ServerId == TestData.ServerId && x.IsOwner).Select(x => x.Id).SingleAsync());
        await state.OverrideAsync(ownerRole, section, ServerPrivileges.None,
            ServerPrivileges.ManageRoles | ServerPrivileges.ManageMessages);

        using var forbidden = delete
            ? await state.Client.DeleteAsync(state.OverrideUrl(section, ownerRole))
            : await state.Client.PutAsJsonAsync(state.OverrideUrl(section, ownerRole), new { allow = Array.Empty<string>(), deny = Array.Empty<string>() });
        Assert.Equal(HttpStatusCode.Forbidden, forbidden.StatusCode);

        using var owner = state.Factory.CreateClient();
        (await owner.PostAsJsonAsync("/api/auth/login", new { username = "maja", password = "Aurora!2026" })).EnsureSuccessStatusCode();
        using var cleared = delete
            ? await owner.DeleteAsync(state.OverrideUrl(section, ownerRole))
            : await owner.PutAsJsonAsync(state.OverrideUrl(section, ownerRole), new { allow = Array.Empty<string>(), deny = Array.Empty<string>() });
        Assert.Equal(delete ? HttpStatusCode.NoContent : HttpStatusCode.OK, cleared.StatusCode);
        await state.WithDbAsync(async db => Assert.False(await db.PermissionOverrides.AnyAsync(x => x.RoleId == ownerRole)));
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task OverridesWhoseResourceWasDeletedDoNotBlockEditingTheRole(bool section)
    {
        using var state = await Scenario.CreateAsync();
        await state.OverrideAsync(state.LowerRole, section, ServerPrivileges.ManageBans, ServerPrivileges.None);
        await state.WithDbAsync(async db =>
        {
            if (section) await db.Sections.Where(x => x.Id == state.Section).ExecuteDeleteAsync();
            else await db.Channels.Where(x => x.Id == TestData.GeneralChannelId).ExecuteDeleteAsync();
            Assert.True(await db.PermissionOverrides.AnyAsync(x => x.RoleId == state.LowerRole));
        });
        using var response = await state.Client.PatchAsJsonAsync($"/api/admin/roles/{state.LowerRole}",
            new { name = "No obsolete delegation" });
        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
    }

    private sealed class Scenario : IDisposable
    {
        internal static readonly Guid ActorId = Guid.Parse("30000000-0000-0000-0000-000000000002");
        internal static readonly Guid TargetId = Guid.Parse("30000000-0000-0000-0000-000000000005");
        internal StellarrWebApplicationFactory Factory { get; } = new();
        internal HttpClient Client { get; private set; } = null!;
        internal Guid ActorRole { get; } = Guid.NewGuid();
        internal Guid LowerRole { get; } = Guid.NewGuid();
        internal Guid DestinationRole { get; } = Guid.NewGuid();
        internal Guid Section { get; } = Guid.NewGuid();

        internal static async Task<Scenario> CreateAsync()
        {
            var state = new Scenario();
            state.Client = state.Factory.CreateClient();
            await state.WithDbAsync(async db =>
            {
                db.ServerRoles.AddRange(
                    Role(state.ActorRole, "Actor", 10, ServerPrivileges.ManageRoles | ServerPrivileges.ManageMembers | ServerPrivileges.ManageMessages),
                    Role(state.LowerRole, "Lower", 20, ServerPrivileges.None),
                    Role(state.DestinationRole, "Destination", 21, ServerPrivileges.ManageMessages));
                db.Sections.Add(new Section { Id = state.Section, ServerId = TestData.ServerId, Name = "Authorization tests", Position = 50 });
                await db.SaveChangesAsync();
                await db.ServerRoles.Where(x => x.ServerId == TestData.ServerId && x.IsSystem && !x.IsOwner)
                    .ExecuteUpdateAsync(s => s.SetProperty(x => x.Position, 30));
                await db.Channels.Where(x => x.Id == TestData.GeneralChannelId)
                    .ExecuteUpdateAsync(s => s.SetProperty(x => x.SectionId, state.Section));
                await db.ServerMembers.Where(x => x.ServerId == TestData.ServerId && x.UserId == ActorId)
                    .ExecuteUpdateAsync(s => s.SetProperty(x => x.RoleId, state.ActorRole));
                await db.ServerMembers.Where(x => x.ServerId == TestData.ServerId && x.UserId == TargetId)
                    .ExecuteUpdateAsync(s => s.SetProperty(x => x.RoleId, state.LowerRole));
            });
            (await state.Client.PostAsJsonAsync("/api/auth/login", new { username = "ada", password = "Quasar!2026" })).EnsureSuccessStatusCode();
            return state;
        }

        internal async Task WithDbAsync(Func<AppDbContext, Task> action)
        {
            await using var scope = Factory.OpenScope(out var db);
            await action(db);
        }

        internal Task OverrideAsync(Guid role, bool section, ServerPrivileges allow, ServerPrivileges deny) =>
            WithDbAsync(async db =>
            {
                db.PermissionOverrides.Add(new PermissionOverride
                {
                    Id = Guid.NewGuid(), ServerId = TestData.ServerId, RoleId = role,
                    TargetType = section ? PermissionTargetType.Section : PermissionTargetType.Channel,
                    TargetId = section ? Section : TestData.GeneralChannelId, AllowMask = allow, DenyMask = deny,
                });
                await db.SaveChangesAsync();
            });

        internal string OverrideUrl(bool section, Guid role) =>
            $"/api/admin/{(section ? "sections" : "channels")}/{(section ? Section : TestData.GeneralChannelId)}/permissions/{role}";
        private static ServerRole Role(Guid id, string name, int position, ServerPrivileges permissions) => new()
        {
            Id = id, ServerId = TestData.ServerId, Name = name, Color = "#123456", Position = position, Permissions = permissions,
        };
        public void Dispose() { Client.Dispose(); Factory.Dispose(); }
    }
}
