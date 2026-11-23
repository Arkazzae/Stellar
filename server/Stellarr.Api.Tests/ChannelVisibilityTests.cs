using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// The questions every private-channel path asks, against the database: is
/// anybody denied view, and who sees what. Each test has a factory of its own,
/// since it changes who sees what.
/// </summary>
public sealed class ChannelVisibilityTests
{
    private static readonly Guid MemberRoleId = Guid.Parse("60000000-0000-0000-0000-000000000003");
    private static readonly Guid ModeratorRoleId = Guid.Parse("60000000-0000-0000-0000-000000000002");
    private static readonly Guid Maja = Guid.Parse("30000000-0000-0000-0000-000000000001");
    private static readonly Guid Ada = Guid.Parse("30000000-0000-0000-0000-000000000002");
    private static readonly Guid Ilya = Guid.Parse("30000000-0000-0000-0000-000000000003");
    private static readonly Guid Nils = Guid.Parse("30000000-0000-0000-0000-000000000004");
    private static readonly Guid Jo = Guid.Parse("30000000-0000-0000-0000-000000000005");

    [Fact]
    public async Task AServerWhereNobodyIsDeniedViewIsOpenEverywhere()
    {
        using var factory = new StellarrWebApplicationFactory();
        await using var scope = factory.OpenScope(out var db);

        Assert.True(await ChannelVisibility.ServerOpenAsync(db, TestData.ServerId, CancellationToken.None));
        Assert.All(await ChannelVisibility.Places(db, db.Channels).ToListAsync(), place => Assert.True(place.Open));
        Assert.Null(await ChannelVisibility.ViewersAsync(db, TestData.ServerId, TestData.ProjectsChannelId, CancellationToken.None));
        Assert.Null(await ChannelVisibility.OfMemberAsync(db, Jo, TestData.ServerId, CancellationToken.None));
        Assert.True((await ServerVisibility.LoadAsync(db, TestData.ServerId, CancellationToken.None)).Open);
    }

    [Fact]
    public async Task ADenyAtOneChannelClosesThatChannelAndTheServerButNoOtherChannel()
    {
        using var factory = new StellarrWebApplicationFactory();
        await using var scope = factory.OpenScope(out var db);
        await DenyAsync(db, PermissionTargetType.Channel, TestData.ProjectsChannelId, MemberRoleId);
        await AllowAsync(db, PermissionTargetType.Channel, TestData.ProjectsChannelId, ModeratorRoleId);

        var places = await ChannelVisibility.Places(db, db.Channels).ToDictionaryAsync(x => x.Id);
        Assert.False(places[TestData.ProjectsChannelId].Open);
        Assert.True(places[TestData.GeneralChannelId].Open);
        Assert.False(await ChannelVisibility.ServerOpenAsync(db, TestData.ServerId, CancellationToken.None));

        // The owner, and the moderator through the role that is allowed.
        var viewers = await ChannelVisibility.ViewersAsync(db, TestData.ServerId, TestData.ProjectsChannelId, CancellationToken.None);
        Assert.NotNull(viewers);
        Assert.True(viewers.SetEquals([Maja, Ada]));
        Assert.Null(await ChannelVisibility.FindAsync(
            db, Jo, TestData.ServerId, TestData.ProjectsChannelId, ChannelKind.Text, CancellationToken.None));
        Assert.NotNull(await ChannelVisibility.FindAsync(
            db, Ada, TestData.ServerId, TestData.ProjectsChannelId, ChannelKind.Text, CancellationToken.None));
        Assert.NotNull(await ChannelVisibility.FindAsync(
            db, Jo, TestData.ServerId, TestData.GeneralChannelId, ChannelKind.Text, CancellationToken.None));

        var view = await ChannelVisibility.OfMemberAsync(db, Jo, TestData.ServerId, CancellationToken.None);
        Assert.NotNull(view);
        Assert.DoesNotContain(TestData.ProjectsChannelId, view.ChannelIds);
        Assert.Contains(TestData.GeneralChannelId, view.ChannelIds);
    }

    [Fact]
    public async Task AnOverrideThatOutlivedItsChannelDeniesNothing()
    {
        using var factory = new StellarrWebApplicationFactory();
        await using var scope = factory.OpenScope(out var db);
        await DenyAsync(db, PermissionTargetType.Channel, Guid.NewGuid(), MemberRoleId);

        Assert.True(await ChannelVisibility.ServerOpenAsync(db, TestData.ServerId, CancellationToken.None));
    }

    [Fact]
    public async Task ABaseRoleWithoutViewClosesEverything()
    {
        using var factory = new StellarrWebApplicationFactory();
        await using var scope = factory.OpenScope(out var db);
        await db.ServerRoles.Where(x => x.Id == MemberRoleId)
            .ExecuteUpdateAsync(s => s.SetProperty(x => x.Permissions, ServerPrivileges.SendMessages));

        Assert.False(await ChannelVisibility.ServerOpenAsync(db, TestData.ServerId, CancellationToken.None));
        Assert.All(await ChannelVisibility.Places(db, db.Channels).ToListAsync(), place => Assert.False(place.Open));
        var visibility = await ServerVisibility.LoadAsync(db, TestData.ServerId, CancellationToken.None);
        Assert.Empty(visibility.Members[Ilya].ChannelIds);
        Assert.Empty(visibility.Members[Nils].ChannelIds);
        Assert.Equal(5, visibility.Members[Maja].ChannelIds.Count);
    }

    [Fact]
    public async Task AnEmptySectionIsListedWhereViewIsHeldAndAPrivateOneOnlyForItsManagers()
    {
        using var factory = new StellarrWebApplicationFactory();
        await using var scope = factory.OpenScope(out var db);
        var open = Guid.NewGuid();
        var staff = Guid.NewGuid();
        db.Sections.AddRange(
            new Section { Id = open, ServerId = TestData.ServerId, Name = "Otwarta", Position = 0 },
            new Section { Id = staff, ServerId = TestData.ServerId, Name = "Kadra", Position = 1 });
        await db.SaveChangesAsync();
        await DenyAsync(db, PermissionTargetType.Section, staff, MemberRoleId);

        var visibility = await ServerVisibility.LoadAsync(db, TestData.ServerId, CancellationToken.None);
        Assert.True(visibility.Members[Jo].SectionIds.SetEquals([open]));
        Assert.True(visibility.Members[Maja].SectionIds.SetEquals([open, staff]));
    }

    private static async Task DenyAsync(AppDbContext db, PermissionTargetType type, Guid target, Guid roleId)
    {
        db.PermissionOverrides.Add(new PermissionOverride
        {
            Id = Guid.NewGuid(),
            ServerId = TestData.ServerId,
            TargetType = type,
            TargetId = target,
            RoleId = roleId,
            DenyMask = ServerPrivileges.ViewChannel,
        });
        await db.SaveChangesAsync();
    }

    private static async Task AllowAsync(AppDbContext db, PermissionTargetType type, Guid target, Guid roleId)
    {
        db.PermissionOverrides.Add(new PermissionOverride
        {
            Id = Guid.NewGuid(),
            ServerId = TestData.ServerId,
            TargetType = type,
            TargetId = target,
            RoleId = roleId,
            AllowMask = ServerPrivileges.ViewChannel,
        });
        await db.SaveChangesAsync();
    }
}
