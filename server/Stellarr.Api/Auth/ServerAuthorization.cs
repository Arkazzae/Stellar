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
        if (await PlaceAsync(db, scope, cancellationToken) is not { } place) return null;
        var snapshot = await PermissionSnapshot.LoadAsync(db, scope.ServerId, [userId], place.Targets, cancellationToken);
        return snapshot.Subject(userId) is { } member ? place.Evaluate(snapshot, member) : null;
    }

    public static async Task<bool> AllowsAsync(
        AppDbContext db, Guid userId, PermissionScope scope, ServerPrivileges required,
        CancellationToken cancellationToken = default) =>
        Valid(required) && await EffectiveAsync(db, userId, scope, cancellationToken) is { } held && Contains(held, required);

    public static async Task<bool> CanCreateRoleAsync(
        AppDbContext db, Guid actorId, Guid serverId, ServerPrivileges permissions, CancellationToken cancellationToken) =>
        await SubjectAsync(db, actorId, serverId, cancellationToken) is { } actor &&
        Contains(actor.Permissions, ServerPrivileges.ManageRoles) && Contains(actor.Permissions, permissions);

    public static async Task<bool> CanEditRoleAsync(
        AppDbContext db, Guid actorId, ServerRole role, ServerPrivileges permissions, CancellationToken cancellationToken)
    {
        var actor = await SubjectAsync(db, actorId, role.ServerId, cancellationToken);
        return actor is not null && Contains(actor.Permissions, ServerPrivileges.ManageRoles) &&
            Outranks(actor.TopRole, role) && Contains(actor.Permissions, permissions) &&
            Contains(actor.Permissions, await DelegatedAsync(db, role, cancellationToken));
    }

    public static async Task<bool> CanDeleteRoleAsync(
        AppDbContext db, Guid actorId, ServerRole role, ServerRole fallback, CancellationToken cancellationToken)
    {
        var actor = await SubjectAsync(db, actorId, role.ServerId, cancellationToken);
        return actor is not null && !role.IsSystem && fallback.ServerId == role.ServerId &&
            Contains(actor.Permissions, ServerPrivileges.ManageRoles) &&
            Outranks(actor.TopRole, role) && Outranks(actor.TopRole, fallback) &&
            Contains(actor.Permissions, await DelegatedAsync(db, role, cancellationToken)) &&
            Contains(actor.Permissions, await DelegatedAsync(db, fallback, cancellationToken));
    }

    /// <summary>
    /// Giving roles to a member and taking them away are decided alike, for
    /// every role in <paramref name="changed"/>: the actor needs ManageMembers,
    /// must outrank the member and each role, must not be the member, and must
    /// hold everything each role could grant, its overrides' allows included.
    /// </summary>
    public static async Task<bool> CanChangeRolesAsync(
        AppDbContext db, Guid actorId, ServerMember target, IReadOnlyCollection<ServerRole> changed,
        CancellationToken cancellationToken)
    {
        var actor = await SubjectAsync(db, actorId, target.ServerId, cancellationToken);
        if (actor is null || actorId == target.UserId ||
            !Contains(actor.Permissions, ServerPrivileges.ManageMembers) || !Outranks(actor.TopRole, target.Role)) return false;
        foreach (var role in changed)
        {
            if (role.ServerId != target.ServerId || !Outranks(actor.TopRole, role) ||
                !Contains(actor.Permissions, await DelegatedAsync(db, role, cancellationToken))) return false;
        }

        return true;
    }

    /// <summary>
    /// Setting, clearing and deleting an override use the same decision. Both
    /// old and new masks count: removing a deny can grant a power as surely as
    /// adding an allow. A local grant cannot delegate powers beyond the actor's
    /// server-wide permissions, and a local deny restricts the actor here too.
    /// </summary>
    public static async Task<bool> CanChangeOverrideAsync(
        AppDbContext db, Guid actorId, PermissionScope scope, Guid roleId,
        ServerPrivileges allow, ServerPrivileges deny, CancellationToken cancellationToken)
    {
        if (scope.Kind == PermissionScope.ScopeKind.Server || !Valid(allow | deny) || (allow & deny) != 0) return false;
        if (await PlaceAsync(db, scope, cancellationToken) is not { } place) return false;
        var snapshot = await PermissionSnapshot.LoadAsync(db, scope.ServerId, [actorId], place.Targets, cancellationToken);
        var actor = snapshot.Subject(actorId);
        var role = snapshot.Roles.FirstOrDefault(x => x.Id == roleId);
        if (actor is null || role is null ||
            !Contains(actor.Permissions, ServerPrivileges.ManageRoles)) return false;

        // The owner can repair legacy restrictions on their own role. Other
        // actors cannot edit their own role or a peer/higher role.
        var ownerRecovery = actor.IsOwner && actor.TopRole.Id == role.Id;
        if (!ownerRecovery && !Outranks(actor.TopRole, role)) return false;

        var effective = place.Evaluate(snapshot, actor);
        if (!ownerRecovery && !Contains(effective, ServerPrivileges.ManageRoles)) return false;
        var current = snapshot.RoleOverride(place.TargetType, scope.ResourceId, roleId);
        var touched = allow | deny | current.Touched;
        return Contains(ownerRecovery ? actor.Permissions : actor.Permissions & effective, touched);
    }

    /// <summary>
    /// The same decision for a member's own override, with the member in place
    /// of the role: the actor must outrank them, which nobody does to
    /// themselves or to the owner, and must hold every bit of both the old and
    /// the new masks, here as well as server-wide.
    /// </summary>
    public static async Task<bool> CanChangeMemberOverrideAsync(
        AppDbContext db, Guid actorId, PermissionScope scope, Guid targetId,
        ServerPrivileges allow, ServerPrivileges deny, CancellationToken cancellationToken)
    {
        if (scope.Kind == PermissionScope.ScopeKind.Server || !Valid(allow | deny) || (allow & deny) != 0 ||
            actorId == targetId) return false;
        if (await PlaceAsync(db, scope, cancellationToken) is not { } place) return false;
        var snapshot = await PermissionSnapshot.LoadAsync(db, scope.ServerId, [actorId, targetId], place.Targets, cancellationToken);
        var actor = snapshot.Subject(actorId);
        // A disabled account still has its rank and its overrides; only acting needs an enabled one.
        var target = await db.ServerMembers.AsNoTracking().Include(x => x.Role)
            .SingleOrDefaultAsync(x => x.ServerId == scope.ServerId && x.UserId == targetId, cancellationToken);
        if (actor is null || target is null ||
            !Contains(actor.Permissions, ServerPrivileges.ManageRoles) || !Outranks(actor.TopRole, target.Role)) return false;

        var effective = place.Evaluate(snapshot, actor);
        if (!Contains(effective, ServerPrivileges.ManageRoles)) return false;
        var current = snapshot.MemberOverride(place.TargetType, scope.ResourceId, targetId);
        return Contains(actor.Permissions & effective, allow | deny | current.Touched);
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
        var forbidden = new MemberAuthorization(null, null, AuthorizationFailure.Forbidden);
        if (actorId == targetId || await PlaceAsync(db, scope, cancellationToken) is not { } place) return forbidden;
        var snapshot = await PermissionSnapshot.LoadAsync(db, scope.ServerId, [actorId, targetId], place.Targets, cancellationToken);
        if (snapshot.Subject(actorId) is not { } actor || !Contains(place.Evaluate(snapshot, actor), required))
            return forbidden;
        if (snapshot.Subject(targetId) is not { } target) return new(null, null, AuthorizationFailure.NotFound);
        return Outranks(actor.TopRole, target.TopRole)
            ? new(actor.Member, target.Member, AuthorizationFailure.None) : forbidden;
    }

    /// <summary>The member with what they hold, server-wide; null for anybody who is not an enabled member.</summary>
    private static async Task<PermissionSubject?> SubjectAsync(
        AppDbContext db, Guid userId, Guid serverId, CancellationToken cancellationToken) =>
        (await PermissionSnapshot.LoadAsync(db, serverId, [userId], PermissionTargets.None, cancellationToken))
            .Subject(userId);

    private static Task<bool> VoiceChannelAsync(AppDbContext db, Guid serverId, Guid channelId, CancellationToken cancellationToken) =>
        db.Channels.AsNoTracking().AnyAsync(x => x.ServerId == serverId && x.Id == channelId && x.Kind == ChannelKind.Voice,
            cancellationToken);

    /// <summary>
    /// Rank is the top role. The owner role outranks every other and is
    /// outranked by none; otherwise a lower position ranks higher, and an
    /// equal position does not outrank.
    /// </summary>
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

    /// <summary>
    /// Where a scope is evaluated: the section and the channel whose overrides
    /// apply, from the outside in, and which of them an override at the scope
    /// itself targets.
    /// </summary>
    private sealed record Place(PermissionTargetType TargetType, Guid? SectionId, Guid? ChannelId)
    {
        public PermissionTargets Targets => PermissionTargets.At(SectionId, ChannelId);

        public ServerPrivileges Evaluate(PermissionSnapshot snapshot, PermissionSubject subject) =>
            snapshot.Evaluate(subject, SectionId, ChannelId);
    }

    /// <summary>The place of a scope, or null for a channel or section that is not on its server.</summary>
    private static async Task<Place?> PlaceAsync(AppDbContext db, PermissionScope scope, CancellationToken cancellationToken)
    {
        if (scope.ServerId == Guid.Empty || scope.ResourceId == Guid.Empty) return null;
        switch (scope.Kind)
        {
            case PermissionScope.ScopeKind.Server:
                return new Place(default, null, null);
            case PermissionScope.ScopeKind.Channel:
                var channel = await db.Channels.AsNoTracking()
                    .Where(x => x.Id == scope.ResourceId && x.ServerId == scope.ServerId)
                    .Select(x => new { x.SectionId }).SingleOrDefaultAsync(cancellationToken);
                return channel is null ? null : new Place(PermissionTargetType.Channel, channel.SectionId, scope.ResourceId);
            default:
                return await db.Sections.AnyAsync(x => x.Id == scope.ResourceId && x.ServerId == scope.ServerId, cancellationToken)
                    ? new Place(PermissionTargetType.Section, scope.ResourceId, null)
                    : null;
        }
    }
}
