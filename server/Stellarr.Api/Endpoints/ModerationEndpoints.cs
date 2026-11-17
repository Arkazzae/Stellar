using Microsoft.AspNetCore.SignalR;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;
using Stellarr.Api.Realtime;

namespace Stellarr.Api.Endpoints;

/// <summary>
/// Removing people from a server: kicking, which they can undo by joining
/// again, and banning, which stays until a moderator lifts it.
/// </summary>
public static class ModerationEndpoints
{
    private const int MaximumReasonLength = 500;

    public static IEndpointRouteBuilder MapModerationEndpoints(this IEndpointRouteBuilder endpoints)
    {
        var moderation = endpoints.MapGroup("/api/admin")
            .RequireAuthorization()
            .AddEndpointFilter<ServerMembershipFilter>();
        moderation.MapGet("/bans", ListBansAsync);
        moderation.MapPost("/members/{userId:guid}/ban", BanAsync);
        moderation.MapDelete("/bans/{userId:guid}", UnbanAsync);
        moderation.MapDelete("/members/{userId:guid}", KickAsync);
        return endpoints;
    }

    private static async Task<IResult> ListBansAsync(
        HttpContext http,
        AppDbContext db,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        if (!await CanManageBansAsync(http, db, serverId, cancellationToken))
        {
            return Results.Forbid();
        }

        var bans = await db.ServerBans.AsNoTracking()
            .Where(x => x.ServerId == serverId)
            .Include(x => x.User)
            .Include(x => x.BannedBy)
            .OrderByDescending(x => x.BannedAt)
            .ToListAsync(cancellationToken);
        return Results.Ok(bans.Select(x => x.ToDto()));
    }

    private static async Task<IResult> BanAsync(
        Guid userId,
        BanMemberRequest request,
        HttpContext http,
        AppDbContext db,
        PresenceTracker presence,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var reason = request.Reason?.Trim() ?? "";
        if (reason.Length > MaximumReasonLength)
        {
            return Results.ValidationProblem(new Dictionary<string, string[]>
            {
                ["reason"] = [messages.Get(MessageKey.BanReasonTooLong, ("max", MaximumReasonLength))],
            });
        }

        var serverId = http.RequireActiveServerId();
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        var lookup = await ServerAuthorization.BanMemberAsync(
            db, http.User.RequireUserId(), serverId, userId, cancellationToken);
        if (!lookup.Allowed)
        {
            return Refusal(lookup);
        }

        var ban = new ServerBan
        {
            ServerId = serverId,
            UserId = userId,
            BannedById = lookup.Actor.UserId,
            Reason = reason.Length == 0 ? messages.Get(MessageKey.NoBanReason) : reason,
            BannedAt = DateTime.UtcNow,
            User = lookup.Target.User,
            BannedBy = lookup.Actor.User,
        };
        // Policy lookups are detached. Existing users must remain unchanged
        // when the new ban graph is attached.
        db.Attach(lookup.Actor.User);
        db.Attach(lookup.Target.User);
        db.ServerBans.Add(ban);
        db.ServerMembers.Remove(lookup.Target);
        if (!await TrySaveRemovalAsync(db, cancellationToken))
        {
            return Results.Conflict();
        }

        await transaction.CommitAsync(cancellationToken);
        await PublishRemovalAsync(serverId, userId, "banned", db, presence, hub);
        return Results.Created($"/api/admin/bans/{userId}", ban.ToDto());
    }

    private static async Task<IResult> UnbanAsync(
        Guid userId,
        HttpContext http,
        AppDbContext db,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        if (!await CanManageBansAsync(http, db, serverId, cancellationToken))
        {
            return Results.Forbid();
        }

        // Lifting a ban lets the person join again; it does not make them a member.
        var ban = await db.ServerBans.FirstOrDefaultAsync(
            x => x.ServerId == serverId && x.UserId == userId, cancellationToken);
        if (ban is null)
        {
            return Results.NotFound();
        }

        db.ServerBans.Remove(ban);
        await db.SaveChangesAsync(cancellationToken);
        return Results.NoContent();
    }

    private static async Task<IResult> KickAsync(
        Guid userId,
        HttpContext http,
        AppDbContext db,
        PresenceTracker presence,
        IHubContext<AppHub> hub,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        var lookup = await ServerAuthorization.KickMemberAsync(
            db, http.User.RequireUserId(), serverId, userId, cancellationToken);
        if (!lookup.Allowed)
        {
            return Refusal(lookup);
        }

        db.ServerMembers.Remove(lookup.Target);
        if (!await TrySaveRemovalAsync(db, cancellationToken))
        {
            return Results.Conflict();
        }

        await transaction.CommitAsync(cancellationToken);
        await PublishRemovalAsync(serverId, userId, "kicked", db, presence, hub);
        return Results.NoContent();
    }

    private static async Task<bool> CanManageBansAsync(
        HttpContext http,
        AppDbContext db,
        Guid serverId,
        CancellationToken cancellationToken) =>
        await ServerAccess.HasAsync(
            db, http.User.RequireUserId(), serverId, ServerPrivileges.ManageBans, cancellationToken);

    /// <summary>
    /// Saves the removal of a membership. False means somebody else got there
    /// first, whether by banning the same person or by removing them.
    /// </summary>
    private static async Task<bool> TrySaveRemovalAsync(AppDbContext db, CancellationToken cancellationToken)
    {
        try
        {
            return await db.TrySaveChangesAsync(cancellationToken);
        }
        catch (DbUpdateConcurrencyException)
        {
            db.ChangeTracker.Clear();
            return false;
        }
    }

    /// <summary>
    /// Cuts a removed member off from everything live: their voice seats, the
    /// server's and its text channels' groups. They hear about it through
    /// their own user group, which outlives the membership.
    /// </summary>
    private static async Task PublishRemovalAsync(
        Guid serverId,
        Guid userId,
        string kind,
        AppDbContext db,
        PresenceTracker presence,
        IHubContext<AppHub> hub)
    {
        var channels = await db.Channels.AsNoTracking()
            .Where(x => x.ServerId == serverId)
            .Select(x => new { x.Id, x.Kind })
            .ToListAsync();
        var voiceRoomIds = channels.Where(x => x.Kind == ChannelKind.Voice).Select(x => x.Id).ToHashSet();
        var departures = presence.RemoveUserFromVoiceChannels(userId, voiceRoomIds);

        await hub.Clients.Group(AppHub.ServerGroup(serverId)).SendAsync(
            "ServerMemberRemoved", new { serverId = serverId.ToString(), userId = userId.ToString() });
        await hub.Clients.Group(AppHub.UserGroup(userId)).SendAsync(
            "ServerAccessRevoked", new { serverId = serverId.ToString(), kind });

        // Only text channels have groups of their own; voice rooms are addressed by connection.
        var groups = channels.Where(x => x.Kind == ChannelKind.Text)
            .Select(x => AppHub.ChannelGroup(x.Id))
            .Prepend(AppHub.ServerGroup(serverId))
            .ToList();
        foreach (var connectionId in presence.ConnectionIds(userId))
        {
            foreach (var group in groups)
            {
                await hub.Groups.RemoveFromGroupAsync(connectionId, group);
            }
        }

        foreach (var roomId in departures.Select(x => x.ChannelId).Distinct())
        {
            await AppHub.BroadcastVoiceSnapshotAsync(hub.Clients, db, presence, roomId);
        }
    }

    internal static IResult Refusal(MemberAuthorization decision) =>
        decision.Failure == AuthorizationFailure.NotFound ? Results.NotFound() : Results.Forbid();
}
