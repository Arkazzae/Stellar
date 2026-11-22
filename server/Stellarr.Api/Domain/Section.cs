namespace Stellarr.Api.Domain;

/// <summary>A named, collapsible group of channels in the rail, ordered within its server.</summary>
public sealed class Section
{
    public Guid Id { get; set; }
    public Guid ServerId { get; set; }
    public required string Name { get; set; }
    public int Position { get; set; }
}

/// <summary>What a <see cref="PermissionOverride"/> narrows: one channel or one section.</summary>
public enum PermissionTargetType
{
    Channel = 0,
    Section = 1,
}

/// <summary>
/// A per-role allow and deny for one channel or one section, on top of what
/// the member's roles grant. A channel's overrides apply after those of its
/// section. Within one level the base role's override comes first, then the
/// overrides of the roles a member holds, where an allow of one role beats a
/// deny of another, then the member's own; see <c>PermissionEvaluator</c>.
/// </summary>
public sealed class PermissionOverride
{
    public Guid Id { get; set; }
    public Guid ServerId { get; set; }
    public PermissionTargetType TargetType { get; set; }

    /// <summary>The channel or section id, as <see cref="TargetType"/> says; not a foreign key.</summary>
    public Guid TargetId { get; set; }

    public Guid RoleId { get; set; }
    public ServerPrivileges AllowMask { get; set; }
    public ServerPrivileges DenyMask { get; set; }
}

/// <summary>
/// A per-member allow and deny for one channel or one section, applied after
/// the overrides of the member's roles at the same level. It belongs to the
/// membership and goes with it, like the roles the member holds.
/// </summary>
public sealed class MemberPermissionOverride
{
    public Guid Id { get; set; }
    public Guid ServerId { get; set; }
    public PermissionTargetType TargetType { get; set; }

    /// <summary>The channel or section id, as <see cref="TargetType"/> says; not a foreign key.</summary>
    public Guid TargetId { get; set; }

    public Guid UserId { get; set; }
    public ServerPrivileges AllowMask { get; set; }
    public ServerPrivileges DenyMask { get; set; }
}
