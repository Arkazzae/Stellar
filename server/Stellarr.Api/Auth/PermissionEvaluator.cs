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
/// <param name="AtChannel">
/// Whether the place is a channel, where somebody without
/// <see cref="ServerPrivileges.ViewChannel"/> holds nothing at all.
/// </param>
public sealed record PermissionInput(
    bool IsOwner,
    ServerPrivileges BasePermissions,
    IReadOnlyList<ServerPrivileges> HeldPermissions,
    IReadOnlyList<OverrideLevel> Levels,
    bool AtChannel);

/// <summary>
/// What a member may do, worked out the way Discord does it. The owner may do
/// everything, and so may anybody whose roles grant Administrator, wherever
/// they are. Anybody else starts from the base role's permissions united
/// with those of every role they hold; each level then removes the base
/// role's denies and adds its allows, then removes what any held role denies
/// and adds what any held role allows, so that an allow of one role beats a
/// deny of another, and finally applies the member's own override. In a
/// channel, whoever ends up without ViewChannel holds nothing: the channel
/// does not exist for them, so neither does anything it would let them do.
/// Nothing here reads the database: callers load the inputs, and every
/// decision of the policy comes through here.
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
        // Administrator comes from the roles alone and passes every override,
        // as on Discord; what rank allows is decided elsewhere and still holds.
        if (held.HasFlag(ServerPrivileges.Administrator))
        {
            return ServerPrivileges.All;
        }

        foreach (var level in input.Levels)
        {
            held = Apply(held, level);
        }

        // The one place the rule lives. A section is not held to it: who may
        // manage a section is asked there, whether or not they see a channel
        // in it, and its overrides reach its channels through the levels.
        return input.AtChannel && !held.HasFlag(ServerPrivileges.ViewChannel) ? ServerPrivileges.None : held;
    }

    /// <summary>
    /// What the member may do server-wide, before any override: the same
    /// evaluation with no levels, for the callers that need nothing more.
    /// </summary>
    public static ServerPrivileges ServerWide(
        bool isOwner, ServerPrivileges basePermissions, IEnumerable<ServerPrivileges> heldPermissions) =>
        Evaluate(new PermissionInput(isOwner, basePermissions, heldPermissions.ToArray(), [], AtChannel: false));

    private static ServerPrivileges Unite(ServerPrivileges basePermissions, IEnumerable<ServerPrivileges> heldPermissions) =>
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

        held = (held & ~(denied & Overridable)) | (allowed & Overridable);
        return Apply(held, level.Member);
    }

    /// <summary>
    /// What an override can change: everything but Administrator, which the
    /// API refuses to store in one. A row written by hand that has it is read
    /// as if it had not.
    /// </summary>
    private const ServerPrivileges Overridable = ServerPrivileges.All & ~ServerPrivileges.Administrator;

    private static ServerPrivileges Apply(ServerPrivileges held, OverrideMasks masks) =>
        (held & ~(masks.Deny & Overridable)) | (masks.Granted & Overridable);
}
