using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Endpoints;

/// <summary>
/// Where a new role ranks. The base role, which everybody holds, ranks below
/// every other role: a role somebody was given has to outrank those who were
/// given none, or its holders could moderate nobody.
/// </summary>
internal static class RoleRanks
{
    /// <summary>
    /// The position for a role about to be added: last among the roles people
    /// are given. The base role is staged one place below it, so the caller
    /// saves the two together. The base role is picked as
    /// <c>ServerAccess.FindMemberRoleAsync</c> picks it.
    /// </summary>
    internal static async Task<int> PlaceNewAsync(AppDbContext db, Guid serverId, CancellationToken cancellationToken)
    {
        var baseRole = await db.ServerRoles
            .Where(x => x.ServerId == serverId && x.IsSystem && !x.IsOwner)
            .OrderByDescending(x => x.Position)
            .ThenBy(x => x.Id)
            .FirstOrDefaultAsync(cancellationToken);
        var baseRoleId = baseRole?.Id;
        var last = await db.ServerRoles
            .Where(x => x.ServerId == serverId && x.Id != baseRoleId)
            .MaxAsync(x => (int?)x.Position, cancellationToken);
        var position = (last ?? -1) + 1;
        if (baseRole is not null)
        {
            baseRole.Position = position + 1;
        }

        return position;
    }
}
