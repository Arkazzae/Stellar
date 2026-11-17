using System.Diagnostics.CodeAnalysis;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Auth;

public enum AuthorizationFailure { None, Forbidden, NotFound }

public sealed record MemberAuthorization(ServerMember? Actor, ServerMember? Target, AuthorizationFailure Failure)
{
    [MemberNotNullWhen(true, nameof(Actor), nameof(Target))]
    public bool Allowed => Failure == AuthorizationFailure.None && Actor is not null && Target is not null;
}

/// <summary>
/// The shared authorization policy for HTTP and realtime operations. Permission
/// checks name their resource; delegation and moderation also check the target
/// and hierarchy. No decisions are cached across requests or hub invocations.
/// </summary>
public static partial class ServerAuthorization
{
    public static async Task<ServerPrivileges?> EffectiveAsync(
        AppDbContext db, Guid userId, PermissionScope scope, CancellationToken cancellationToken = default)
    {
        var member = await MemberAsync(db, userId, scope.ServerId, cancellationToken);
        return member is null ? null : await EffectiveAsync(db, member.Role, scope, cancellationToken);
    }

    public static async Task<bool> AllowsAsync(
        AppDbContext db, Guid userId, PermissionScope scope, ServerPrivileges required,
        CancellationToken cancellationToken = default) =>
        Valid(required) && await EffectiveAsync(db, userId, scope, cancellationToken) is { } held && Contains(held, required);

    public static async Task<bool> CanCreateRoleAsync(
        AppDbContext db, Guid actorId, Guid serverId, ServerPrivileges permissions, CancellationToken cancellationToken) =>
        await MemberAsync(db, actorId, serverId, cancellationToken) is { } actor &&
        Contains(actor.Role.Permissions, ServerPrivileges.ManageRoles) && Contains(actor.Role.Permissions, permissions);

    public static async Task<bool> CanEditRoleAsync(
        AppDbContext db, Guid actorId, ServerRole role, ServerPrivileges permissions, CancellationToken cancellationToken)
    {
        var actor = await MemberAsync(db, actorId, role.ServerId, cancellationToken);
        return actor is not null && Contains(actor.Role.Permissions, ServerPrivileges.ManageRoles) &&
            Outranks(actor.Role, role) && Contains(actor.Role.Permissions, permissions) &&
            Contains(actor.Role.Permissions, await DelegatedAsync(db, role, cancellationToken));
    }

    public static async Task<bool> CanDeleteRoleAsync(
        AppDbContext db, Guid actorId, ServerRole role, ServerRole fallback, CancellationToken cancellationToken)
    {
        var actor = await MemberAsync(db, actorId, role.ServerId, cancellationToken);
        return actor is not null && !role.IsSystem && fallback.ServerId == role.ServerId &&
            Contains(actor.Role.Permissions, ServerPrivileges.ManageRoles) &&
            Outranks(actor.Role, role) && Outranks(actor.Role, fallback) &&
            Contains(actor.Role.Permissions, await DelegatedAsync(db, role, cancellationToken)) &&
            Contains(actor.Role.Permissions, await DelegatedAsync(db, fallback, cancellationToken));
    }

    public static async Task<bool> CanAssignRoleAsync(
        AppDbContext db, Guid actorId, ServerMember target, ServerRole role, CancellationToken cancellationToken)
    {
        var actor = await MemberAsync(db, actorId, target.ServerId, cancellationToken);
        return actor is not null && actorId != target.UserId && role.ServerId == target.ServerId &&
            Contains(actor.Role.Permissions, ServerPrivileges.ManageMembers) &&
            Outranks(actor.Role, target.Role) && Outranks(actor.Role, role) &&
            Contains(actor.Role.Permissions, await DelegatedAsync(db, target.Role, cancellationToken)) &&
            Contains(actor.Role.Permissions, await DelegatedAsync(db, role, cancellationToken));
    }

    /// <summary>
    /// Setting, clearing and deleting an override use the same decision. Both
    /// old and new masks count: removing a deny can grant a power as surely as
    /// adding an allow. A local grant cannot delegate powers beyond the actor's
    /// server role, and a local deny restricts the actor here too.
    /// </summary>
    public static async Task<bool> CanChangeOverrideAsync(
        AppDbContext db, Guid actorId, PermissionScope scope, Guid roleId,
        ServerPrivileges allow, ServerPrivileges deny, CancellationToken cancellationToken)
    {
        if (scope.Kind == PermissionScope.ScopeKind.Server || !Valid(allow | deny) || (allow & deny) != 0) return false;
        var actor = await MemberAsync(db, actorId, scope.ServerId, cancellationToken);
        var role = await db.ServerRoles.AsNoTracking()
            .SingleOrDefaultAsync(x => x.Id == roleId && x.ServerId == scope.ServerId, cancellationToken);
        if (actor is null || role is null ||
            !Contains(actor.Role.Permissions, ServerPrivileges.ManageRoles)) return false;

        // The owner can repair legacy restrictions on their own role. Other
        // actors cannot edit their own role or a peer/higher role.
        var ownerRecovery = actor.Role.IsOwner && actor.Role.Id == role.Id;
        if (!ownerRecovery && !Outranks(actor.Role, role)) return false;

        var effective = await EffectiveAsync(db, actor.Role, scope, cancellationToken);
        if (effective is null || (!ownerRecovery && !Contains(effective.Value, ServerPrivileges.ManageRoles))) return false;
        var type = scope.Kind == PermissionScope.ScopeKind.Channel ? PermissionTargetType.Channel : PermissionTargetType.Section;
        var current = await db.PermissionOverrides.AsNoTracking().SingleOrDefaultAsync(x =>
            x.ServerId == scope.ServerId && x.TargetType == type && x.TargetId == scope.ResourceId && x.RoleId == roleId,
            cancellationToken);
        var touched = allow | deny | (current?.AllowMask ?? ServerPrivileges.None) | (current?.DenyMask ?? ServerPrivileges.None);
        return Contains(ownerRecovery ? actor.Role.Permissions : actor.Role.Permissions & effective.Value, touched);
    }

    public static Task<MemberAuthorization> KickMemberAsync(
        AppDbContext db, Guid actorId, Guid serverId, Guid targetId, CancellationToken cancellationToken) =>
        ModerateAsync(db, actorId, targetId, PermissionScope.Server(serverId), ServerPrivileges.ModerateMembers, cancellationToken);

    public static Task<MemberAuthorization> BanMemberAsync(
        AppDbContext db, Guid actorId, Guid serverId, Guid targetId, CancellationToken cancellationToken) =>
        ModerateAsync(db, actorId, targetId, PermissionScope.Server(serverId), ServerPrivileges.ManageBans, cancellationToken);

    public static async Task<MemberAuthorization> KickVoiceMemberAsync(
        AppDbContext db, Guid actorId, Guid serverId, Guid channelId, Guid targetId, CancellationToken cancellationToken)
    {
        if (!await VoiceChannelAsync(db, serverId, channelId, cancellationToken))
            return new(null, null, AuthorizationFailure.NotFound);
        return await ModerateAsync(db, actorId, targetId, PermissionScope.Channel(serverId, channelId),
            ServerPrivileges.ModerateMembers, cancellationToken);
    }

    public static async Task<MemberAuthorization> MoveVoiceMemberAsync(
        AppDbContext db, Guid actorId, Guid serverId, Guid sourceChannelId, Guid targetChannelId,
        Guid targetId, CancellationToken cancellationToken)
    {
        if (!await VoiceChannelAsync(db, serverId, targetChannelId, cancellationToken))
            return new(null, null, AuthorizationFailure.NotFound);
        var source = await KickVoiceMemberAsync(db, actorId, serverId, sourceChannelId, targetId, cancellationToken);
        if (!source.Allowed) return source;
        return await AllowsAsync(db, actorId, PermissionScope.Channel(serverId, targetChannelId),
            ServerPrivileges.ModerateMembers, cancellationToken)
            ? source : new(null, null, AuthorizationFailure.Forbidden);
    }

    private static async Task<MemberAuthorization> ModerateAsync(
        AppDbContext db, Guid actorId, Guid targetId, PermissionScope scope,
        ServerPrivileges required, CancellationToken cancellationToken)
    {
        var actor = await MemberAsync(db, actorId, scope.ServerId, cancellationToken);
        if (actor is null || actorId == targetId ||
            await EffectiveAsync(db, actor.Role, scope, cancellationToken) is not { } held || !Contains(held, required))
            return new(null, null, AuthorizationFailure.Forbidden);
        var target = await MemberAsync(db, targetId, scope.ServerId, cancellationToken);
        if (target is null) return new(null, null, AuthorizationFailure.NotFound);
        return Outranks(actor.Role, target.Role)
            ? new(actor, target, AuthorizationFailure.None) : new(null, null, AuthorizationFailure.Forbidden);
    }

    private static Task<ServerMember?> MemberAsync(
        AppDbContext db, Guid userId, Guid serverId, CancellationToken cancellationToken) =>
        db.ServerMembers.AsNoTracking().Include(x => x.Role).Include(x => x.User)
            .SingleOrDefaultAsync(x => x.ServerId == serverId && x.UserId == userId && !x.User.IsDisabled, cancellationToken);

    private static Task<bool> VoiceChannelAsync(AppDbContext db, Guid serverId, Guid channelId, CancellationToken cancellationToken) =>
        db.Channels.AsNoTracking().AnyAsync(x => x.ServerId == serverId && x.Id == channelId && x.Kind == ChannelKind.Voice,
            cancellationToken);

    private static bool Outranks(ServerRole actor, ServerRole target) =>
        actor.ServerId == target.ServerId && actor.Id != target.Id && !target.IsOwner &&
        (actor.IsOwner || actor.Position < target.Position);

    private static bool Valid(ServerPrivileges permissions) => (permissions & ~ServerPrivileges.All) == 0;
    private static bool Contains(ServerPrivileges held, ServerPrivileges required) =>
        Valid(required) && (held & required) == required;

    private static async Task<ServerPrivileges> DelegatedAsync(AppDbContext db, ServerRole role, CancellationToken cancellationToken)
    {
        var grants = await db.PermissionOverrides.AsNoTracking()
            .Where(x => x.ServerId == role.ServerId && x.RoleId == role.Id &&
                ((x.TargetType == PermissionTargetType.Channel &&
                    db.Channels.Any(channel => channel.Id == x.TargetId && channel.ServerId == role.ServerId)) ||
                 (x.TargetType == PermissionTargetType.Section &&
                    db.Sections.Any(section => section.Id == x.TargetId && section.ServerId == role.ServerId))))
            .Select(x => x.AllowMask).ToListAsync(cancellationToken);
        return grants.Aggregate(role.Permissions, (held, grant) => held | grant);
    }

    private static async Task<ServerPrivileges?> EffectiveAsync(
        AppDbContext db, ServerRole role, PermissionScope scope, CancellationToken cancellationToken)
    {
        if (scope.ServerId == Guid.Empty || scope.ServerId != role.ServerId || scope.ResourceId == Guid.Empty) return null;
        if (scope.Kind == PermissionScope.ScopeKind.Server) return role.Permissions;
        Guid? sectionId;
        if (scope.Kind == PermissionScope.ScopeKind.Channel)
        {
            var channel = await db.Channels.AsNoTracking()
                .Where(x => x.Id == scope.ResourceId && x.ServerId == scope.ServerId)
                .Select(x => new { x.SectionId }).SingleOrDefaultAsync(cancellationToken);
            if (channel is null) return null;
            sectionId = channel.SectionId;
        }
        else
        {
            if (!await db.Sections.AnyAsync(x => x.Id == scope.ResourceId && x.ServerId == scope.ServerId, cancellationToken)) return null;
            sectionId = scope.ResourceId;
        }
        var effective = role.Permissions;
        if (sectionId is { } section)
            effective = await ApplyAsync(db, role, effective, PermissionTargetType.Section, section, cancellationToken);
        if (scope.Kind == PermissionScope.ScopeKind.Channel)
            effective = await ApplyAsync(db, role, effective, PermissionTargetType.Channel, scope.ResourceId, cancellationToken);
        return effective;
    }

    private static async Task<ServerPrivileges> ApplyAsync(
        AppDbContext db, ServerRole role, ServerPrivileges held, PermissionTargetType type, Guid targetId, CancellationToken cancellationToken)
    {
        var masks = await db.PermissionOverrides.AsNoTracking().SingleOrDefaultAsync(x =>
            x.ServerId == role.ServerId && x.RoleId == role.Id && x.TargetType == type && x.TargetId == targetId, cancellationToken);
        return masks is null ? held : (held | masks.AllowMask) & ~masks.DenyMask;
    }
}
