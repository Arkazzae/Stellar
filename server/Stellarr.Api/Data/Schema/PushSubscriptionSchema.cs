using Microsoft.EntityFrameworkCore;

namespace Stellarr.Api.Data.Schema;

/// <summary>
/// Adds the push subscription table to a database created before the table
/// existed. <c>EnsureCreated</c> only builds a schema for an empty database,
/// so an installation upgrading in place needs this step as well.
/// </summary>
internal static class PushSubscriptionSchema
{
    internal static async Task CreateAsync(AppDbContext db, CancellationToken cancellationToken)
    {
        await db.Database.ExecuteSqlRawAsync(
            """
            CREATE TABLE IF NOT EXISTS "PushSubscriptions" (
                "Id" TEXT NOT NULL CONSTRAINT "PK_PushSubscriptions" PRIMARY KEY,
                "UserId" TEXT NOT NULL,
                "SessionId" TEXT NOT NULL,
                "Endpoint" TEXT NOT NULL,
                "P256dh" TEXT NOT NULL,
                "Auth" TEXT NOT NULL,
                "CreatedAt" TEXT NOT NULL,
                CONSTRAINT "FK_PushSubscriptions_Users_UserId"
                    FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
            );
            """,
            cancellationToken);

        await db.Database.ExecuteSqlRawAsync(
            """
            CREATE UNIQUE INDEX IF NOT EXISTS "IX_PushSubscriptions_Endpoint"
                ON "PushSubscriptions" ("Endpoint");
            CREATE INDEX IF NOT EXISTS "IX_PushSubscriptions_UserId_SessionId"
                ON "PushSubscriptions" ("UserId", "SessionId");
            """,
            cancellationToken);
    }
}
