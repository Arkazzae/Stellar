using System.Linq.Expressions;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Auth;

/// <summary>A channel as the visibility checks see it.</summary>
/// <param name="Open">
/// Nobody is denied view there: the base role holds it and no override of a
/// role or a member, at the channel or at its section, denies it. Everybody on
/// the server then sees the channel, and nothing has to be evaluated.
/// </param>
/// <param name="Member">
/// Whether the person the place was looked up for is an enabled member of its
/// server; true when it was looked up for nobody in particular.
/// </param>
public sealed record ChannelPlace(
    Guid Id, Guid ServerId, Guid? SectionId, ChannelKind Kind, string Name, int SlowModeSeconds, bool Open, bool Member);

/// <summary>
/// Who sees which channel. A channel somebody does not view does not exist for
/// them, so every route and hub method about a channel, or about anything in
/// one, asks here first and answers a non-viewer as it answers for an id that
/// does not exist.
/// </summary>
/// <remarks>
/// Each question starts with the cheap one, whether anybody is denied view at
/// all, which one query answers. Only where somebody is does the evaluator
/// run, through one <see cref="PermissionSnapshot"/> for the whole question.
/// Nothing is cached: a change of roles or overrides counts from the next
/// request or event on.
/// </remarks>
public static class ChannelVisibility
{
    /// <summary>
    /// Whether nobody is denied view at a channel, as one condition every
    /// query that needs it shares. An override at a section another channel
    /// is in does not count against this one.
    /// </summary>
    private static Expression<Func<Channel, bool>> IsOpen(AppDbContext db) => channel =>
        // The base role, picked as ServerAccess.FindMemberRoleAsync picks it.
        db.ServerRoles
            .Where(role => role.ServerId == channel.ServerId && role.IsSystem && !role.IsOwner)
            .OrderByDescending(role => role.Position).ThenBy(role => role.Id)
            .Select(role => (role.Permissions & ServerPrivileges.ViewChannel) != ServerPrivileges.None)
            .FirstOrDefault() &&
        !db.PermissionOverrides.Any(row =>
            row.ServerId == channel.ServerId &&
            (row.DenyMask & ServerPrivileges.ViewChannel) != ServerPrivileges.None &&
            ((row.TargetType == PermissionTargetType.Channel && row.TargetId == channel.Id) ||
             (row.TargetType == PermissionTargetType.Section && row.TargetId == channel.SectionId))) &&
        !db.MemberPermissionOverrides.Any(row =>
            row.ServerId == channel.ServerId &&
            (row.DenyMask & ServerPrivileges.ViewChannel) != ServerPrivileges.None &&
            ((row.TargetType == PermissionTargetType.Channel && row.TargetId == channel.Id) ||
             (row.TargetType == PermissionTargetType.Section && row.TargetId == channel.SectionId)));

    /// <summary>The channels nobody is denied view at, to be used inside a larger query.</summary>
    public static IQueryable<Channel> OpenChannels(AppDbContext db) => db.Channels.Where(IsOpen(db));

    /// <summary>
    /// The channels of <paramref name="channels"/> with whether each is open,
    /// and whether <paramref name="viewer"/> is an enabled member of its
    /// server, in one query.
    /// </summary>
    public static IQueryable<ChannelPlace> Places(AppDbContext db, IQueryable<Channel> channels, Guid? viewer = null)
    {
        var open = OpenChannels(db);
        return channels.Select(channel => new ChannelPlace(
            channel.Id,
            channel.ServerId,
            channel.SectionId,
            channel.Kind,
            channel.Name,
            channel.SlowModeSeconds,
            open.Any(x => x.Id == channel.Id),
            viewer == null || db.ServerMembers.Any(member =>
                member.ServerId == channel.ServerId && member.UserId == viewer && !member.User.IsDisabled)));
    }

    /// <summary>
    /// Whether nobody is denied view anywhere on the server: the base role
    /// holds it and no override of a channel or section that exists denies it.
    /// Every channel is then open, and every member sees every channel and
    /// every section.
    /// </summary>
    /// <remarks>
    /// An override can outlive its channel, which is deleted without it; such
    /// a row denies nothing and is not counted.
    /// </remarks>
    public static async Task<bool> ServerOpenAsync(AppDbContext db, Guid serverId, CancellationToken cancellationToken) =>
        await db.Servers.AsNoTracking()
            .Where(server => server.Id == serverId)
            .Select(server =>
                db.ServerRoles
                    .Where(role => role.ServerId == server.Id && role.IsSystem && !role.IsOwner)
                    .OrderByDescending(role => role.Position).ThenBy(role => role.Id)
                    .Select(role => (role.Permissions & ServerPrivileges.ViewChannel) != ServerPrivileges.None)
                    .FirstOrDefault() &&
                !db.PermissionOverrides.Any(row =>
                    row.ServerId == server.Id &&
                    (row.DenyMask & ServerPrivileges.ViewChannel) != ServerPrivileges.None &&
                    ((row.TargetType == PermissionTargetType.Channel &&
                        db.Channels.Any(channel => channel.Id == row.TargetId && channel.ServerId == server.Id)) ||
                     (row.TargetType == PermissionTargetType.Section &&
                        db.Sections.Any(section => section.Id == row.TargetId && section.ServerId == server.Id)))) &&
                !db.MemberPermissionOverrides.Any(row =>
                    row.ServerId == server.Id &&
                    (row.DenyMask & ServerPrivileges.ViewChannel) != ServerPrivileges.None &&
                    ((row.TargetType == PermissionTargetType.Channel &&
                        db.Channels.Any(channel => channel.Id == row.TargetId && channel.ServerId == server.Id)) ||
                     (row.TargetType == PermissionTargetType.Section &&
                        db.Sections.Any(section => section.Id == row.TargetId && section.ServerId == server.Id)))))
            .SingleOrDefaultAsync(cancellationToken);

    /// <summary>
    /// The channel of the server when <paramref name="userId"/>, an enabled
    /// member, sees it, and null when there is no such channel for them:
    /// unknown, of another server, of another kind, or not viewed.
    /// </summary>
    public static Task<ChannelPlace?> FindAsync(
        AppDbContext db,
        Guid userId,
        Guid serverId,
        Guid channelId,
        ChannelKind? kind,
        CancellationToken cancellationToken) =>
        FindAsync(
            db,
            userId,
            db.Channels.Where(x => x.Id == channelId && x.ServerId == serverId && (kind == null || x.Kind == kind)),
            cancellationToken);

    /// <summary>The one channel of <paramref name="channels"/> when the member sees it, else null.</summary>
    public static async Task<ChannelPlace?> FindAsync(
        AppDbContext db, Guid userId, IQueryable<Channel> channels, CancellationToken cancellationToken)
    {
        var place = await Places(db, channels.AsNoTracking(), userId).SingleOrDefaultAsync(cancellationToken);
        return place is not null && await ViewsAsync(db, userId, place, cancellationToken) ? place : null;
    }

    /// <summary>
    /// Whether the member, for whom <paramref name="place"/> was looked up,
    /// sees the channel. An open one needs nothing more than the membership.
    /// </summary>
    private static async Task<bool> ViewsAsync(
        AppDbContext db, Guid userId, ChannelPlace place, CancellationToken cancellationToken)
    {
        if (!place.Member)
        {
            return false;
        }

        if (place.Open)
        {
            return true;
        }

        var snapshot = await PermissionSnapshot.LoadAsync(
            db, place.ServerId, [userId], PermissionTargets.At(place.SectionId, place.Id), cancellationToken);
        return snapshot.Subject(userId) is { } subject && snapshot.Views(subject, place.SectionId, place.Id);
    }

    /// <summary>Whether the member sees the channel, which need not be on any server they are on.</summary>
    public static async Task<bool> ViewsAsync(
        AppDbContext db, Guid userId, Guid channelId, CancellationToken cancellationToken) =>
        await FindAsync(db, userId, db.Channels.Where(x => x.Id == channelId), cancellationToken) is not null;

    /// <summary>
    /// Whether somebody who has just joined, holding the base role alone, would
    /// see the channel: what an invite to it may name to whoever is not a
    /// member yet.
    /// </summary>
    public static async Task<bool> NewcomerViewsAsync(AppDbContext db, Guid channelId, CancellationToken cancellationToken)
    {
        var place = await Places(db, db.Channels.AsNoTracking().Where(x => x.Id == channelId)).SingleOrDefaultAsync(cancellationToken);
        if (place is null || place.Open)
        {
            return place is not null;
        }

        var snapshot = await PermissionSnapshot.LoadAsync(
            db, place.ServerId, [], PermissionTargets.At(place.SectionId, place.Id), cancellationToken);
        return snapshot.EvaluateNewcomer(place.SectionId, place.Id).HasFlag(ServerPrivileges.ViewChannel);
    }

    /// <summary>
    /// Whether the section is listed for the member, by the rule of
    /// <see cref="ServerLayout.ViewOf"/>: false for one that is not on the
    /// server, which is how a section they do not see is answered.
    /// </summary>
    public static async Task<bool> SectionListedAsync(
        AppDbContext db, Guid userId, Guid serverId, Guid sectionId, CancellationToken cancellationToken)
    {
        if (!await db.Sections.AsNoTracking().AnyAsync(x => x.Id == sectionId && x.ServerId == serverId, cancellationToken))
        {
            return false;
        }

        if (await ServerOpenAsync(db, serverId, cancellationToken))
        {
            return await db.ServerMembers.AsNoTracking().AnyAsync(
                x => x.ServerId == serverId && x.UserId == userId && !x.User.IsDisabled, cancellationToken);
        }

        var channels = await db.Channels.AsNoTracking()
            .Where(x => x.ServerId == serverId && x.SectionId == sectionId)
            .Select(x => x.Id)
            .ToListAsync(cancellationToken);
        var layout = new ServerLayout(channels.Select(id => (id, (Guid?)sectionId)).ToArray(), [sectionId]);
        var snapshot = await PermissionSnapshot.LoadAsync(db, serverId, [userId], layout.Targets, cancellationToken);
        return snapshot.Subject(userId) is { } subject && layout.ViewOf(snapshot, subject).SectionIds.Contains(sectionId);
    }

    /// <summary>
    /// Who sees the channel: null when it is open, which is everybody on the
    /// server, else the enabled members who view it. An unknown channel has
    /// nobody.
    /// </summary>
    public static async Task<IReadOnlySet<Guid>?> ViewersAsync(
        AppDbContext db, Guid serverId, Guid channelId, CancellationToken cancellationToken)
    {
        var place = await Places(db, db.Channels.AsNoTracking().Where(x => x.Id == channelId && x.ServerId == serverId))
            .SingleOrDefaultAsync(cancellationToken);
        if (place is null)
        {
            return new HashSet<Guid>();
        }

        if (place.Open)
        {
            return null;
        }

        var snapshot = await PermissionSnapshot.LoadAsync(
            db, serverId, null, PermissionTargets.At(place.SectionId, place.Id), cancellationToken);
        return snapshot.Subjects
            .Where(subject => snapshot.Views(subject, place.SectionId, place.Id))
            .Select(subject => subject.UserId)
            .ToHashSet();
    }

    /// <summary>
    /// Those of <paramref name="userIds"/>, members of the server, who see the
    /// channel. Only they are evaluated, and an open channel needs nothing.
    /// </summary>
    public static async Task<IReadOnlyCollection<Guid>> WhoViewAsync(
        AppDbContext db, Guid serverId, Guid channelId, IReadOnlyCollection<Guid> userIds, CancellationToken cancellationToken)
    {
        if (userIds.Count == 0)
        {
            return userIds;
        }

        var place = await Places(db, db.Channels.AsNoTracking().Where(x => x.Id == channelId && x.ServerId == serverId))
            .SingleOrDefaultAsync(cancellationToken);
        if (place is null || place.Open)
        {
            return place is null ? [] : userIds;
        }

        var snapshot = await PermissionSnapshot.LoadAsync(
            db, serverId, userIds, PermissionTargets.At(place.SectionId, place.Id), cancellationToken);
        return userIds
            .Where(id => snapshot.Subject(id) is { } subject && snapshot.Views(subject, place.SectionId, place.Id))
            .ToArray();
    }

    /// <summary>
    /// What one member sees of the server: null when nobody is denied view
    /// anywhere, which is everything, else the channels they view and the
    /// sections listed for them (<see cref="ServerVisibility"/>).
    /// </summary>
    public static async Task<MemberView?> OfMemberAsync(
        AppDbContext db, Guid userId, Guid serverId, CancellationToken cancellationToken)
    {
        if (await ServerOpenAsync(db, serverId, cancellationToken))
        {
            return null;
        }

        var layout = await ServerLayout.LoadAsync(db, serverId, cancellationToken);
        var snapshot = await PermissionSnapshot.LoadAsync(db, serverId, [userId], layout.Targets, cancellationToken);
        return snapshot.Subject(userId) is { } subject ? layout.ViewOf(snapshot, subject) : MemberView.Nothing;
    }
}

/// <summary>The channels one member views and the sections listed for them.</summary>
public sealed record MemberView(IReadOnlySet<Guid> ChannelIds, IReadOnlySet<Guid> SectionIds)
{
    public static readonly MemberView Nothing = new(new HashSet<Guid>(), new HashSet<Guid>());

    /// <summary>Whether two views show the same channels and sections.</summary>
    public bool SameAs(MemberView other) =>
        ChannelIds.SetEquals(other.ChannelIds) && SectionIds.SetEquals(other.SectionIds);
}

/// <summary>Every channel and section of a server, as the visibility of a whole server needs them.</summary>
internal sealed record ServerLayout(
    IReadOnlyList<(Guid Id, Guid? SectionId)> Channels,
    IReadOnlyList<Guid> SectionIds)
{
    /// <remarks>
    /// The sections channels point at are read as well as those that exist,
    /// as <see cref="ServerAuthorization"/> reads a channel's section by its
    /// id, so both decide alike.
    /// </remarks>
    public PermissionTargets Targets => new(
        SectionIds.Concat(Channels.Select(x => x.SectionId).OfType<Guid>()).Distinct().ToArray(),
        Channels.Select(x => x.Id).ToArray());

    public MemberView Everything => new(Channels.Select(x => x.Id).ToHashSet(), SectionIds.ToHashSet());

    public static async Task<ServerLayout> LoadAsync(AppDbContext db, Guid serverId, CancellationToken cancellationToken)
    {
        var channels = await db.Channels.AsNoTracking()
            .Where(x => x.ServerId == serverId)
            .Select(x => new { x.Id, x.SectionId })
            .ToListAsync(cancellationToken);
        var sections = await db.Sections.AsNoTracking()
            .Where(x => x.ServerId == serverId)
            .Select(x => x.Id)
            .ToListAsync(cancellationToken);
        return new ServerLayout(channels.Select(x => (x.Id, x.SectionId)).ToArray(), sections);
    }

    /// <summary>
    /// The channels the member views, and the sections listed for them: one
    /// with a channel they view, one where they hold ManageChannels, and an
    /// empty one where they hold ViewChannel, as every member does on a server
    /// where nobody is denied it, so that an empty section stays where
    /// everybody sees it today.
    /// </summary>
    public MemberView ViewOf(PermissionSnapshot snapshot, PermissionSubject subject)
    {
        var channels = Channels
            .Where(x => snapshot.Views(subject, x.SectionId, x.Id))
            .ToArray();
        var viewed = channels.Select(x => x.Id).ToHashSet();
        var sectionsWithViewed = channels.Select(x => x.SectionId).OfType<Guid>().ToHashSet();
        var sectionsWithAny = Channels.Select(x => x.SectionId).OfType<Guid>().ToHashSet();
        var sections = SectionIds.Where(section =>
        {
            if (sectionsWithViewed.Contains(section))
            {
                return true;
            }

            var held = snapshot.Evaluate(subject, section, null);
            return held.HasFlag(ServerPrivileges.ManageChannels) ||
                (!sectionsWithAny.Contains(section) && held.HasFlag(ServerPrivileges.ViewChannel));
        }).ToHashSet();
        return new MemberView(viewed, sections);
    }
}

/// <summary>
/// What every member of one server sees, for the events that are about the
/// whole layout and for telling people whose view a change altered.
/// </summary>
public sealed class ServerVisibility
{
    private readonly ServerLayout? _layout;
    private readonly IReadOnlyDictionary<Guid, MemberView>? _members;

    private ServerVisibility(Guid serverId, ServerLayout? layout, IReadOnlyDictionary<Guid, MemberView>? members)
    {
        ServerId = serverId;
        _layout = layout;
        _members = members;
    }

    public Guid ServerId { get; }

    /// <summary>Nobody is denied view anywhere: everybody sees everything, and nothing was evaluated.</summary>
    public bool Open => _members is null;

    /// <summary>The members who were evaluated, with what each sees; empty when <see cref="Open"/>.</summary>
    public IReadOnlyDictionary<Guid, MemberView> Members => _members ?? new Dictionary<Guid, MemberView>();

    public static async Task<ServerVisibility> LoadAsync(AppDbContext db, Guid serverId, CancellationToken cancellationToken)
    {
        if (await ServerOpenAsync(db, serverId, cancellationToken))
        {
            return new ServerVisibility(serverId, null, null);
        }

        var layout = await ServerLayout.LoadAsync(db, serverId, cancellationToken);
        var snapshot = await PermissionSnapshot.LoadAsync(db, serverId, null, layout.Targets, cancellationToken);
        return new ServerVisibility(
            serverId,
            layout,
            snapshot.Subjects.ToDictionary(x => x.UserId, x => layout.ViewOf(snapshot, x)));
    }

    private static Task<bool> ServerOpenAsync(AppDbContext db, Guid serverId, CancellationToken cancellationToken) =>
        ChannelVisibility.ServerOpenAsync(db, serverId, cancellationToken);

    /// <summary>
    /// The members whose view differs between <paramref name="before"/> and
    /// <paramref name="after"/>, the same server read on both sides of one
    /// change. Two open readings differ for nobody. An open side shows its
    /// members everything there is now, which for a change of roles,
    /// overrides or a channel's section is everything there was before too.
    /// </summary>
    public static async Task<IReadOnlyList<Guid>> ChangedAsync(
        AppDbContext db, ServerVisibility before, ServerVisibility after, CancellationToken cancellationToken)
    {
        if (before.Open && after.Open)
        {
            return [];
        }

        var everything = (before._layout ?? after._layout ?? await ServerLayout.LoadAsync(db, before.ServerId, cancellationToken))
            .Everything;
        var members = before.Open || after.Open
            ? await db.ServerMembers.AsNoTracking()
                .Where(x => x.ServerId == before.ServerId && !x.User.IsDisabled)
                .Select(x => x.UserId)
                .ToListAsync(cancellationToken)
            : [];
        MemberView ViewOf(ServerVisibility reading, Guid userId) =>
            reading.Open
                ? members.Contains(userId) ? everything : MemberView.Nothing
                : reading.Members.GetValueOrDefault(userId) ?? MemberView.Nothing;
        return members
            .Concat(before.Members.Keys)
            .Concat(after.Members.Keys)
            .Distinct()
            .Where(userId => !ViewOf(before, userId).SameAs(ViewOf(after, userId)))
            .ToArray();
    }
}
