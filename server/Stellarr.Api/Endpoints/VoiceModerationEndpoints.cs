using Microsoft.AspNetCore.SignalR;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Localization;
using Stellarr.Api.Realtime;

namespace Stellarr.Api.Endpoints;

/// <summary>
/// Moderating a live voice session without touching the membership: sending a
/// member out of a voice channel, or from one voice channel to another.
/// </summary>
public static class VoiceModerationEndpoints
{
    public static IEndpointRouteBuilder MapVoiceModerationEndpoints(this IEndpointRouteBuilder endpoints)
    {
        var moderation = endpoints.MapGroup("/api/admin")
            .RequireAuthorization()
            .AddEndpointFilter<ServerMembershipFilter>();
        moderation.MapDelete("/channels/{channelId:guid}/members/{userId:guid}", KickFromVoiceAsync);
        moderation.MapPatch("/channels/{channelId:guid}/members/{userId:guid}", MoveAsync);
        return endpoints;
    }

    private static async Task<IResult> KickFromVoiceAsync(
        Guid channelId,
        Guid userId,
        HttpContext http,
        AppDbContext db,
        PresenceTracker presence,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        var lookup = await ServerAuthorization.KickVoiceMemberAsync(
            db, http.User.RequireUserId(), serverId, channelId, userId, cancellationToken);
        if (!lookup.Allowed)
        {
            return ModerationEndpoints.Refusal(lookup);
        }

        var seated = presence.VoiceConnectionIds(userId, channelId);
        presence.RemoveUserFromVoiceChannels(userId, new HashSet<Guid> { channelId });
        if (seated.Count > 0)
        {
            var channelName = await db.Channels.Where(x => x.Id == channelId).Select(x => x.Name).SingleAsync(cancellationToken);
            await ServerAudit.StageAsync(db, serverId, lookup.Actor.UserId, new AuditEvent(
                "voice.disconnect",
                messages.Get(MessageKey.AuditVoiceDisconnected, ("name", lookup.Target.User.DisplayName), ("channel", channelName)),
                "member", userId, lookup.Target.User.DisplayName,
                [new AuditChange("channel", From: channelName)]), cancellationToken);
            await db.SaveChangesAsync(cancellationToken);
        }

        await transaction.CommitAsync(cancellationToken);
        if (seated.Count > 0)
        {
            await hub.Clients.Clients(seated).SendAsync(
                "VoiceChannelKicked", new { channelId = channelId.ToString() }, CancellationToken.None);
            await AppHub.BroadcastVoiceSnapshotAsync(hub.Clients, db, presence, channelId, CancellationToken.None);
        }

        // Kicking somebody who has already gone is the end state that was asked for.
        return Results.NoContent();
    }

    private static async Task<IResult> MoveAsync(
        Guid channelId,
        Guid userId,
        MoveVoiceMemberRequest request,
        HttpContext http,
        AppDbContext db,
        PresenceTracker presence,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        var targetChannelId = request.TargetChannelId;
        var lookup = await ServerAuthorization.MoveVoiceMemberAsync(
            db, http.User.RequireUserId(), serverId, channelId, targetChannelId, userId, cancellationToken);
        if (!lookup.Allowed)
        {
            return ModerationEndpoints.Refusal(lookup);
        }
        Guid[] rooms = [.. new[] { channelId, targetChannelId }.Distinct()];

        if (channelId == targetChannelId)
        {
            return Results.NoContent();
        }

        // The connections keep their seats and their mute state; the client is
        // told to rebuild its peers for the new room.
        var move = presence.MoveUserBetweenVoiceChannels(userId, channelId, targetChannelId);
        if (!move.Moved)
        {
            return Results.Conflict();
        }

        var names = await db.Channels.Where(x => x.Id == channelId || x.Id == targetChannelId)
            .ToDictionaryAsync(x => x.Id, x => x.Name, cancellationToken);
        await ServerAudit.StageAsync(db, serverId, lookup.Actor.UserId, new AuditEvent(
            "voice.move",
            messages.Get(MessageKey.AuditVoiceMoved,
                ("name", lookup.Target.User.DisplayName), ("from", names[channelId]), ("to", names[targetChannelId])),
            "member", userId, lookup.Target.User.DisplayName,
            [new AuditChange("channel", names[channelId], names[targetChannelId])]), cancellationToken);
        await db.SaveChangesAsync(cancellationToken);
        await transaction.CommitAsync(cancellationToken);
        await hub.Clients.Clients(move.ConnectionIds).SendAsync(
            "VoiceChannelMoved",
            new { fromChannelId = channelId.ToString(), toChannelId = targetChannelId.ToString() },
            CancellationToken.None);
        foreach (var room in rooms)
        {
            await AppHub.BroadcastVoiceSnapshotAsync(hub.Clients, db, presence, room, CancellationToken.None);
        }

        return Results.NoContent();
    }

}
