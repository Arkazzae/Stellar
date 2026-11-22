using Microsoft.EntityFrameworkCore;

namespace Stellarr.Api.Data.Schema;

/// <summary>
/// Ranks the base role below every other role in a database from before new
/// roles were placed above it. Until then a new role took the position after
/// the last one, the base role's included, so the roles people were given
/// ranked below the one everybody holds, and their holders outranked nobody.
/// </summary>
/// <remarks>
/// Runs on every start and changes nothing once the base role is last. Only
/// the base role moves; the other roles keep their order among themselves.
/// The base role is picked as <c>ServerAccess.FindMemberRoleAsync</c> picks it.
/// </remarks>
internal static class RoleRankSchema
{
    internal static Task RepairAsync(AppDbContext db, CancellationToken cancellationToken) =>
        db.Database.ExecuteSqlRawAsync(
            """
            UPDATE "ServerRoles" AS b
            SET "Position" = (
                SELECT MAX(o."Position") + 1 FROM "ServerRoles" AS o
                WHERE o."ServerId" = b."ServerId" AND o."Id" <> b."Id")
            WHERE b."Id" = (
                    SELECT s."Id" FROM "ServerRoles" AS s
                    WHERE s."ServerId" = b."ServerId" AND s."IsSystem" = 1 AND s."IsOwner" = 0
                    ORDER BY s."Position" DESC, s."Id"
                    LIMIT 1)
                AND EXISTS (
                    SELECT 1 FROM "ServerRoles" AS o
                    WHERE o."ServerId" = b."ServerId" AND o."Id" <> b."Id" AND o."Position" >= b."Position");
            """,
            cancellationToken);
}
