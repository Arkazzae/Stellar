using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Data.OneTime;

/// <summary>
/// Gives a server from before <see cref="ServerPrivileges.ViewChannel"/> what
/// a new server starts with, so that nobody there stops seeing a channel when
/// seeing one began to need a permission: the base role gets it, and the
/// owner role stores everything there now is.
/// </summary>
/// <remarks>
/// It runs once, through <see cref="OneTimeSteps"/>: an administrator who
/// takes the permission from the base role later keeps it taken away. The
/// base role is picked as <c>ServerAccess.FindMemberRoleAsync</c> picks it.
/// No other role needs it: every member holds the base role.
/// </remarks>
internal static class ViewChannelDefaults
{
    internal static Task ApplyAsync(AppDbContext db, CancellationToken cancellationToken)
    {
        var view = (long)ServerPrivileges.ViewChannel;
        var everything = (long)ServerPrivileges.All;
        return db.Database.ExecuteSqlInterpolatedAsync(
            $"""
            UPDATE "ServerRoles" AS b
            SET "Permissions" = b."Permissions" | {view}
            WHERE b."Id" = (
                SELECT s."Id" FROM "ServerRoles" AS s
                WHERE s."ServerId" = b."ServerId" AND s."IsSystem" = 1 AND s."IsOwner" = 0
                ORDER BY s."Position" DESC, s."Id"
                LIMIT 1);
            UPDATE "ServerRoles" SET "Permissions" = {everything} WHERE "IsOwner" = 1;
            """,
            cancellationToken);
    }
}
