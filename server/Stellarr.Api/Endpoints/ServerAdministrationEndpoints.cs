using System.Security.Cryptography;
using System.Text.Json;
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
/// What the people who run a server do beyond its profile: invite people,
/// read the audit log, set the notification defaults, and the two things only
/// the owner may do, handing the server on and deleting it.
/// </summary>
public static class ServerAdministrationEndpoints
{
    private const int InviteCodeBytes = 8;
    private const int InviteMaxUses = 25;
    private const int InviteLifetimeDays = 7;
    private const int AuditPageSize = 50;

    /// <summary>The caller, and the server the request is about.</summary>
    private readonly record struct Actor(Guid ServerId, Guid UserId);

    public static IEndpointRouteBuilder MapServerAdministrationEndpoints(this IEndpointRouteBuilder endpoints)
    {
        var admin = endpoints.MapGroup("/api/admin")
            .RequireAuthorization()
            .AddEndpointFilter<ServerMembershipFilter>();
        admin.MapGet("/invites", ListInvitesAsync);
        admin.MapPost("/invites", CreateInviteAsync);
        admin.MapDelete("/invites/{inviteId:guid}", RevokeInviteAsync);
        admin.MapPost("/invites/{inviteId:guid}/members/{userId:guid}", SendInviteAsync);
        admin.MapGet("/audit", ListAuditAsync);
        admin.MapGet("/notifications", GetNotificationsAsync);
        admin.MapPatch("/notifications", UpdateNotificationsAsync);
        // Handing a server over or deleting it cannot be undone; on the instance
        // server a hand-over makes someone else the instance owner, and the
        // instance server cannot be deleted at all.
        admin.MapPost("/server/transfer", TransferAsync).AddEndpointFilter<FreshSecondStepFilter>();
        admin.MapDelete("/server", DeleteServerAsync).AddEndpointFilter<FreshSecondStepFilter>();
        return endpoints;
    }

    private static async Task<IResult> ListInvitesAsync(
        HttpContext http,
        AppDbContext db,
        CancellationToken cancellationToken)
    {
        if (await AuthorizeAsync(http, db, ServerPrivileges.ManageServer, cancellationToken) is not { } actor)
        {
            return Results.Forbid();
        }

        var now = DateTime.UtcNow;
        var invites = await db.ServerInvites.AsNoTracking()
            .Include(x => x.CreatedBy)
            .Include(x => x.Channel)
            .Where(x => x.ServerId == actor.ServerId &&
                (x.ExpiresAt == null || x.ExpiresAt > now) &&
                (x.MaxUses == null || x.Uses < x.MaxUses))
            .OrderByDescending(x => x.CreatedAt)
            .ToListAsync(cancellationToken);
        var origin = InviteOrigin(http);
        return Results.Ok(invites.Select(x => x.ToDto(origin)));
    }

    private static async Task<IResult> CreateInviteAsync(
        CreateInviteRequest? request,
        HttpContext http,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        if (await AuthorizeAsync(http, db, ServerPrivileges.ManageServer, cancellationToken) is not { } actor)
        {
            return Results.Forbid();
        }

        Channel? channel = null;
        if (request?.ChannelId is { } channelId)
        {
            channel = await db.Channels.FirstOrDefaultAsync(
                x => x.Id == channelId && x.ServerId == actor.ServerId, cancellationToken);
            if (channel is null)
            {
                return Invalid("channelId", messages.Get(MessageKey.ChannelUnknown));
            }
        }

        var now = DateTime.UtcNow;
        var invite = new ServerInvite
        {
            Id = Guid.NewGuid(),
            ServerId = actor.ServerId,
            CreatedById = actor.UserId,
            ChannelId = channel?.Id,
            Channel = channel,
            Code = Convert.ToHexString(RandomNumberGenerator.GetBytes(InviteCodeBytes)).ToLowerInvariant(),
            MaxUses = InviteMaxUses,
            ExpiresAt = now.AddDays(InviteLifetimeDays),
            CreatedAt = now,
        };
        db.ServerInvites.Add(invite);
        await ServerAudit.StageAsync(db, actor.ServerId, actor.UserId, new AuditEvent(
            "invite.create",
            channel is null
                ? messages.Get(MessageKey.AuditInviteCreated)
                : messages.Get(MessageKey.AuditInviteCreatedForChannel, ("channel", channel.Name)),
            "invite", invite.Id, invite.Code,
            channel is null ? null : [new AuditChange("channel", To: channel.Name)]), cancellationToken);
        await db.SaveChangesAsync(cancellationToken);

        await db.Entry(invite).Reference(x => x.CreatedBy).LoadAsync(cancellationToken);
        return Results.Created($"/api/admin/invites/{invite.Id}", invite.ToDto(InviteOrigin(http)));
    }

    private static async Task<IResult> RevokeInviteAsync(
        Guid inviteId,
        HttpContext http,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        if (await AuthorizeAsync(http, db, ServerPrivileges.ManageServer, cancellationToken) is not { } actor)
        {
            return Results.Forbid();
        }

        var invite = await db.ServerInvites.FirstOrDefaultAsync(
            x => x.Id == inviteId && x.ServerId == actor.ServerId, cancellationToken);
        if (invite is null)
        {
            return Results.NotFound();
        }

        // People who were sent the invite lose it from their inbox with it.
        var sent = await db.UserNotifications
            .Where(x => x.ServerId == actor.ServerId && x.Source == "system" && x.TargetInviteCode == invite.Code)
            .ToListAsync(cancellationToken);
        db.UserNotifications.RemoveRange(sent);
        db.ServerInvites.Remove(invite);
        await ServerAudit.StageAsync(db, actor.ServerId, actor.UserId, new AuditEvent(
            "invite.revoke", messages.Get(MessageKey.AuditInviteRevoked), "invite", invite.Id, invite.Code), cancellationToken);
        await db.SaveChangesAsync(cancellationToken);

        foreach (var recipientId in sent.Select(x => x.UserId).Distinct())
        {
            await NotificationEndpoints.InvitationsChangedAsync(hub, recipientId, actor.ServerId, cancellationToken);
        }

        return Results.NoContent();
    }

    private static async Task<IResult> SendInviteAsync(
        Guid inviteId,
        Guid userId,
        HttpContext http,
        AppDbContext db,
        IHubContext<AppHub> hub,
        CancellationToken cancellationToken)
    {
        if (await AuthorizeAsync(http, db, ServerPrivileges.ManageServer, cancellationToken) is not { } actor)
        {
            return Results.Forbid();
        }

        var now = DateTime.UtcNow;
        var invite = await db.ServerInvites.AsNoTracking().FirstOrDefaultAsync(
            x => x.Id == inviteId && x.ServerId == actor.ServerId, cancellationToken);
        if (invite is null ||
            invite.ExpiresAt <= now ||
            invite.MaxUses <= invite.Uses ||
            !await db.Users.AsNoTracking().AnyAsync(x => x.Id == userId, cancellationToken))
        {
            return Results.NotFound();
        }

        if (await db.ServerMembers.AsNoTracking().AnyAsync(
                x => x.ServerId == actor.ServerId && x.UserId == userId, cancellationToken))
        {
            return Results.Conflict(new { error = "already_member" });
        }

        if (await db.ServerBans.AsNoTracking().AnyAsync(
                x => x.ServerId == actor.ServerId && x.UserId == userId, cancellationToken))
        {
            return Results.Forbid();
        }

        var alreadySent = await db.UserNotifications.AsNoTracking().AnyAsync(
            x => x.UserId == userId &&
                x.ServerId == actor.ServerId &&
                x.Source == "system" &&
                x.TargetInviteCode == invite.Code,
            cancellationToken);
        if (alreadySent)
        {
            return Results.NoContent();
        }

        var serverName = await db.Servers.AsNoTracking()
            .Where(x => x.Id == actor.ServerId)
            .Select(x => x.Name)
            .SingleAsync(cancellationToken);
        var actorName = await DisplayNameAsync(db, actor.UserId, cancellationToken);
        var notification = new UserNotification
        {
            Id = Guid.NewGuid(),
            ServerId = actor.ServerId,
            UserId = userId,
            Source = "system",
            SourcePackageId = "stellarr.server-invite",
            SourcePackageName = serverName,
            SourceModuleId = "server-invite",
            SourceModuleName = actorName,
            Title = "Server invitation",
            Body = $"{actorName} invited you to {serverName}.",
            Tone = "info",
            TargetInviteCode = invite.Code,
            CreatedAt = now,
        };
        db.UserNotifications.Add(notification);
        await db.SaveChangesAsync(cancellationToken);

        await NotificationEndpoints.ChangedAsync(hub, userId, notification, cancellationToken);
        return Results.NoContent();
    }

    private static async Task<IResult> ListAuditAsync(
        DateTime? before,
        HttpContext http,
        AppDbContext db,
        CancellationToken cancellationToken)
    {
        if (await AuthorizeAsync(http, db, ServerPrivileges.ViewAuditLog, cancellationToken) is not { } actor)
        {
            return Results.Forbid();
        }

        // A page of the newest entries, or of those older than `before`: the
        // time of the last entry the reader already has.
        var query = db.ServerAuditEntries.AsNoTracking()
            .Include(x => x.Actor)
            .Where(x => x.ServerId == actor.ServerId);
        if (before is { } cursor)
        {
            var utc = cursor.Kind == DateTimeKind.Unspecified ? DateTime.SpecifyKind(cursor, DateTimeKind.Utc) : cursor.ToUniversalTime();
            query = query.Where(x => x.CreatedAt < utc);
        }

        var entries = await query
            .OrderByDescending(x => x.CreatedAt)
            .Take(AuditPageSize)
            .ToListAsync(cancellationToken);
        return Results.Ok(entries.Select(x => x.ToDto()));
    }

    private static async Task<IResult> GetNotificationsAsync(
        HttpContext http,
        AppDbContext db,
        CancellationToken cancellationToken)
    {
        if (await AuthorizeAsync(http, db, ServerPrivileges.ManageServer, cancellationToken) is not { } actor)
        {
            return Results.Forbid();
        }

        return Results.Ok((await SettingsAsync(db, actor.ServerId, cancellationToken)).ToDto());
    }

    private static async Task<IResult> UpdateNotificationsAsync(
        UpdateNotificationSettingsRequest request,
        HttpContext http,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        if (await AuthorizeAsync(http, db, ServerPrivileges.ManageServer, cancellationToken) is not { } actor)
        {
            return Results.Forbid();
        }

        var muted = request.MutedChannelIds?.Distinct().ToArray();
        if (muted is not null && await db.Channels.CountAsync(
                x => x.ServerId == actor.ServerId && muted.Contains(x.Id), cancellationToken) != muted.Length)
        {
            return Invalid("mutedChannelIds", messages.Get(MessageKey.ChannelUnknown));
        }

        var settings = await SettingsAsync(db, actor.ServerId, cancellationToken);
        settings.AllMessages = request.AllMessages ?? settings.AllMessages;
        settings.MentionsOnly = request.MentionsOnly ?? settings.MentionsOnly;
        settings.VoiceJoinSound = request.VoiceJoinSound ?? settings.VoiceJoinSound;
        settings.ServerMuted = request.ServerMuted ?? settings.ServerMuted;
        if (muted is not null)
        {
            settings.MutedChannelIdsJson = JsonSerializer.Serialize(muted);
        }

        await db.SaveChangesAsync(cancellationToken);
        return Results.Ok(settings.ToDto());
    }

    private static async Task<IResult> TransferAsync(
        TransferServerRequest request,
        HttpContext http,
        AppDbContext db,
        ServerMutationGate gate,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var actor = new Actor(http.RequireActiveServerId(), http.User.RequireUserId());
        await using var turn = await gate.EnterAsync(cancellationToken);
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);

        var owner = await db.ServerMembers.Include(x => x.Role).FirstOrDefaultAsync(
            x => x.ServerId == actor.ServerId && x.UserId == actor.UserId, cancellationToken);
        if (owner?.Role.IsOwner != true)
        {
            return Results.Forbid();
        }

        var successor = await db.ServerMembers.Include(x => x.User).FirstOrDefaultAsync(
            x => x.ServerId == actor.ServerId && x.UserId == request.UserId, cancellationToken);
        if (successor is null || successor.UserId == actor.UserId)
        {
            return Invalid("userId", messages.Get(MessageKey.TransferTargetInvalid));
        }

        await ServerOwnership.HandOverAsync(db, actor.ServerId, owner, successor, cancellationToken);
        await ServerAudit.StageAsync(db, actor.ServerId, actor.UserId, new AuditEvent(
            "server.transfer",
            messages.Get(MessageKey.AuditOwnershipTransferredTo, ("name", successor.User.DisplayName)),
            "member", successor.UserId, successor.User.DisplayName), cancellationToken);
        await db.SaveChangesAsync(cancellationToken);
        await transaction.CommitAsync(cancellationToken);
        return Results.NoContent();
    }

    private static async Task<IResult> DeleteServerAsync(
        HttpContext http,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var actor = new Actor(http.RequireActiveServerId(), http.User.RequireUserId());
        if (!await db.ServerMembers.AsNoTracking().AnyAsync(
                x => x.ServerId == actor.ServerId && x.UserId == actor.UserId && x.Role.IsOwner,
                cancellationToken))
        {
            return Results.Forbid();
        }

        // The instance server carries the instance's settings, and owning it
        // is what opens Cockpit. Without it, whoever owned the next server, or
        // created one, would run the instance. Cockpit refuses it as well.
        if (await InstanceAccess.FindAdministeredServerAsync(db, actor.UserId, cancellationToken) is { } instance &&
            instance.Id == actor.ServerId)
        {
            return Results.Problem(
                statusCode: StatusCodes.Status409Conflict,
                title: messages.Get(MessageKey.InstanceServerCannotBeDeleted));
        }

        return await ServerOwnership.DeleteAsync(db, actor.ServerId, cancellationToken)
            ? Results.NoContent()
            : Results.NotFound();
    }

    private static async Task<Actor?> AuthorizeAsync(
        HttpContext http,
        AppDbContext db,
        ServerPrivileges privilege,
        CancellationToken cancellationToken)
    {
        var actor = new Actor(http.RequireActiveServerId(), http.User.RequireUserId());
        return await ServerAccess.HasAsync(db, actor.UserId, actor.ServerId, privilege, cancellationToken)
            ? actor
            : null;
    }

    /// <summary>
    /// The defaults of a server. Older servers have none yet; they get the
    /// column defaults the first time somebody looks.
    /// </summary>
    private static async Task<ServerAdminSettings> SettingsAsync(
        AppDbContext db,
        Guid serverId,
        CancellationToken cancellationToken)
    {
        if (await db.ServerAdminSettings.FindAsync([serverId], cancellationToken) is { } settings)
        {
            return settings;
        }

        settings = new ServerAdminSettings { ServerId = serverId };
        db.ServerAdminSettings.Add(settings);
        if (!await db.TrySaveChangesAsync(cancellationToken))
        {
            // Another request created them first.
            settings = await db.ServerAdminSettings.SingleAsync(x => x.ServerId == serverId, cancellationToken);
        }

        return settings;
    }

    private static Task<string> DisplayNameAsync(AppDbContext db, Guid userId, CancellationToken cancellationToken) =>
        db.Users.AsNoTracking()
            .Where(x => x.Id == userId)
            .Select(x => x.DisplayName)
            .SingleAsync(cancellationToken);

    /// <summary>
    /// Where an invite link leads: the instance's public address, the one
    /// Cockpit says invitations use, with the scheme and port browsers reach
    /// it at. The request alone loses the port, as nginx hands the API the
    /// host without it, and behind another proxy it may not be the public
    /// address at all. Without a public address the request's is the best
    /// there is.
    /// </summary>
    private static string InviteOrigin(HttpContext http)
    {
        var publicUrl = CockpitEndpoints.PublicUrlOf(http.RequestServices.GetRequiredService<IConfiguration>());
        return publicUrl.Length > 0 ? publicUrl : $"{http.Request.Scheme}://{http.Request.Host}";
    }

    private static IResult Invalid(string field, string message) =>
        Results.ValidationProblem(new Dictionary<string, string[]> { [field] = [message] });
}
