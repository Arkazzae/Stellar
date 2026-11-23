using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.SignalR;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Realtime;

/// <summary>
/// The one realtime connection of a signed-in client. Everything the server
/// pushes reaches clients through the groups named here: one per account, one
/// per server, and one per account on the server a connection follows.
/// </summary>
/// <remarks>
/// No group stands for who may see something. A group's members are decided
/// when a connection joins it and would go stale when roles or overrides
/// change, so whoever may see an event is worked out when it is sent, and the
/// event goes to the server's group when that is everybody, else to the
/// groups of the people it is for (<see cref="ChannelAudience"/>).
/// </remarks>
[Authorize]
public sealed partial class AppHub(
    AppDbContext db,
    PresenceTracker presence,
    SoundboardPlaybackGate soundboardGate,
    HubThrottleGate throttle,
    MediaPlane mediaPlane,
    MediaControlClient mediaControl,
    HubConnectionRegistry connections,
    ILogger<AppHub> logger) : Hub
{
    private static readonly TimeSpan TypingLifetime = TimeSpan.FromSeconds(6);

    public static string ServerGroup(Guid serverId) => $"server:{serverId:N}";

    /// <summary>
    /// The connections of one account that follow one server: what an event
    /// meant for some of a server's members goes to. Unlike the account's own
    /// group, it leaves out the account's tabs on other servers, which would
    /// take a message for a channel they do not know as something to sound.
    /// </summary>
    public static string FollowerGroup(Guid serverId, Guid userId) => $"server:{serverId:N}:user:{userId:N}";

    public static string UserGroup(Guid userId) => $"user:{userId:N}";

    public override async Task OnConnectedAsync()
    {
        var userId = Context.User!.RequireUserId();
        if (Context.User!.FindSessionId() is { } sessionId)
        {
            var version = int.TryParse(
                Context.User!.FindFirst(SessionAuthentication.VersionClaim)?.Value,
                System.Globalization.CultureInfo.InvariantCulture,
                out var parsed) ? parsed : 0;
            connections.Register(Context.ConnectionId, userId, sessionId, version, Context.Abort);
        }

        await Groups.AddToGroupAsync(Context.ConnectionId, UserGroup(userId));

        // A connection follows one server at a time: the one the client asked
        // for with ?serverId=, else the first it belongs to. That keeps one
        // server's traffic away from clients that are looking at another.
        var serverIds = await MemberServerIdsAsync(userId);
        var requested = Context.GetHttpContext()?.Request.Query["serverId"].FirstOrDefault();
        var activeServerId = Guid.TryParse(requested, out var requestedId) && serverIds.Contains(requestedId)
            ? requestedId
            : serverIds.FirstOrDefault();
        if (activeServerId != Guid.Empty)
        {
            await Groups.AddToGroupAsync(Context.ConnectionId, ServerGroup(activeServerId));
            await Groups.AddToGroupAsync(Context.ConnectionId, FollowerGroup(activeServerId, userId));
        }

        var showPresence = await db.UserPreferences.AsNoTracking()
            .Where(x => x.UserId == userId)
            .Select(x => (bool?)x.ShowPresence)
            .SingleOrDefaultAsync() ?? true;
        var becameConnected = presence.Connect(Context.ConnectionId, userId, showPresence);

        // Only the first connection changes what others see. Every connection
        // still gets its own answer, because two tabs may have loaded while
        // the account was offline and then race to connect.
        if (becameConnected)
        {
            await Clients.Groups(await PresenceAudience.GroupsAsync(db, userId))
                .SendAsync("PresenceChanged", PresenceUpdate(userId, presence.PublicPresenceOf(userId)));
        }
        await Clients.Caller.SendAsync("PresenceChanged", PresenceUpdate(userId, presence.PresenceOf(userId)));

        await RestoreDirectCallSnapshotsAsync(userId);

        await base.OnConnectedAsync();
    }

    public override async Task OnDisconnectedAsync(Exception? exception)
    {
        connections.Unregister(Context.ConnectionId);
        var (becameOffline, departure) = presence.Disconnect(Context.ConnectionId);
        if (departure is not null)
        {
            await BroadcastVoiceSnapshotAsync(departure.ChannelId);
        }

        if (becameOffline && Context.User is { } user)
        {
            var userId = user.RequireUserId();
            var groups = await PresenceAudience.GroupsAsync(db, userId);
            await Clients.Groups(groups).SendAsync("PresenceChanged", PresenceUpdate(userId, PresenceTracker.Offline));
        }

        await base.OnDisconnectedAsync(exception);
    }

    public async Task SetPresence(string value)
    {
        if (!PresenceTracker.IsSupportedPresence(value))
        {
            throw new HubException("Unknown presence.");
        }

        var userId = Context.User!.RequireUserId();
        if (!presence.SetPresence(userId, value))
        {
            return;
        }

        var groups = await PresenceAudience.GroupsAsync(db, userId);
        await Clients.Groups(groups).SendAsync("PresenceChanged", PresenceUpdate(userId, presence.PublicPresenceOf(userId)));
        await Clients.Caller.SendAsync("PresenceChanged", PresenceUpdate(userId, value));
    }

    /// <summary>
    /// Confirms that the account may follow one text channel, which it may
    /// when it sees it. Nothing is joined: the channel's events reach whoever
    /// sees it when they are sent (<see cref="ChannelAudience"/>), so a group
    /// joined here would only go stale when somebody lost view.
    /// </summary>
    public async Task JoinChannel(string channelId)
    {
        var id = ParseGuid(channelId, nameof(channelId));
        if (await VisibleTextChannelAsync(id, Context.User!.RequireUserId()) is null)
        {
            throw new HubException("Unknown channel.");
        }
    }

    /// <summary>
    /// Tells the others who see a text channel that this account is typing
    /// in it. Typing needs what sending does: seeing the channel, which
    /// somebody who does not is answered as for no channel at all, and
    /// SendMessages there.
    /// </summary>
    public async Task SendTyping(string channelId)
    {
        // One broadcast reaches the channel's whole audience and costs a few
        // lookups, so the cadence is decided here rather than trusted to every
        // client.
        if (!throttle.TryAcquire(Context.ConnectionId, HubThrottleGate.Policy.Typing))
        {
            throw new HubException("Typing is too fast.");
        }

        var id = ParseGuid(channelId, nameof(channelId));
        var userId = Context.User!.RequireUserId();
        var serverId = await db.Channels.AsNoTracking()
            .Where(x => x.Id == id && x.Kind == ChannelKind.Text)
            .Select(x => (Guid?)x.ServerId)
            .SingleOrDefaultAsync();
        var held = serverId is { } server
            ? await ServerAuthorization.EffectiveAsync(db, userId, PermissionScope.Channel(server, id)) ?? ServerPrivileges.None
            : ServerPrivileges.None;
        if (serverId is null || !held.HasFlag(ServerPrivileges.ViewChannel))
        {
            throw new HubException("Unknown channel.");
        }

        if (!held.HasFlag(ServerPrivileges.SendMessages))
        {
            throw new HubException("Not allowed to send messages here.");
        }

        var displayName = await db.Users.Where(user => user.Id == userId)
            .Select(user => user.DisplayName)
            .SingleAsync();
        var typing = new TypingChangedDto(id.ToString(), userId.ToString(), displayName, DateTime.UtcNow + TypingLifetime);
        var viewers = await ChannelVisibility.ViewersAsync(db, serverId.Value, id, Context.ConnectionAborted);
        if (viewers is null)
        {
            await Clients.OthersInGroup(ServerGroup(serverId.Value)).SendAsync("TypingChanged", typing);
            return;
        }

        // Everybody else who sees it, and this account's other tabs, as the
        // server's group without this connection would have reached.
        await Clients.Groups(viewers.Where(x => x != userId).Select(x => FollowerGroup(serverId.Value, x)).ToArray())
            .SendAsync("TypingChanged", typing);
        await Clients.GroupExcept(FollowerGroup(serverId.Value, userId), [Context.ConnectionId])
            .SendAsync("TypingChanged", typing);
    }

    /// <summary>The server's UTC clock in Unix milliseconds, which clients use to correct for their own skew.</summary>
    public long GetRtcClock() => DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();

    private static object PresenceUpdate(Guid userId, string presence) => new { userId = userId.ToString(), presence };

    private static Guid ParseGuid(string value, string field) =>
        Guid.TryParse(value, out var parsed) ? parsed : throw new HubException($"Invalid {field}.");

    private async Task<Guid[]> MemberServerIdsAsync(Guid userId) =>
        await db.ServerMembers.AsNoTracking()
            .Where(x => x.UserId == userId)
            .Select(x => x.ServerId)
            .ToArrayAsync();

    /// <summary>A text channel the account sees, or null when there is no such channel for it.</summary>
    private Task<ChannelPlace?> VisibleTextChannelAsync(Guid channelId, Guid userId) =>
        ChannelVisibility.FindAsync(
            db, userId, db.Channels.Where(x => x.Id == channelId && x.Kind == ChannelKind.Text), Context.ConnectionAborted);
}
