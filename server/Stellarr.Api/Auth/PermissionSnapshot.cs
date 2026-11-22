using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Auth;

/// <summary>
/// A member as the policy sees them. Their top role ranks them; the base role
/// and the roles they hold give them their permissions.
/// </summary>
public sealed record PermissionSubject(ServerMember Member, ServerRole? BaseRole, IReadOnlyList<ServerRole> HeldRoles)
{
    public Guid UserId => Member.UserId;

    /// <summary>The role that ranks the member, see <see cref="ServerMember.RoleId"/>.</summary>
    public ServerRole TopRole => Member.Role;

    public bool IsOwner => Member.Role.IsOwner;

    /// <summary>The member's server-wide permissions, before any override.</summary>
    public ServerPrivileges Permissions => IsOwner
        ? ServerPrivileges.All
        : PermissionEvaluator.Unite(BaseRole?.Permissions ?? ServerPrivileges.None, HeldRoles.Select(x => x.Permissions));
}

/// <summary>The sections and channels whose overrides a <see cref="PermissionSnapshot"/> reads.</summary>
public sealed record PermissionTargets(IReadOnlyCollection<Guid> SectionIds, IReadOnlyCollection<Guid> ChannelIds)
{
    public static readonly PermissionTargets None = new([], []);

    public static PermissionTargets At(Guid? sectionId, Guid? channelId) => new(
        sectionId is { } section ? [section] : [],
        channelId is { } channel ? [channel] : []);

    public bool IsEmpty => SectionIds.Count == 0 && ChannelIds.Count == 0;
}

/// <summary>
/// What <see cref="PermissionEvaluator"/> reads for one server, loaded in a
/// handful of queries: the server's roles, the members asked for with the
/// roles they hold, and the role and member overrides of the sections and
/// channels asked for. Any of those members can then be evaluated at any of
/// those places without another query, so deciding for one member in many
/// channels, or for many members in one channel, costs the same few queries.
/// </summary>
public sealed class PermissionSnapshot
{
    private readonly Dictionary<Guid, PermissionSubject> _subjects;
    private readonly Dictionary<(PermissionTargetType, Guid, Guid), OverrideMasks> _roleOverrides;
    private readonly Dictionary<(PermissionTargetType, Guid, Guid), OverrideMasks> _memberOverrides;
    private readonly PermissionTargets _targets;

    private PermissionSnapshot(
        Guid serverId,
        IReadOnlyList<ServerRole> roles,
        ServerRole? baseRole,
        Dictionary<Guid, PermissionSubject> subjects,
        Dictionary<(PermissionTargetType, Guid, Guid), OverrideMasks> roleOverrides,
        Dictionary<(PermissionTargetType, Guid, Guid), OverrideMasks> memberOverrides,
        PermissionTargets targets)
    {
        ServerId = serverId;
        Roles = roles;
        BaseRole = baseRole;
        _subjects = subjects;
        _roleOverrides = roleOverrides;
        _memberOverrides = memberOverrides;
        _targets = targets;
    }

    public Guid ServerId { get; }

    /// <summary>Every role of the server, highest first.</summary>
    public IReadOnlyList<ServerRole> Roles { get; }

    /// <summary>The role every member holds without a row; null on a server that lost it.</summary>
    public ServerRole? BaseRole { get; }

    /// <summary>The members that were loaded, without disabled accounts.</summary>
    public IEnumerable<PermissionSubject> Subjects => _subjects.Values;

    /// <summary>
    /// Loads the server's roles, the members named by <paramref name="userIds"/>
    /// (every member when null) and the overrides at <paramref name="targets"/>.
    /// A disabled account is left out, as if it were no member.
    /// </summary>
    public static async Task<PermissionSnapshot> LoadAsync(
        AppDbContext db,
        Guid serverId,
        IReadOnlyCollection<Guid>? userIds,
        PermissionTargets targets,
        CancellationToken cancellationToken)
    {
        var (serverRoles, baseRole) = await HeldRoles.ServerRolesAsync(db, serverId, cancellationToken);
        var roles = serverRoles.ToDictionary(x => x.Id);

        var members = db.ServerMembers.AsNoTracking().Include(x => x.Role).Include(x => x.User)
            .Where(x => x.ServerId == serverId && !x.User.IsDisabled);
        var rows = db.ServerMemberRoles.AsNoTracking().Where(x => x.ServerId == serverId);
        if (userIds is not null)
        {
            members = members.Where(x => userIds.Contains(x.UserId));
            rows = rows.Where(x => userIds.Contains(x.UserId));
        }

        var loaded = await members.ToListAsync(cancellationToken);
        var held = (await rows.Select(x => new { x.UserId, x.RoleId }).ToListAsync(cancellationToken))
            .ToLookup(x => x.UserId, x => x.RoleId);
        var subjects = loaded.ToDictionary(
            x => x.UserId,
            x => new PermissionSubject(x, baseRole, HeldRoles.Of(x, held[x.UserId], roles, baseRole)));

        var roleOverrides = new Dictionary<(PermissionTargetType, Guid, Guid), OverrideMasks>();
        var memberOverrides = new Dictionary<(PermissionTargetType, Guid, Guid), OverrideMasks>();
        if (!targets.IsEmpty)
        {
            var sections = targets.SectionIds;
            var channels = targets.ChannelIds;
            foreach (var row in await db.PermissionOverrides.AsNoTracking()
                         .Where(x => x.ServerId == serverId &&
                             ((x.TargetType == PermissionTargetType.Section && sections.Contains(x.TargetId)) ||
                              (x.TargetType == PermissionTargetType.Channel && channels.Contains(x.TargetId))))
                         .ToListAsync(cancellationToken))
            {
                roleOverrides[(row.TargetType, row.TargetId, row.RoleId)] = OverrideMasks.Of(row);
            }

            var memberRows = db.MemberPermissionOverrides.AsNoTracking()
                .Where(x => x.ServerId == serverId &&
                    ((x.TargetType == PermissionTargetType.Section && sections.Contains(x.TargetId)) ||
                     (x.TargetType == PermissionTargetType.Channel && channels.Contains(x.TargetId))));
            if (userIds is not null)
            {
                memberRows = memberRows.Where(x => userIds.Contains(x.UserId));
            }

            foreach (var row in await memberRows.ToListAsync(cancellationToken))
            {
                memberOverrides[(row.TargetType, row.TargetId, row.UserId)] = OverrideMasks.Of(row);
            }
        }

        return new PermissionSnapshot(
            serverId, HeldRoles.Ranked(serverRoles).ToArray(), baseRole, subjects, roleOverrides, memberOverrides, targets);
    }

    public PermissionSubject? Subject(Guid userId) => _subjects.GetValueOrDefault(userId);

    /// <summary>
    /// What the member may do in the channel or section named, or server-wide
    /// when neither is. A channel in a section names both.
    /// </summary>
    public ServerPrivileges Evaluate(PermissionSubject subject, Guid? sectionId, Guid? channelId)
    {
        var levels = new List<OverrideLevel>(2);
        if (sectionId is { } section)
        {
            levels.Add(Level(subject, PermissionTargetType.Section, section));
        }

        if (channelId is { } channel)
        {
            levels.Add(Level(subject, PermissionTargetType.Channel, channel));
        }

        return PermissionEvaluator.Evaluate(new PermissionInput(
            subject.IsOwner,
            subject.BaseRole?.Permissions ?? ServerPrivileges.None,
            subject.HeldRoles.Select(x => x.Permissions).ToArray(),
            levels));
    }

    /// <summary>The overrides that concern the member at one section or channel.</summary>
    public OverrideLevel Level(PermissionSubject subject, PermissionTargetType type, Guid targetId) => new(
        subject.BaseRole is { } baseRole ? RoleOverride(type, targetId, baseRole.Id) : default,
        subject.HeldRoles.Select(x => RoleOverride(type, targetId, x.Id)).ToArray(),
        MemberOverride(type, targetId, subject.UserId));

    /// <summary>A role's override at a place this snapshot loaded; none is <c>default</c>.</summary>
    public OverrideMasks RoleOverride(PermissionTargetType type, Guid targetId, Guid roleId)
    {
        EnsureLoaded(type, targetId);
        return _roleOverrides.GetValueOrDefault((type, targetId, roleId));
    }

    /// <summary>A member's own override at a place this snapshot loaded, for a member it loaded.</summary>
    public OverrideMasks MemberOverride(PermissionTargetType type, Guid targetId, Guid userId)
    {
        EnsureLoaded(type, targetId);
        return _memberOverrides.GetValueOrDefault((type, targetId, userId));
    }

    /// <summary>
    /// An override that was never read would look like no override, which
    /// reads as a grant wherever it would have denied.
    /// </summary>
    private void EnsureLoaded(PermissionTargetType type, Guid targetId)
    {
        var loaded = type == PermissionTargetType.Section ? _targets.SectionIds : _targets.ChannelIds;
        if (!loaded.Contains(targetId))
        {
            throw new InvalidOperationException($"The overrides of {type} {targetId} were not loaded.");
        }
    }
}
