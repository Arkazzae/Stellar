using Stellarr.Api.Domain;

namespace Stellarr.Api.Auth;

/// <summary>What one override says at one level: the bits it allows and the bits it denies.</summary>
public readonly record struct OverrideMasks(ServerPrivileges Allow, ServerPrivileges Deny)
{
    /// <summary>
    /// The bits the override really grants. The API never stores a bit as both
    /// allowed and denied, but a row written by hand may, and inside one
    /// override such a bit stays denied.
    /// </summary>
    public ServerPrivileges Granted => Allow & ~Deny;

    /// <summary>Every bit the override decides, either way.</summary>
    public ServerPrivileges Touched => Allow | Deny;

    public static OverrideMasks Of(PermissionOverride? row) =>
        row is null ? default : new OverrideMasks(row.AllowMask, row.DenyMask);

    public static OverrideMasks Of(MemberPermissionOverride? row) =>
        row is null ? default : new OverrideMasks(row.AllowMask, row.DenyMask);
}

/// <summary>
/// The overrides that concern one member at one level, a section or a
/// channel: the base role's, those of the roles the member holds, and the
/// member's own. An override that does not exist is <c>default</c>, which
/// changes nothing.
/// </summary>
public sealed record OverrideLevel(OverrideMasks Base, IReadOnlyList<OverrideMasks> HeldRoles, OverrideMasks Member)
{
    public static readonly OverrideLevel None = new(default, [], default);
}

/// <summary>
/// Everything the evaluation of one member at one place reads. The levels run
/// from the outside in: a channel in a section has the section's and then the
/// channel's, a channel outside any section only its own, a section its own,
/// and the server none.
/// </summary>
public sealed record PermissionInput(
    bool IsOwner,
    ServerPrivileges BasePermissions,
    IReadOnlyList<ServerPrivileges> HeldPermissions,
    IReadOnlyList<OverrideLevel> Levels);

/// <summary>
/// What a member may do, worked out the way Discord does it. The owner may do
/// everything. Anybody else starts from the base role's permissions united
/// with those of every role they hold; each level then removes the base
/// role's denies and adds its allows, then removes what any held role denies
/// and adds what any held role allows, so that an allow of one role beats a
/// deny of another, and finally applies the member's own override. Nothing
/// here reads the database: callers load the inputs, and every decision of
/// the policy comes through here.
/// </summary>
public static class PermissionEvaluator
{
    public static ServerPrivileges Evaluate(PermissionInput input)
    {
        if (input.IsOwner)
        {
            return ServerPrivileges.All;
        }

        var held = Unite(input.BasePermissions, input.HeldPermissions);
        foreach (var level in input.Levels)
        {
            held = Apply(held, level);
        }

        return held;
    }

    /// <summary>The server-wide permissions of a member who is not the owner.</summary>
    public static ServerPrivileges Unite(ServerPrivileges basePermissions, IEnumerable<ServerPrivileges> heldPermissions) =>
        heldPermissions.Aggregate(basePermissions, (all, one) => all | one);

    /// <summary>One level applied to what the member holds from the levels outside it.</summary>
    public static ServerPrivileges Apply(ServerPrivileges held, OverrideLevel level)
    {
        held = Apply(held, level.Base);
        var denied = ServerPrivileges.None;
        var allowed = ServerPrivileges.None;
        foreach (var role in level.HeldRoles)
        {
            denied |= role.Deny;
            allowed |= role.Granted;
        }

        held = (held & ~denied) | allowed;
        return Apply(held, level.Member);
    }

    private static ServerPrivileges Apply(ServerPrivileges held, OverrideMasks masks) =>
        (held & ~masks.Deny) | masks.Granted;
}
