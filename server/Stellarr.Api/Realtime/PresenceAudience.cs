using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;

namespace Stellarr.Api.Realtime;

/// <summary>Presence follows social contacts even when their tabs follow different servers.</summary>
internal static class PresenceAudience
{
    public static async Task<string[]> GroupsAsync(
        AppDbContext db, Guid userId, CancellationToken cancellationToken = default)
    {
        var servers = await db.ServerMembers.AsNoTracking().Where(x => x.UserId == userId)
            .Select(x => x.ServerId).ToArrayAsync(cancellationToken);
        var friends = db.Friendships.AsNoTracking()
            .Where(x => x.UserAId == userId || x.UserBId == userId)
            .Select(x => x.UserAId == userId ? x.UserBId : x.UserAId);
        var conversations = db.DirectConversations.AsNoTracking()
            .Where(x => x.UserAId == userId || x.UserBId == userId)
            .Select(x => x.UserAId == userId ? x.UserBId : x.UserAId);
        var peers = await friends.Union(conversations).Where(peer =>
            !db.UserBlocks.Any(block =>
                (block.BlockerId == userId && block.BlockedId == peer) ||
                (block.BlockerId == peer && block.BlockedId == userId)))
            .ToArrayAsync(cancellationToken);
        return servers.Select(AppHub.ServerGroup).Concat(peers.Select(AppHub.UserGroup)).ToArray();
    }
}
