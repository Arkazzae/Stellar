using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Auth;

public sealed record ChannelMoveAuthorization(bool Allowed, bool ChangesPermissions);

public static partial class ServerAuthorization
{
    /// <summary>
    /// Moving a channel is also a permission change when its section changes
    /// what a role or a member may do in it. Such a move obeys the same
    /// delegation boundaries as editing the overrides directly.
    /// </summary>
    /// <remarks>
    /// Evaluation works bit by bit, so a move can change a bit for somebody
    /// only where the section overrides of one of their roles, of the base
    /// role or of their own differ between the two sections, and the channel
    /// override of that same role or member does not decide the bit anyway.
    /// The check asks that of every role, whether anybody holds it yet or not,
    /// and of every member with an override in either section.
    /// </remarks>
    public static async Task<ChannelMoveAuthorization> MoveChannelAsync(
        AppDbContext db, Guid actorId, Guid serverId, Guid channelId, Guid? destinationSectionId,
        CancellationToken cancellationToken)
    {
        var channel = await db.Channels.AsNoTracking()
            .SingleOrDefaultAsync(x => x.Id == channelId && x.ServerId == serverId, cancellationToken);
        if (channel is null || (destinationSectionId is { } destination &&
                !await db.Sections.AnyAsync(x => x.Id == destination && x.ServerId == serverId, cancellationToken)))
            return new(false, false);

        var sections = new[] { channel.SectionId, destinationSectionId }.OfType<Guid>().Distinct().ToArray();
        var snapshot = await PermissionSnapshot.LoadAsync(
            db, serverId, [actorId], new PermissionTargets(sections, [channelId]), cancellationToken);
        if (snapshot.Subject(actorId) is not { } actor ||
            !Contains(snapshot.Evaluate(actor, destinationSectionId, null), ServerPrivileges.ManageChannels))
            return new(false, false);

        var sourceHeld = snapshot.Evaluate(actor, channel.SectionId, channelId);
        if (!Contains(sourceHeld, ServerPrivileges.ManageChannels)) return new(false, false);
        var destinationHeld = snapshot.Evaluate(actor, destinationSectionId, channelId);
        var ceiling = actor.Permissions & sourceHeld & destinationHeld;

        OverrideMasks RoleAt(Guid? section, Guid roleId) =>
            section is { } id ? snapshot.RoleOverride(PermissionTargetType.Section, id, roleId) : default;
        var changesPermissions = false;
        // The owner role is held by the owner alone, who skips every override.
        foreach (var role in snapshot.Roles.Where(x => !x.IsOwner))
        {
            var changed = Changed(RoleAt(channel.SectionId, role.Id), RoleAt(destinationSectionId, role.Id),
                snapshot.RoleOverride(PermissionTargetType.Channel, channelId, role.Id));
            if (changed == ServerPrivileges.None) continue;
            changesPermissions = true;
            if (!Contains(ceiling, ServerPrivileges.ManageRoles | changed) || !Outranks(actor.TopRole, role))
                return new(false, true);
        }

        var members = await db.MemberPermissionOverrides.AsNoTracking()
            .Where(x => x.ServerId == serverId &&
                ((x.TargetType == PermissionTargetType.Section && sections.Contains(x.TargetId)) ||
                 (x.TargetType == PermissionTargetType.Channel && x.TargetId == channelId)))
            .ToListAsync(cancellationToken);
        if (members.Count == 0) return new(true, changesPermissions);
        OverrideMasks MemberAt(PermissionTargetType type, Guid? target, Guid userId) =>
            OverrideMasks.Of(members.FirstOrDefault(x => x.TargetType == type && x.TargetId == target && x.UserId == userId));
        var userIds = members.Select(x => x.UserId).Distinct().ToArray();
        var ranks = await db.ServerMembers.AsNoTracking().Include(x => x.Role)
            .Where(x => x.ServerId == serverId && userIds.Contains(x.UserId))
            .ToListAsync(cancellationToken);
        foreach (var member in ranks.Where(x => !x.Role.IsOwner))
        {
            var changed = Changed(
                MemberAt(PermissionTargetType.Section, channel.SectionId, member.UserId),
                MemberAt(PermissionTargetType.Section, destinationSectionId, member.UserId),
                MemberAt(PermissionTargetType.Channel, channelId, member.UserId));
            if (changed == ServerPrivileges.None) continue;
            changesPermissions = true;
            if (!Contains(ceiling, ServerPrivileges.ManageRoles | changed) ||
                member.UserId == actorId || !Outranks(actor.TopRole, member.Role))
                return new(false, true);
        }

        return new(true, changesPermissions);
    }

    /// <summary>
    /// The bits one role's or one member's section override decides
    /// differently in the two sections, less those its channel override
    /// decides either way.
    /// </summary>
    private static ServerPrivileges Changed(OverrideMasks source, OverrideMasks destination, OverrideMasks channel) =>
        ((source.Granted ^ destination.Granted) | (source.Deny ^ destination.Deny)) & ~channel.Touched;
}
