using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Endpoints;

/// <summary>
/// The one writer of the roles members hold. Whatever changes them keeps
/// <see cref="ServerMember.RoleId"/> the highest role the member holds, or the
/// base role when they hold none, and stores that top role as held too.
/// </summary>
internal static class MemberRoles
{
    /// <summary>
    /// Stages a new membership. A role other than the base role, such as the
    /// owner role of a new server, is stored as held as well.
    /// </summary>
    internal static void Admit(AppDbContext db, ServerMember member, bool isBaseRole)
    {
        db.ServerMembers.Add(member);
        if (!isBaseRole)
        {
            db.ServerMemberRoles.Add(new ServerMemberRole
            {
                ServerId = member.ServerId,
                UserId = member.UserId,
                RoleId = member.RoleId,
            });
        }
    }

    /// <summary>
    /// Stages the roles <paramref name="member"/> holds as exactly
    /// <paramref name="roles"/>, and their top role with them; the base role,
    /// held by everybody, is left out. The member must be tracked.
    /// </summary>
    internal static async Task SetAsync(
        AppDbContext db,
        ServerMember member,
        IEnumerable<ServerRole> roles,
        ServerRole baseRole,
        CancellationToken cancellationToken)
    {
        var wanted = roles.Where(x => x.Id != baseRole.Id).DistinctBy(x => x.Id).ToList();
        var stored = await db.ServerMemberRoles
            .Where(x => x.ServerId == member.ServerId && x.UserId == member.UserId)
            .ToListAsync(cancellationToken);
        db.ServerMemberRoles.RemoveRange(stored.Where(row => wanted.All(role => role.Id != row.RoleId)));
        db.ServerMemberRoles.AddRange(wanted
            .Where(role => stored.All(row => row.RoleId != role.Id))
            .Select(role => new ServerMemberRole { ServerId = member.ServerId, UserId = member.UserId, RoleId = role.Id }));
        member.RoleId = HeldRoles.Top(wanted, baseRole).Id;
    }

    /// <summary>
    /// Puts the top role of every member of the server right again, counting
    /// <paramref name="leaving"/>, a role about to be deleted, as held by
    /// nobody. Moving roles can change which held role is the highest, so
    /// whatever changes their positions runs this as well. It writes at once,
    /// inside the caller's transaction.
    /// </summary>
    internal static async Task RankAgainAsync(
        AppDbContext db,
        Guid serverId,
        Guid? leaving,
        CancellationToken cancellationToken)
    {
        var table = await HeldRoles.LoadAsync(db, serverId, cancellationToken);
        var baseRole = table.BaseRole ?? throw new InvalidOperationException("The server has no role for plain members.");
        var members = await db.ServerMembers.AsNoTracking()
            .Where(x => x.ServerId == serverId)
            .Select(x => new { x.UserId, x.RoleId })
            .ToListAsync(cancellationToken);
        foreach (var member in members)
        {
            var top = HeldRoles.Top(table.Of(member.UserId).Where(x => x.Id != leaving), baseRole).Id;
            if (top != member.RoleId)
            {
                await db.ServerMembers.Where(x => x.ServerId == serverId && x.UserId == member.UserId)
                    .ExecuteUpdateAsync(x => x.SetProperty(row => row.RoleId, top), cancellationToken);
            }
        }
    }
}
