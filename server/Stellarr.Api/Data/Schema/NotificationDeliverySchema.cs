using Microsoft.EntityFrameworkCore;

namespace Stellarr.Api.Data.Schema;

/// <summary>
/// Creates the activity delivery outbox on installations that already have a
/// database. <c>EnsureCreated</c> never adds a table to an existing database,
/// so this step mirrors the EF model and runs on every start. It is idempotent,
/// which also makes a start interrupted between the model and this step safe.
/// </summary>
internal static class NotificationDeliverySchema
{
    internal static Task CreateAsync(AppDbContext db, CancellationToken cancellationToken) =>
        db.Database.ExecuteSqlRawAsync(
            """
            CREATE TABLE IF NOT EXISTS "NotificationDeliveries" (
                "Id" TEXT NOT NULL CONSTRAINT "PK_NotificationDeliveries" PRIMARY KEY,
                "SourceMessageId" TEXT NOT NULL,
                "RecipientId" TEXT NOT NULL,
                "EventKind" TEXT NOT NULL,
                "Channel" TEXT NOT NULL,
                "DestinationKey" TEXT NOT NULL,
                "PushSubscriptionId" TEXT NULL,
                "CreatedAt" TEXT NOT NULL,
                "ExpiresAt" TEXT NOT NULL,
                "Attempts" INTEGER NOT NULL,
                "NextAttemptAt" TEXT NOT NULL,
                "LastAttemptAt" TEXT NULL,
                "SentAt" TEXT NULL,
                "Status" TEXT NOT NULL,
                "LeaseId" TEXT NULL,
                "LeaseUntil" TEXT NULL,
                "LastError" TEXT NOT NULL DEFAULT '',
                CONSTRAINT "FK_NotificationDeliveries_Users_RecipientId"
                    FOREIGN KEY ("RecipientId") REFERENCES "Users" ("Id") ON DELETE CASCADE
            );

            CREATE UNIQUE INDEX IF NOT EXISTS "IX_NotificationDeliveries_Unique"
                ON "NotificationDeliveries" ("SourceMessageId", "RecipientId", "EventKind", "Channel", "DestinationKey");
            CREATE INDEX IF NOT EXISTS "IX_NotificationDeliveries_Status_NextAttemptAt_LeaseUntil"
                ON "NotificationDeliveries" ("Status", "NextAttemptAt", "LeaseUntil");
            CREATE INDEX IF NOT EXISTS "IX_NotificationDeliveries_ExpiresAt"
                ON "NotificationDeliveries" ("ExpiresAt");
            CREATE INDEX IF NOT EXISTS "IX_NotificationDeliveries_RecipientId_Channel"
                ON "NotificationDeliveries" ("RecipientId", "Channel");
            CREATE INDEX IF NOT EXISTS "IX_NotificationDeliveries_PushSubscriptionId"
                ON "NotificationDeliveries" ("PushSubscriptionId");
            """,
            cancellationToken);
}
