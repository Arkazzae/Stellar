using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Auth;

/// <summary>
/// Which roles a member holds, and which of them ranks them. Every member
/// holds the base role, without a row. The others are rows of
/// <see cref="ServerMemberRole"/>, and the top role,
/// <see cref="ServerMember.RoleId"/>, counts as held too. Every writer stores
/// the top role as a row as well, so that only decides for a membership
/// written by hand, and there it keeps what ranks a member and what they may
/// do from parting.
/// </summary>
public static class HeldRoles
{
    /// <summary>
    /// Highest first: the owner role, then by position, then by id, so that
    /// roles at an equal position still come in one order.
    /// </summary>
    public static IOrderedEnumerable<ServerRole> Ranked(IEnumerable<ServerRole> roles) =>
        roles.OrderByDescending(x => x.IsOwner).ThenBy(x => x.Position).ThenBy(x => x.Id);

    /// <summary>The role that ranks a member who holds <paramref name="held"/>: the highest of them, or else the base role.</summary>
    public static ServerRole Top(IEnumerable<ServerRole> held, ServerRole baseRole) =>
        Ranked(held).FirstOrDefault() ?? baseRole;

    /// <summary>
    /// The roles a member holds besides the base role, highest first, from the
    /// ids stored for them. Ids of roles that are gone or belong to another
    /// server count for nothing.
    /// </summary>
    public static IReadOnlyList<ServerRole> Of(
        ServerMember member,
        IEnumerable<Guid> storedRoleIds,
        IReadOnlyDictionary<Guid, ServerRole> serverRoles,
        ServerRole? baseRole) =>
        Ranked(storedRoleIds.Append(member.RoleId).Distinct()
                .Where(id => id != baseRole?.Id)
                .Select(id => serverRoles.GetValueOrDefault(id))
                .OfType<ServerRole>()
                .Where(role => role.ServerId == member.ServerId))
            .ToArray();

    /// <summary>
    /// Every member of the server with the roles they hold besides the base
    /// role, highest first, disabled accounts included: the lists people see
    /// and the writers need everybody, not only those who may act.
    /// </summary>
    public static async Task<HeldRoleTable> LoadAsync(
        AppDbContext db,
        Guid serverId,
        CancellationToken cancellationToken)
    {
        var (roles, baseRole) = await ServerRolesAsync(db, serverId, cancellationToken);
        var byId = roles.ToDictionary(x => x.Id);
        var members = await db.ServerMembers.AsNoTracking()
            .Where(x => x.ServerId == serverId)
            .ToListAsync(cancellationToken);
        var rows = (await db.ServerMemberRoles.AsNoTracking()
                .Where(x => x.ServerId == serverId)
                .Select(x => new { x.UserId, x.RoleId })
                .ToListAsync(cancellationToken))
            .ToLookup(x => x.UserId, x => x.RoleId);
        return new HeldRoleTable(
            baseRole,
            members.ToDictionary(x => x.UserId, x => Of(x, rows[x.UserId], byId, baseRole)));
    }

    /// <summary>
    /// The server-wide permissions of one member, whether their account is
    /// enabled or not; <paramref name="member"/> comes with its top role.
    /// </summary>
    public static async Task<ServerPrivileges> PermissionsAsync(
        AppDbContext db,
        ServerMember member,
        CancellationToken cancellationToken)
    {
        var (held, baseRole) = await OfMemberAsync(db, member, cancellationToken);
        return PermissionEvaluator.ServerWide(
            member.Role.IsOwner, baseRole?.Permissions ?? ServerPrivileges.None, held.Select(x => x.Permissions));
    }

    /// <summary>The roles one member holds besides the base role, highest first, and the base role.</summary>
    public static async Task<(IReadOnlyList<ServerRole> Held, ServerRole? BaseRole)> OfMemberAsync(
        AppDbContext db,
        ServerMember member,
        CancellationToken cancellationToken)
    {
        var (roles, baseRole) = await ServerRolesAsync(db, member.ServerId, cancellationToken);
        var stored = await db.ServerMemberRoles.AsNoTracking()
            .Where(x => x.ServerId == member.ServerId && x.UserId == member.UserId)
            .Select(x => x.RoleId)
            .ToListAsync(cancellationToken);
        return (Of(member, stored, roles.ToDictionary(x => x.Id), baseRole), baseRole);
    }

    /// <summary>
    /// Every role of the server, highest position value first as the database
    /// orders them, and the base role among them, picked by the rule of
    /// <see cref="ServerAccess.FindMemberRoleAsync"/>, ties included.
    /// </summary>
    internal static async Task<(IReadOnlyList<ServerRole> Roles, ServerRole? BaseRole)> ServerRolesAsync(
        AppDbContext db,
        Guid serverId,
        CancellationToken cancellationToken)
    {
        var roles = await db.ServerRoles.AsNoTracking()
            .Where(x => x.ServerId == serverId)
            .OrderByDescending(x => x.Position).ThenBy(x => x.Id)
            .ToListAsync(cancellationToken);
        return (roles, roles.FirstOrDefault(x => x.IsSystem && !x.IsOwner));
    }
}

/// <summary>The roles every member of one server holds besides the base role.</summary>
public sealed record HeldRoleTable(ServerRole? BaseRole, IReadOnlyDictionary<Guid, IReadOnlyList<ServerRole>> ByMember)
{
    /// <summary>How many members hold the role; the base role every member holds.</summary>
    public int CountHolders(Guid roleId) =>
        roleId == BaseRole?.Id ? ByMember.Count : ByMember.Values.Count(held => held.Any(x => x.Id == roleId));

    /// <summary>The members who hold the role; the base role every member holds.</summary>
    public IReadOnlyList<Guid> Holders(Guid roleId) =>
        (roleId == BaseRole?.Id
            ? ByMember.Keys
            : ByMember.Where(x => x.Value.Any(role => role.Id == roleId)).Select(x => x.Key)).ToArray();

    public IReadOnlyList<ServerRole> Of(Guid userId) => ByMember.GetValueOrDefault(userId) ?? [];

    /// <summary>A member's server-wide permissions: everything for the owner, else the base role's and every held role's.</summary>
    public ServerPrivileges PermissionsOf(Guid userId)
    {
        var held = Of(userId);
        return PermissionEvaluator.ServerWide(
            held.Any(x => x.IsOwner), BaseRole?.Permissions ?? ServerPrivileges.None, held.Select(x => x.Permissions));
    }
}
