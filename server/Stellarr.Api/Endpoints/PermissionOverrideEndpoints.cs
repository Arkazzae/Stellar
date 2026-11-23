using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;

namespace Stellarr.Api.Endpoints;

/// <summary>
/// What a role, or one member, may do beyond, or short of, what their roles
/// grant inside one channel or one section. Who may set it is decided by
/// ManageRoles and not by ManageChannels: it edits what people may do, as the
/// roles editor does. Member overrides have routes of their own beside the
/// role ones, so a list of role overrides stays a list of role overrides.
/// Administrator is server-wide only and never part of an override.
/// </summary>
public static class PermissionOverrideEndpoints
{
    public static IEndpointRouteBuilder MapPermissionOverrideEndpoints(this IEndpointRouteBuilder endpoints)
    {
        MapTarget(endpoints, "channels", PermissionTargetType.Channel);
        MapTarget(endpoints, "sections", PermissionTargetType.Section);
        return endpoints;
    }

    private static void MapTarget(IEndpointRouteBuilder endpoints, string collection, PermissionTargetType targetType)
    {
        var overrides = endpoints.MapGroup($"/api/admin/{collection}/{{targetId:guid}}/permissions")
            .RequireAuthorization()
            .AddEndpointFilter<ServerMembershipFilter>();
        overrides.MapGet("", (
            Guid targetId, HttpContext http, AppDbContext db, CancellationToken cancellationToken) =>
            ListAsync(targetType, targetId, http, db, cancellationToken));
        overrides.MapPut("/{roleId:guid}", (
            Guid targetId, Guid roleId, SetPermissionOverrideRequest request,
            HttpContext http, AppDbContext db, Messages messages, CancellationToken cancellationToken) =>
            SetAsync(targetType, targetId, roleId, request, http, db, messages, cancellationToken))
            .AddEndpointFilter<InstanceServerFreshSecondStepFilter>();
        overrides.MapDelete("/{roleId:guid}", (
            Guid targetId, Guid roleId, HttpContext http, AppDbContext db, Messages messages,
            CancellationToken cancellationToken) =>
            ClearAsync(targetType, targetId, roleId, http, db, messages, cancellationToken))
            .AddEndpointFilter<InstanceServerFreshSecondStepFilter>();
        overrides.MapGet("/members", (
            Guid targetId, HttpContext http, AppDbContext db, CancellationToken cancellationToken) =>
            ListMembersAsync(targetType, targetId, http, db, cancellationToken));
        overrides.MapPut("/members/{userId:guid}", (
            Guid targetId, Guid userId, SetPermissionOverrideRequest request,
            HttpContext http, AppDbContext db, Messages messages, CancellationToken cancellationToken) =>
            SetMemberAsync(targetType, targetId, userId, request, http, db, messages, cancellationToken))
            .AddEndpointFilter<InstanceServerFreshSecondStepFilter>();
        overrides.MapDelete("/members/{userId:guid}", (
            Guid targetId, Guid userId, HttpContext http, AppDbContext db, Messages messages,
            CancellationToken cancellationToken) =>
            ClearMemberAsync(targetType, targetId, userId, http, db, messages, cancellationToken))
            .AddEndpointFilter<InstanceServerFreshSecondStepFilter>();
    }

    private static async Task<IResult> ListAsync(
        PermissionTargetType targetType,
        Guid targetId,
        HttpContext http,
        AppDbContext db,
        CancellationToken cancellationToken)
    {
        if (await RefuseAsync(targetType, targetId, http, db, cancellationToken) is { } refusal)
        {
            return refusal;
        }

        var overrides = await db.PermissionOverrides.AsNoTracking()
            .Where(x => x.TargetType == targetType && x.TargetId == targetId)
            .ToListAsync(cancellationToken);
        return Results.Ok(overrides.Select(x => x.ToDto()).ToArray());
    }

    private static async Task<IResult> SetAsync(
        PermissionTargetType targetType,
        Guid targetId,
        Guid roleId,
        SetPermissionOverrideRequest request,
        HttpContext http,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        if (await RefuseAsync(targetType, targetId, http, db, cancellationToken) is { } refusal)
        {
            return refusal;
        }

        var serverId = http.RequireActiveServerId();
        if (!await db.ServerRoles.AnyAsync(x => x.Id == roleId && x.ServerId == serverId, cancellationToken))
        {
            return Results.NotFound();
        }

        if (!ServerAccess.TryParse(request.Allow, out var allow) || !ServerAccess.TryParse(request.Deny, out var deny))
        {
            return Invalid(messages.Get(MessageKey.PermissionUnknown));
        }

        if ((allow & deny) != ServerPrivileges.None)
        {
            return Invalid(messages.Get(MessageKey.PermissionAllowedAndDenied));
        }

        if (((allow | deny) & ServerPrivileges.Administrator) != ServerPrivileges.None)
        {
            return Invalid(messages.Get(MessageKey.PermissionServerWideOnly));
        }

        if (!await ServerAuthorization.CanChangeOverrideAsync(
                db, http.User.RequireUserId(), Scope(targetType, serverId, targetId), roleId, allow, deny, cancellationToken))
        {
            return Results.Forbid();
        }

        var current = await db.PermissionOverrides.FirstOrDefaultAsync(
            x => x.TargetType == targetType && x.TargetId == targetId && x.RoleId == roleId, cancellationToken);
        await StageAsync(db, messages, http, serverId, targetType, targetId, roleId,
            current?.AllowMask ?? ServerPrivileges.None, current?.DenyMask ?? ServerPrivileges.None, allow, deny,
            cancellationToken);
        if (allow == ServerPrivileges.None && deny == ServerPrivileges.None)
        {
            if (current is not null)
            {
                db.PermissionOverrides.Remove(current);
            }

            await db.SaveChangesAsync(cancellationToken);

            await transaction.CommitAsync(cancellationToken);
            return Results.Ok(new PermissionOverrideDto(roleId.ToString(), [], []));
        }

        if (current is null)
        {
            current = new PermissionOverride
            {
                Id = Guid.NewGuid(),
                ServerId = serverId,
                TargetType = targetType,
                TargetId = targetId,
                RoleId = roleId,
            };
            db.PermissionOverrides.Add(current);
        }

        current.AllowMask = allow;
        current.DenyMask = deny;
        if (!await db.TrySaveChangesAsync(cancellationToken)) return Results.Conflict();
        await transaction.CommitAsync(cancellationToken);
        return Results.Ok(current.ToDto());
    }

    private static async Task<IResult> ClearAsync(
        PermissionTargetType targetType,
        Guid targetId,
        Guid roleId,
        HttpContext http,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        if (await RefuseAsync(targetType, targetId, http, db, cancellationToken) is { } refusal)
        {
            return refusal;
        }

        var serverId = http.RequireActiveServerId();
        if (!await db.ServerRoles.AnyAsync(x => x.Id == roleId && x.ServerId == serverId, cancellationToken))
        {
            return Results.NotFound();
        }
        if (!await ServerAuthorization.CanChangeOverrideAsync(
                db, http.User.RequireUserId(), Scope(targetType, serverId, targetId), roleId,
                ServerPrivileges.None, ServerPrivileges.None, cancellationToken))
        {
            return Results.Forbid();
        }

        var cleared = await db.PermissionOverrides.AsNoTracking().FirstOrDefaultAsync(
            x => x.TargetType == targetType && x.TargetId == targetId && x.RoleId == roleId, cancellationToken);
        if (cleared is not null)
        {
            await db.PermissionOverrides.Where(x => x.Id == cleared.Id).ExecuteDeleteAsync(cancellationToken);
            await StageAsync(db, messages, http, serverId, targetType, targetId, roleId,
                cleared.AllowMask, cleared.DenyMask, ServerPrivileges.None, ServerPrivileges.None, cancellationToken);
            await db.SaveChangesAsync(cancellationToken);
        }

        await transaction.CommitAsync(cancellationToken);
        return Results.NoContent();
    }

    private static async Task<IResult> ListMembersAsync(
        PermissionTargetType targetType,
        Guid targetId,
        HttpContext http,
        AppDbContext db,
        CancellationToken cancellationToken)
    {
        if (await RefuseAsync(targetType, targetId, http, db, cancellationToken) is { } refusal)
        {
            return refusal;
        }

        var serverId = http.RequireActiveServerId();
        var overrides = await db.MemberPermissionOverrides.AsNoTracking()
            .Where(x => x.ServerId == serverId && x.TargetType == targetType && x.TargetId == targetId)
            .ToListAsync(cancellationToken);
        return Results.Ok(overrides.Select(x => x.ToDto()).ToArray());
    }

    private static async Task<IResult> SetMemberAsync(
        PermissionTargetType targetType,
        Guid targetId,
        Guid userId,
        SetPermissionOverrideRequest request,
        HttpContext http,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        if (await RefuseAsync(targetType, targetId, http, db, cancellationToken) is { } refusal)
        {
            return refusal;
        }

        var serverId = http.RequireActiveServerId();
        var member = await MemberAsync(db, serverId, userId, cancellationToken);
        if (member is null)
        {
            return Results.NotFound();
        }

        if (!ServerAccess.TryParse(request.Allow, out var allow) || !ServerAccess.TryParse(request.Deny, out var deny))
        {
            return Invalid(messages.Get(MessageKey.PermissionUnknown));
        }

        if ((allow & deny) != ServerPrivileges.None)
        {
            return Invalid(messages.Get(MessageKey.PermissionAllowedAndDenied));
        }

        if (((allow | deny) & ServerPrivileges.Administrator) != ServerPrivileges.None)
        {
            return Invalid(messages.Get(MessageKey.PermissionServerWideOnly));
        }

        // The owner may do everything everywhere; an override would say otherwise.
        if (member.Role.IsOwner)
        {
            return Results.Conflict();
        }

        if (!await ServerAuthorization.CanChangeMemberOverrideAsync(
                db, http.User.RequireUserId(), Scope(targetType, serverId, targetId), userId, allow, deny, cancellationToken))
        {
            return Results.Forbid();
        }

        var current = await db.MemberPermissionOverrides.FirstOrDefaultAsync(
            x => x.ServerId == serverId && x.TargetType == targetType && x.TargetId == targetId && x.UserId == userId,
            cancellationToken);
        await StageMemberAsync(db, messages, http, serverId, targetType, targetId, member.User,
            OverrideMasks.Of(current), new OverrideMasks(allow, deny), cancellationToken);
        if (allow == ServerPrivileges.None && deny == ServerPrivileges.None)
        {
            if (current is not null)
            {
                db.MemberPermissionOverrides.Remove(current);
            }

            await db.SaveChangesAsync(cancellationToken);
            await transaction.CommitAsync(cancellationToken);
            return Results.Ok(new MemberPermissionOverrideDto(userId.ToString(), [], []));
        }

        if (current is null)
        {
            current = new MemberPermissionOverride
            {
                Id = Guid.NewGuid(),
                ServerId = serverId,
                TargetType = targetType,
                TargetId = targetId,
                UserId = userId,
            };
            db.MemberPermissionOverrides.Add(current);
        }

        current.AllowMask = allow;
        current.DenyMask = deny;
        if (!await db.TrySaveChangesAsync(cancellationToken)) return Results.Conflict();
        await transaction.CommitAsync(cancellationToken);
        return Results.Ok(current.ToDto());
    }

    private static async Task<IResult> ClearMemberAsync(
        PermissionTargetType targetType,
        Guid targetId,
        Guid userId,
        HttpContext http,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        if (await RefuseAsync(targetType, targetId, http, db, cancellationToken) is { } refusal)
        {
            return refusal;
        }

        var serverId = http.RequireActiveServerId();
        var member = await MemberAsync(db, serverId, userId, cancellationToken);
        if (member is null)
        {
            return Results.NotFound();
        }

        if (member.Role.IsOwner)
        {
            return Results.Conflict();
        }

        if (!await ServerAuthorization.CanChangeMemberOverrideAsync(
                db, http.User.RequireUserId(), Scope(targetType, serverId, targetId), userId,
                ServerPrivileges.None, ServerPrivileges.None, cancellationToken))
        {
            return Results.Forbid();
        }

        var cleared = await db.MemberPermissionOverrides.AsNoTracking().FirstOrDefaultAsync(
            x => x.ServerId == serverId && x.TargetType == targetType && x.TargetId == targetId && x.UserId == userId,
            cancellationToken);
        if (cleared is not null)
        {
            await db.MemberPermissionOverrides.Where(x => x.Id == cleared.Id).ExecuteDeleteAsync(cancellationToken);
            await StageMemberAsync(db, messages, http, serverId, targetType, targetId, member.User,
                OverrideMasks.Of(cleared), default, cancellationToken);
            await db.SaveChangesAsync(cancellationToken);
        }

        await transaction.CommitAsync(cancellationToken);
        return Results.NoContent();
    }

    private static Task<ServerMember?> MemberAsync(
        AppDbContext db, Guid serverId, Guid userId, CancellationToken cancellationToken) =>
        db.ServerMembers.AsNoTracking().Include(x => x.Role).Include(x => x.User)
            .FirstOrDefaultAsync(x => x.ServerId == serverId && x.UserId == userId, cancellationToken);

    /// <summary>
    /// The answer for a request that may not go on: 404 for a channel or section
    /// that is not on the server, which is looked for first, then 403 for a
    /// caller who may not manage roles. Null when it may.
    /// </summary>
    private static async Task<IResult?> RefuseAsync(
        PermissionTargetType targetType,
        Guid targetId,
        HttpContext http,
        AppDbContext db,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        var exists = targetType == PermissionTargetType.Channel
            ? await db.Channels.AnyAsync(x => x.Id == targetId && x.ServerId == serverId, cancellationToken)
            : await db.Sections.AnyAsync(x => x.Id == targetId && x.ServerId == serverId, cancellationToken);
        if (!exists)
        {
            return Results.NotFound();
        }

        return await ServerAccess.HasAsync(
            db, http.User.RequireUserId(), serverId, ServerPrivileges.ManageRoles, cancellationToken)
            ? null
            : Results.Forbid();
    }

    /// <summary>A line for the audit log: which role, where, and what it was allowed and denied.</summary>
    private static async Task StageAsync(
        AppDbContext db,
        Messages messages,
        HttpContext http,
        Guid serverId,
        PermissionTargetType targetType,
        Guid targetId,
        Guid roleId,
        ServerPrivileges allowBefore,
        ServerPrivileges denyBefore,
        ServerPrivileges allow,
        ServerPrivileges deny,
        CancellationToken cancellationToken)
    {
        if (allowBefore == allow && denyBefore == deny)
        {
            return;
        }

        var role = await db.ServerRoles.AsNoTracking().SingleAsync(x => x.Id == roleId, cancellationToken);
        var isChannel = targetType == PermissionTargetType.Channel;
        var name = await TargetNameAsync(db, targetType, targetId, cancellationToken);
        await ServerAudit.StageAsync(db, serverId, http.User.RequireUserId(), new AuditEvent(
            isChannel ? "channel.permissions" : "section.permissions",
            messages.Get(
                isChannel ? MessageKey.AuditChannelPermissions : MessageKey.AuditSectionPermissions,
                ("role", role.Name), ("name", name)),
            isChannel ? "channel" : "section", targetId, name,
            ServerAudit.Only(
                new AuditChange("role", To: role.Name, ToColor: role.Color),
                Mask("allow", allowBefore, allow),
                Mask("deny", denyBefore, deny))), cancellationToken);
    }

    /// <summary>A line for the audit log: which member, where, and what they were allowed and denied.</summary>
    private static async Task StageMemberAsync(
        AppDbContext db,
        Messages messages,
        HttpContext http,
        Guid serverId,
        PermissionTargetType targetType,
        Guid targetId,
        User member,
        OverrideMasks before,
        OverrideMasks after,
        CancellationToken cancellationToken)
    {
        if (before == after)
        {
            return;
        }

        var isChannel = targetType == PermissionTargetType.Channel;
        var name = await TargetNameAsync(db, targetType, targetId, cancellationToken);
        await ServerAudit.StageAsync(db, serverId, http.User.RequireUserId(), new AuditEvent(
            isChannel ? "channel.permissions.member" : "section.permissions.member",
            messages.Get(
                isChannel ? MessageKey.AuditChannelMemberPermissions : MessageKey.AuditSectionMemberPermissions,
                ("member", member.DisplayName), ("name", name)),
            isChannel ? "channel" : "section", targetId, name,
            ServerAudit.Only(
                new AuditChange("member", To: member.DisplayName),
                Mask("allow", before.Allow, after.Allow),
                Mask("deny", before.Deny, after.Deny))), cancellationToken);
    }

    private static async Task<string> TargetNameAsync(
        AppDbContext db, PermissionTargetType targetType, Guid targetId, CancellationToken cancellationToken) =>
        targetType == PermissionTargetType.Channel
            ? await db.Channels.Where(x => x.Id == targetId).Select(x => x.Name).SingleAsync(cancellationToken)
            : await db.Sections.Where(x => x.Id == targetId).Select(x => x.Name).SingleAsync(cancellationToken);

    private static AuditChange? Mask(string field, ServerPrivileges before, ServerPrivileges after)
    {
        var added = ServerAccess.ToNames(after & ~before);
        var removed = ServerAccess.ToNames(before & ~after);
        return added.Count + removed.Count == 0 ? null : new AuditChange(field, Added: added, Removed: removed);
    }

    private static PermissionScope Scope(PermissionTargetType type, Guid serverId, Guid targetId) =>
        type == PermissionTargetType.Channel
            ? PermissionScope.Channel(serverId, targetId)
            : PermissionScope.Section(serverId, targetId);

    private static IResult Invalid(string message) =>
        Results.ValidationProblem(new Dictionary<string, string[]> { ["permissions"] = [message] });
}
