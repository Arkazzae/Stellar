using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Auth;

public sealed record ChannelMoveAuthorization(bool Allowed, bool ChangesPermissions);

public static partial class ServerAuthorization
{
    /// <summary>
    /// Moving a channel is also a permission change when its section changes
    /// anybody's effective rights. Such a move obeys the same delegation
    /// boundaries as editing the overrides directly.
    /// </summary>
    public static async Task<ChannelMoveAuthorization> MoveChannelAsync(
        AppDbContext db, Guid actorId, Guid serverId, Guid channelId, Guid? destinationSectionId,
        CancellationToken cancellationToken)
    {
        var actor = await MemberAsync(db, actorId, serverId, cancellationToken);
        var channel = await db.Channels.AsNoTracking()
            .SingleOrDefaultAsync(x => x.Id == channelId && x.ServerId == serverId, cancellationToken);
        if (actor is null || channel is null) return new(false, false);

        var destination = destinationSectionId is { } section
            ? PermissionScope.Section(serverId, section) : PermissionScope.Server(serverId);
        if (await EffectiveAsync(db, actor.Role, destination, cancellationToken) is not { } destinationScopeHeld ||
            !Contains(destinationScopeHeld, ServerPrivileges.ManageChannels)) return new(false, false);

        var overrides = await db.PermissionOverrides.AsNoTracking().Where(x => x.ServerId == serverId &&
            ((x.TargetType == PermissionTargetType.Channel && x.TargetId == channelId) ||
             (x.TargetType == PermissionTargetType.Section &&
              (x.TargetId == channel.SectionId || x.TargetId == destinationSectionId))))
            .ToListAsync(cancellationToken);
        var sourceHeld = InheritedChannelPermissions(actor.Role, channel.SectionId, channelId, overrides);
        if (!Contains(sourceHeld, ServerPrivileges.ManageChannels)) return new(false, false);
        var destinationHeld = InheritedChannelPermissions(actor.Role, destinationSectionId, channelId, overrides);
        var roles = await db.ServerRoles.AsNoTracking().Where(x => x.ServerId == serverId).ToListAsync(cancellationToken);

        var changesPermissions = false;
        foreach (var role in roles)
        {
            var before = InheritedChannelPermissions(role, channel.SectionId, channelId, overrides);
            var after = InheritedChannelPermissions(role, destinationSectionId, channelId, overrides);
            var changed = before ^ after;
            if (changed == ServerPrivileges.None) continue;
            changesPermissions = true;

            if (!Contains(actor.Role.Permissions & sourceHeld & destinationHeld, ServerPrivileges.ManageRoles | changed) ||
                (!(actor.Role.IsOwner && role.Id == actor.Role.Id) && !Outranks(actor.Role, role))) return new(false, true);
        }
        return new(true, changesPermissions);
    }

    private static ServerPrivileges InheritedChannelPermissions(
        ServerRole role, Guid? sectionId, Guid channelId, IReadOnlyList<PermissionOverride> overrides)
    {
        var held = role.Permissions;
        if (sectionId is { } section && overrides.FirstOrDefault(x => x.RoleId == role.Id &&
                x.TargetType == PermissionTargetType.Section && x.TargetId == section) is { } inherited)
            held = (held | inherited.AllowMask) & ~inherited.DenyMask;
        if (overrides.FirstOrDefault(x => x.RoleId == role.Id &&
                x.TargetType == PermissionTargetType.Channel && x.TargetId == channelId) is { } local)
            held = (held | local.AllowMask) & ~local.DenyMask;
        return held;
    }
}
