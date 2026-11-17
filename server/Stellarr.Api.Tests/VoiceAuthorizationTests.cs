using System.Net;
using System.Net.Http.Json;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Domain;
using Stellarr.Api.Realtime;

namespace Stellarr.Api.Tests;

public sealed class VoiceAuthorizationTests
{
    private static readonly Guid ModeratorRoleId = Guid.Parse("60000000-0000-0000-0000-000000000002");
    private static readonly Guid TargetId = Guid.Parse("30000000-0000-0000-0000-000000000005");
    private const string ConnectionId = "voice-permission-target";

    [Theory]
    [InlineData(false, false)]
    [InlineData(false, true)]
    [InlineData(true, false)]
    [InlineData(true, true)]
    public async Task MoveCannotBypassSourceOrDestinationDeny(bool denyDestination, bool sectionDeny)
    {
        using var factory = new StellarrWebApplicationFactory();
        using var actor = factory.CreateClient();
        await using (var scope = factory.OpenScope(out var db))
        {
            var channelId = denyDestination ? TestData.NightShiftChannelId : TestData.VoiceChannelId;
            var targetId = channelId;
            if (sectionDeny)
            {
                targetId = Guid.NewGuid();
                db.Sections.Add(new Section { Id = targetId, ServerId = TestData.ServerId, Name = "Restricted" });
                (await db.Channels.SingleAsync(x => x.Id == channelId)).SectionId = targetId;
            }
            db.PermissionOverrides.Add(new PermissionOverride
            {
                Id = Guid.NewGuid(), ServerId = TestData.ServerId, RoleId = ModeratorRoleId,
                TargetId = targetId,
                TargetType = sectionDeny ? PermissionTargetType.Section : PermissionTargetType.Channel,
                DenyMask = ServerPrivileges.ModerateMembers,
            });
            await db.SaveChangesAsync();
        }
        await LoginAsync(actor);
        var presence = SeatTarget(factory);
        var moved = await MoveAsync(actor);
        Assert.Equal(HttpStatusCode.Forbidden, moved.StatusCode);
        Assert.True(presence.IsConnectionInVoice(ConnectionId, TestData.VoiceChannelId));
        var kicked = await actor.DeleteAsync($"/api/admin/channels/{TestData.VoiceChannelId}/members/{TargetId}");
        Assert.Equal(denyDestination ? HttpStatusCode.NoContent : HttpStatusCode.Forbidden, kicked.StatusCode);
    }

    [Fact]
    public async Task LocalModerationGrantsOnBothRoomsAuthorizeMoveAndTheirRemovalRevokesIt()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var actor = factory.CreateClient();
        await using (var scope = factory.OpenScope(out var db))
        {
            (await db.ServerRoles.SingleAsync(x => x.Id == ModeratorRoleId)).Permissions = ServerPrivileges.None;
            foreach (var channelId in new[] { TestData.VoiceChannelId, TestData.NightShiftChannelId })
                db.PermissionOverrides.Add(new PermissionOverride
                {
                    Id = Guid.NewGuid(), ServerId = TestData.ServerId, RoleId = ModeratorRoleId,
                    TargetId = channelId, TargetType = PermissionTargetType.Channel,
                    AllowMask = ServerPrivileges.ModerateMembers,
                });
            await db.SaveChangesAsync();
        }
        await LoginAsync(actor);
        var presence = SeatTarget(factory);
        Assert.Equal(HttpStatusCode.NoContent, (await MoveAsync(actor)).StatusCode);
        Assert.True(presence.IsConnectionInVoice(ConnectionId, TestData.NightShiftChannelId));
        await using (var scope = factory.OpenScope(out var db))
            await db.PermissionOverrides.Where(x => x.RoleId == ModeratorRoleId).ExecuteDeleteAsync();
        var revoked = await actor.DeleteAsync($"/api/admin/channels/{TestData.NightShiftChannelId}/members/{TargetId}");
        Assert.Equal(HttpStatusCode.Forbidden, revoked.StatusCode);
        Assert.True(presence.IsConnectionInVoice(ConnectionId, TestData.NightShiftChannelId));
    }

    private static PresenceTracker SeatTarget(StellarrWebApplicationFactory factory)
    {
        var presence = factory.Services.GetRequiredService<PresenceTracker>();
        presence.Connect(ConnectionId, TargetId);
        Assert.True(presence.JoinVoice(ConnectionId, TargetId, TestData.VoiceChannelId).Accepted);
        return presence;
    }

    private static Task<HttpResponseMessage> MoveAsync(HttpClient actor) =>
        actor.PatchAsJsonAsync($"/api/admin/channels/{TestData.VoiceChannelId}/members/{TargetId}",
            new { targetChannelId = TestData.NightShiftChannelId });

    private static async Task LoginAsync(HttpClient actor) =>
        (await actor.PostAsJsonAsync("/api/auth/login", new { username = "ada", password = "Quasar!2026" }))
            .EnsureSuccessStatusCode();
}
