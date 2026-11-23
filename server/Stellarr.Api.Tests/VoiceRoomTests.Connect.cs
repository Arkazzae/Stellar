using System.Net;
using System.Net.Http.Json;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// Connect, which admits a member to a voice channel. The seat is the
/// admission: a media session is only ever opened for one, so a connection
/// that is refused its seat gets no media either, whatever it calls next.
/// </summary>
public sealed partial class VoiceRoomTests
{
    private const string NotAllowed = "Not allowed to join this voice room.";
    private static readonly Guid MemberRoleId = Guid.Parse("60000000-0000-0000-0000-000000000003");

    [Theory]
    [InlineData("default", true)]
    [InlineData("base-role-loses-it", false)]
    [InlineData("held-role", true)]
    [InlineData("channel-allow", true)]
    [InlineData("channel-deny", false)]
    [InlineData("member-deny", false)]
    [InlineData("administrator-past-channel-deny", true)]
    public async Task ConnectIsDecidedInTheChannelBeforeASeatOrMediaIsGiven(string way, bool allowed)
    {
        var media = new FakeMediaServer();
        await using var app = new VoiceApp(media);
        await app.MediaReadyAsync();
        app.Seed(db =>
        {
            var jo = Accounts["jo"].Id;
            switch (way)
            {
                case "base-role-loses-it":
                    LoseConnect(db);
                    break;
                case "held-role":
                    LoseConnect(db);
                    Hold(db, jo, ServerPrivileges.Connect);
                    break;
                case "channel-allow":
                    LoseConnect(db);
                    db.PermissionOverrides.Add(SalonOverride(MemberRoleId, allow: ServerPrivileges.Connect));
                    break;
                case "channel-deny":
                    db.PermissionOverrides.Add(SalonOverride(MemberRoleId, deny: ServerPrivileges.Connect));
                    break;
                case "member-deny":
                    db.MemberPermissionOverrides.Add(new MemberPermissionOverride
                    {
                        Id = Guid.NewGuid(), ServerId = TestData.ServerId, UserId = jo,
                        TargetType = PermissionTargetType.Channel, TargetId = TestData.VoiceChannelId,
                        DenyMask = ServerPrivileges.Connect,
                    });
                    break;
                case "administrator-past-channel-deny":
                    db.PermissionOverrides.Add(SalonOverride(MemberRoleId, deny: ServerPrivileges.Connect));
                    Hold(db, jo, ServerPrivileges.Administrator);
                    break;
            }
        });
        var jo = await app.ConnectAsync("jo");

        if (allowed)
        {
            await jo.JoinAsync(Salon);
            await jo.OpenMediaAsync(Salon);
            Assert.Equal(1, media.Count("POST", "/v1/sessions"));
            return;
        }

        await RefusedAsync(NotAllowed, jo.JoinAsync(Salon));
        // Skipping the join does not help: media is opened for a seat only.
        await RefusedAsync("Not seated in this voice room.", jo.OpenMediaAsync(Salon));
        Assert.Empty(app.Presence.VoiceUsers(TestData.VoiceChannelId));
        Assert.Equal(0, media.Count("POST", "/v1/sessions"));
    }

    [Fact]
    public async Task AMemberWithoutConnectCannotMoveThemselvesThereFromAnotherRoom()
    {
        await using var app = new VoiceApp();
        app.Seed(db => db.PermissionOverrides.Add(new PermissionOverride
        {
            Id = Guid.NewGuid(), ServerId = TestData.ServerId, RoleId = MemberRoleId,
            TargetType = PermissionTargetType.Channel, TargetId = TestData.NightShiftChannelId,
            DenyMask = ServerPrivileges.Connect,
        }));
        var jo = await app.ConnectAsync("jo");

        await jo.JoinAsync(Salon);
        await RefusedAsync(NotAllowed, jo.JoinAsync(NightShift));

        // The refused move leaves the seat where it was.
        Assert.Equal([Accounts["jo"].Id], app.Presence.VoiceUsers(TestData.VoiceChannelId).Select(x => x.UserId));
        Assert.Empty(app.Presence.VoiceUsers(TestData.NightShiftChannelId));
    }

    [Fact]
    public async Task AModeratorMayMoveSomebodyIntoAChannelTheyCouldNotJoinAndTheyGetMediaThere()
    {
        var media = new FakeMediaServer();
        await using var app = new VoiceApp(media);
        await app.MediaReadyAsync();
        app.Seed(db => db.PermissionOverrides.Add(new PermissionOverride
        {
            Id = Guid.NewGuid(), ServerId = TestData.ServerId, RoleId = MemberRoleId,
            TargetType = PermissionTargetType.Channel, TargetId = TestData.NightShiftChannelId,
            DenyMask = ServerPrivileges.Connect,
        }));
        var jo = await app.ConnectAsync("jo");
        using var moderator = await app.HttpAsync("ada");
        await jo.JoinAsync(Salon);

        using var moved = await moderator.PatchAsJsonAsync(
            $"/api/admin/channels/{TestData.VoiceChannelId}/members/{jo.Id}",
            new { targetChannelId = TestData.NightShiftChannelId });
        Assert.Equal(HttpStatusCode.NoContent, moved.StatusCode);
        await jo["VoiceChannelMoved"].NextAsync();

        // The client confirms the move by joining the room it was moved to, and opens media there.
        await jo.JoinAsync(NightShift);
        await jo.OpenMediaAsync(NightShift);
        Assert.Equal([jo.Id], app.Presence.VoiceUsers(TestData.NightShiftChannelId).Select(x => x.UserId));
        Assert.Contains(media.Requests, x => x.Method == "POST" && x.Path == "/v1/sessions" &&
            x.Json.GetProperty("roomId").GetString() == NightShift);

        // Once out of the room, coming back is a join of their own again.
        await jo.LeaveAsync();
        await RefusedAsync(NotAllowed, jo.JoinAsync(NightShift));
    }

    [Fact]
    public async Task ConnectIsAboutServerChannelsAndLeavesDirectCallsAlone()
    {
        await using var app = new VoiceApp();
        app.Seed(db =>
        {
            LoseConnect(db);
            db.DirectConversations.Add(Conversation("jo", "nils"));
        });
        var jo = await app.ConnectAsync("jo");

        await jo.JoinAsync(MajaAndAda.ToString());
        await RefusedAsync(NotAllowed, jo.JoinAsync(Salon));
    }

    private static void LoseConnect(Stellarr.Api.Data.AppDbContext db)
    {
        var baseRole = db.ServerRoles.Single(x => x.Id == MemberRoleId);
        baseRole.Permissions &= ~ServerPrivileges.Connect;
    }

    /// <summary>Gives the member a role of their own, ranked just above the base role.</summary>
    private static void Hold(Stellarr.Api.Data.AppDbContext db, Guid userId, ServerPrivileges permissions)
    {
        var role = new ServerRole
        {
            Id = Guid.NewGuid(), ServerId = TestData.ServerId, Name = "Głos", Color = "#123456",
            Position = 2, Permissions = permissions,
        };
        db.ServerRoles.Add(role);
        db.ServerRoles.Single(x => x.Id == MemberRoleId).Position = 3;
        db.ServerMemberRoles.Add(new ServerMemberRole { ServerId = TestData.ServerId, UserId = userId, RoleId = role.Id });
        db.ServerMembers.Single(x => x.ServerId == TestData.ServerId && x.UserId == userId).RoleId = role.Id;
    }

    private static PermissionOverride SalonOverride(
        Guid roleId, ServerPrivileges allow = ServerPrivileges.None, ServerPrivileges deny = ServerPrivileges.None) => new()
    {
        Id = Guid.NewGuid(), ServerId = TestData.ServerId, RoleId = roleId,
        TargetType = PermissionTargetType.Channel, TargetId = TestData.VoiceChannelId,
        AllowMask = allow, DenyMask = deny,
    };
}
