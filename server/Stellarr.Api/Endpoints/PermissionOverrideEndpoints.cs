using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;

namespace Stellarr.Api.Endpoints;

/// <summary>
/// What a role may do beyond, or short of, its own permissions inside one
/// channel or one section. Who may set it is decided by ManageRoles and not by
/// ManageChannels: it edits what a role grants, as the roles editor does.
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
        var name = isChannel
            ? await db.Channels.Where(x => x.Id == targetId).Select(x => x.Name).SingleAsync(cancellationToken)
            : await db.Sections.Where(x => x.Id == targetId).Select(x => x.Name).SingleAsync(cancellationToken);
        AuditChange? Mask(string field, ServerPrivileges before, ServerPrivileges after)
        {
            var added = ServerAccess.ToNames(after & ~before);
            var removed = ServerAccess.ToNames(before & ~after);
            return added.Count + removed.Count == 0 ? null : new AuditChange(field, Added: added, Removed: removed);
        }

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

    private static PermissionScope Scope(PermissionTargetType type, Guid serverId, Guid targetId) =>
        type == PermissionTargetType.Channel
            ? PermissionScope.Channel(serverId, targetId)
            : PermissionScope.Section(serverId, targetId);

    private static IResult Invalid(string message) =>
        Results.ValidationProblem(new Dictionary<string, string[]> { ["permissions"] = [message] });
}
