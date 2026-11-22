using Microsoft.EntityFrameworkCore;

namespace Stellarr.Api.Data.Schema;

/// <summary>
/// Gives a database made before members could hold several roles the tables
/// for them: the roles each member holds and the overrides set on a member.
/// The tables are written as the model creates them, so an upgraded database
/// and a new one cannot drift apart.
/// </summary>
/// <remarks>
/// Every member whose single role was not the base role then holds that role.
/// The fill runs on every start; it only adds what is missing, and in a
/// database this release wrote nothing is, because every writer stores the
/// top role as held. A backup restored from before the change gets the same.
/// The base role is picked as <c>ServerAccess.FindMemberRoleAsync</c> picks it.
/// </remarks>
internal static class MemberPermissionsSchema
{
    internal static async Task CreateAsync(AppDbContext db, CancellationToken cancellationToken)
    {
        await db.Database.ExecuteSqlRawAsync(
            """
            CREATE TABLE IF NOT EXISTS "ServerMemberRoles" (
                "ServerId" TEXT NOT NULL,
                "UserId" TEXT NOT NULL,
                "RoleId" TEXT NOT NULL,
                CONSTRAINT "PK_ServerMemberRoles" PRIMARY KEY ("ServerId", "UserId", "RoleId"),
                CONSTRAINT "FK_ServerMemberRoles_ServerMembers_ServerId_UserId" FOREIGN KEY ("ServerId", "UserId") REFERENCES "ServerMembers" ("ServerId", "UserId") ON DELETE CASCADE,
                CONSTRAINT "FK_ServerMemberRoles_ServerRoles_RoleId" FOREIGN KEY ("RoleId") REFERENCES "ServerRoles" ("Id") ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS "IX_ServerMemberRoles_RoleId" ON "ServerMemberRoles" ("RoleId");

            CREATE TABLE IF NOT EXISTS "MemberPermissionOverrides" (
                "Id" TEXT NOT NULL CONSTRAINT "PK_MemberPermissionOverrides" PRIMARY KEY,
                "ServerId" TEXT NOT NULL,
                "TargetType" INTEGER NOT NULL,
                "TargetId" TEXT NOT NULL,
                "UserId" TEXT NOT NULL,
                "AllowMask" INTEGER NOT NULL,
                "DenyMask" INTEGER NOT NULL,
                CONSTRAINT "FK_MemberPermissionOverrides_ServerMembers_ServerId_UserId" FOREIGN KEY ("ServerId", "UserId") REFERENCES "ServerMembers" ("ServerId", "UserId") ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS "IX_MemberPermissionOverrides_ServerId_UserId" ON "MemberPermissionOverrides" ("ServerId", "UserId");
            CREATE UNIQUE INDEX IF NOT EXISTS "IX_MemberPermissionOverrides_TargetType_TargetId_UserId" ON "MemberPermissionOverrides" ("TargetType", "TargetId", "UserId");

            INSERT OR IGNORE INTO "ServerMemberRoles" ("ServerId", "UserId", "RoleId")
            SELECT m."ServerId", m."UserId", m."RoleId"
            FROM "ServerMembers" AS m
            JOIN "ServerRoles" AS r ON r."Id" = m."RoleId" AND r."ServerId" = m."ServerId"
            WHERE m."RoleId" IS NOT (
                SELECT b."Id" FROM "ServerRoles" AS b
                WHERE b."ServerId" = m."ServerId" AND b."IsSystem" = 1 AND b."IsOwner" = 0
                ORDER BY b."Position" DESC, b."Id"
                LIMIT 1);
            """,
            cancellationToken);
    }
}
