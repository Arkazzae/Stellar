using Microsoft.AspNetCore.SignalR;
using Stellarr.Api.Auth;
using Stellarr.Api.Data;

namespace Stellarr.Api.Realtime;

/// <summary>
/// Who an event about one channel reaches: only the people who see it,
/// worked out when the event is sent.
/// </summary>
/// <remarks>
/// The cheap question comes first. When nobody is denied view at the channel,
/// everybody on the server sees it, and the event goes to the server's group
/// as it always has, with nothing evaluated. Otherwise one snapshot of every
/// member at the channel says who views it, and the event goes to their
/// <see cref="AppHub.FollowerGroup"/>s. Nothing is remembered between events,
/// so whoever loses view stops receiving with the next one.
/// </remarks>
internal static class ChannelAudience
{
    /// <summary>
    /// The clients an event about the channel goes to. <paramref name="seated"/>,
    /// the people sitting in a voice room, receive the room's events while they
    /// sit there whether or not they see it, since a moderator can move
    /// somebody into a room they do not see.
    /// </summary>
    internal static async Task<IClientProxy> OfAsync(
        IHubClients<IClientProxy> clients,
        AppDbContext db,
        Guid serverId,
        Guid channelId,
        IEnumerable<Guid>? seated,
        CancellationToken cancellationToken)
    {
        var viewers = await ChannelVisibility.ViewersAsync(db, serverId, channelId, cancellationToken);
        if (viewers is null)
        {
            return clients.Group(AppHub.ServerGroup(serverId));
        }

        var people = seated is null ? viewers : viewers.Concat(seated).Distinct();
        return clients.Groups(people.Select(x => AppHub.FollowerGroup(serverId, x)).ToArray());
    }

    /// <summary>Sends one event about the channel to whoever sees it.</summary>
    internal static async Task SendAsync(
        IHubClients<IClientProxy> clients,
        AppDbContext db,
        Guid serverId,
        Guid channelId,
        string method,
        object payload,
        CancellationToken cancellationToken)
    {
        var audience = await OfAsync(clients, db, serverId, channelId, seated: null, cancellationToken);
        await audience.SendAsync(method, payload, cancellationToken);
    }
}
