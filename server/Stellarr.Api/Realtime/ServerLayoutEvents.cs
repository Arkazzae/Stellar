using Microsoft.AspNetCore.SignalR;
using Stellarr.Api.Auth;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Realtime;

/// <summary>
/// The events about how a server's channels are laid out, which name every
/// channel or section they are about, and the one that tells somebody what
/// they see has changed.
/// </summary>
/// <remarks>
/// <para>
/// On a server where nobody is denied view anywhere, a layout event goes to
/// the server's group whole, as it always has. Otherwise each member gets the
/// layout of what they see, from one <see cref="ServerVisibility"/> read when
/// the event is sent; members who see the same get one send between them.
/// </para>
/// <para>
/// A layout only orders and files the channels a client already has, and a
/// client cannot make a channel out of an id. So when a change shows somebody
/// a channel or hides one from them, they are sent
/// <c>ChannelAccessChanged</c> instead, which carries nothing but the server,
/// and their client reads its own channels again from the bootstrap.
/// </para>
/// </remarks>
internal static class ServerLayoutEvents
{
    internal static async Task ChannelLayoutAsync(
        IHubClients<IClientProxy> clients,
        ServerVisibility visibility,
        IReadOnlyList<Channel> ordered,
        CancellationToken cancellationToken)
    {
        var serverId = visibility.ServerId;
        ChannelLayoutDto Layout(Func<Guid, bool> seen) => new(
            serverId.ToString(),
            ordered.Where(x => seen(x.Id))
                .Select(x => new ChannelPlacementDto(x.Id.ToString(), x.SectionId?.ToString()))
                .ToArray());

        if (visibility.Open)
        {
            await clients.Group(AppHub.ServerGroup(serverId))
                .SendAsync("ChannelLayoutChanged", Layout(_ => true), cancellationToken);
            return;
        }

        foreach (var alike in visibility.Members.Where(x => x.Value.ChannelIds.Count > 0).GroupBy(x => Key(x.Value.ChannelIds)))
        {
            var seen = alike.First().Value.ChannelIds;
            await clients.Groups(alike.Select(x => AppHub.FollowerGroup(serverId, x.Key)).ToArray())
                .SendAsync("ChannelLayoutChanged", Layout(seen.Contains), cancellationToken);
        }
    }

    internal static async Task SectionOrderAsync(
        IHubClients<IClientProxy> clients,
        ServerVisibility visibility,
        IReadOnlyList<Guid> ordered,
        CancellationToken cancellationToken)
    {
        var serverId = visibility.ServerId;
        SectionOrderDto Order(Func<Guid, bool> seen) =>
            new(serverId.ToString(), ordered.Where(seen).Select(x => x.ToString()).ToArray());

        if (visibility.Open)
        {
            await clients.Group(AppHub.ServerGroup(serverId))
                .SendAsync("SectionOrderChanged", Order(_ => true), cancellationToken);
            return;
        }

        foreach (var alike in visibility.Members.Where(x => x.Value.SectionIds.Count > 0).GroupBy(x => Key(x.Value.SectionIds)))
        {
            var seen = alike.First().Value.SectionIds;
            await clients.Groups(alike.Select(x => AppHub.FollowerGroup(serverId, x.Key)).ToArray())
                .SendAsync("SectionOrderChanged", Order(seen.Contains), cancellationToken);
        }
    }

    /// <summary>Tells each of <paramref name="userIds"/> that what they see on the server has changed.</summary>
    internal static async Task AccessChangedAsync(
        IHubClients<IClientProxy> clients,
        Guid serverId,
        IReadOnlyCollection<Guid> userIds,
        CancellationToken cancellationToken)
    {
        if (userIds.Count == 0)
        {
            return;
        }

        await clients.Groups(userIds.Select(x => AppHub.FollowerGroup(serverId, x)).ToArray())
            .SendAsync("ChannelAccessChanged", new ChannelAccessChangedDto(serverId.ToString()), cancellationToken);
    }

    private static string Key(IReadOnlySet<Guid> ids) => string.Join(',', ids.Order());
}

/// <summary>
/// One change that may show somebody a channel or hide one from them: a
/// change of roles, of what a role may do, of overrides, or of the section a
/// channel is in. What everybody sees is read before it, and again once it is
/// committed, and whoever sees something else is told.
/// </summary>
/// <remarks>
/// On a server where nobody is denied view before or after, each reading is
/// one query and nobody is told anything.
/// </remarks>
internal sealed class ViewChange
{
    private readonly ServerVisibility _before;

    private ViewChange(ServerVisibility before) => _before = before;

    internal static async Task<ViewChange> BeginAsync(AppDbContext db, Guid serverId, CancellationToken cancellationToken) =>
        new(await ServerVisibility.LoadAsync(db, serverId, cancellationToken));

    /// <summary>Tells whoever sees something else now; call it after the change is committed.</summary>
    internal async Task PublishAsync(IHubClients<IClientProxy> clients, AppDbContext db)
    {
        // Committed: the people told must not depend on the caller staying.
        var after = await ServerVisibility.LoadAsync(db, _before.ServerId, CancellationToken.None);
        var changed = await ServerVisibility.ChangedAsync(db, _before, after, CancellationToken.None);
        await ServerLayoutEvents.AccessChangedAsync(clients, _before.ServerId, changed, CancellationToken.None);
    }
}
