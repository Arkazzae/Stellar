using Microsoft.AspNetCore.SignalR;
using Stellarr.Api.Auth;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
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
