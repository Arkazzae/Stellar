using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Data.OneTime;

/// <summary>
/// Gives a server from before the ordinary permissions what a new server
/// starts with, so that nobody there loses what they could do when posting,
/// attaching, reacting, joining voice and creating invites began to need a
/// permission: the base role gets <see cref="ServerRole.BaseRoleDefaults"/>,
/// every role that manages the server may create invites, as managing the
/// server let it until then, and the owner role stores everything there now
/// is.
/// </summary>
/// <remarks>
/// It runs once, through <see cref="OneTimeSteps"/>: an administrator who
/// takes one of these away later keeps it taken away. The base role is
/// picked as <c>ServerAccess.FindMemberRoleAsync</c> picks it.
/// </remarks>
internal static class OrdinaryPermissionDefaults
{
    internal static Task ApplyAsync(AppDbContext db, CancellationToken cancellationToken)
    {
        var defaults = (long)ServerRole.BaseRoleDefaults;
        var manageServer = (long)ServerPrivileges.ManageServer;
        var createInvite = (long)ServerPrivileges.CreateInvite;
        var everything = (long)ServerPrivileges.All;
        return db.Database.ExecuteSqlInterpolatedAsync(
            $"""
            UPDATE "ServerRoles" AS b
            SET "Permissions" = b."Permissions" | {defaults}
            WHERE b."Id" = (
                SELECT s."Id" FROM "ServerRoles" AS s
                WHERE s."ServerId" = b."ServerId" AND s."IsSystem" = 1 AND s."IsOwner" = 0
                ORDER BY s."Position" DESC, s."Id"
                LIMIT 1);
            UPDATE "ServerRoles" SET "Permissions" = "Permissions" | {createInvite}
            WHERE ("Permissions" & {manageServer}) <> 0;
            UPDATE "ServerRoles" SET "Permissions" = {everything} WHERE "IsOwner" = 1;
            """,
            cancellationToken);
    }
}
