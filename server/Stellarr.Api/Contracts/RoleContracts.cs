using Stellarr.Api.Auth;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Contracts;

public sealed record CreateRoleRequest(string? Name, string? Color, IReadOnlyList<string>? Permissions);

/// <summary>Every field is optional; a field left out keeps its value.</summary>
public sealed record UpdateRoleRequest(string? Name, string? Color, IReadOnlyList<string>? Permissions);

/// <summary>Gives the member exactly this role; the base role's id leaves them only the base role.</summary>
public sealed record AssignRoleRequest(Guid RoleId);

/// <summary>Every role the member is to hold besides the base role; an empty list leaves only the base role.</summary>
public sealed record SetMemberRolesRequest(IReadOnlyList<Guid>? RoleIds);

/// <summary>A role of the server; <c>MemberCount</c> counts who holds it, and for the base role every member.</summary>
public sealed record ServerRoleDto(
    string Id,
    string Name,
    string Color,
    IReadOnlyList<string> Permissions,
    int MemberCount,
    bool IsSystem,
    bool IsOwner);

/// <summary>
/// A member of the server. <c>RoleId</c> is the role that ranks them, the
/// highest they hold or the base role; <c>RoleIds</c> every role they hold
/// besides the base role, highest first.
/// </summary>
public sealed record ServerMemberDto(UserDto User, string RoleId, DateTime JoinedAt, IReadOnlyList<string> RoleIds);

/// <summary>What a client learns when the roles of a signed-in member, or what one of them grants, change.</summary>
public sealed record PermissionsChangedDto(string ServerId, IReadOnlyList<string> Permissions);

public static class RoleContractMappings
{
    public static ServerRoleDto ToDto(this ServerRole role, int memberCount) => new(
        role.Id.ToString(),
        role.Name,
        role.Color,
        ServerAccess.ToNames(role.Permissions),
        memberCount,
        role.IsSystem,
        role.IsOwner);
}
