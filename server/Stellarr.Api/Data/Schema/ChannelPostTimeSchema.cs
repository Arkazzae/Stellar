using Microsoft.EntityFrameworkCore;

namespace Stellarr.Api.Data.Schema;

/// <summary>Records successful sends in their transaction, including when their messages are later deleted.</summary>
internal static class ChannelPostTimeSchema
{
    internal static Task CreateAsync(AppDbContext db, CancellationToken cancellationToken) =>
        db.Database.ExecuteSqlRawAsync(
            """
            CREATE TABLE IF NOT EXISTS "ChannelPostTimes" (
                "ChannelId" TEXT NOT NULL REFERENCES "Channels" ("Id") ON DELETE CASCADE,
                "UserId" TEXT NOT NULL REFERENCES "Users" ("Id") ON DELETE CASCADE,
                "LastPostedAt" TEXT NOT NULL,
                PRIMARY KEY ("ChannelId", "UserId")
            );
            CREATE INDEX IF NOT EXISTS "IX_ChannelPostTimes_UserId" ON "ChannelPostTimes" ("UserId");

            INSERT INTO "ChannelPostTimes" ("ChannelId", "UserId", "LastPostedAt")
            SELECT "ChannelId", "AuthorId", max("CreatedAt") FROM "Messages"
            WHERE "IsSystem" = 0 AND NOT EXISTS (
                SELECT 1 FROM sqlite_master WHERE type = 'trigger' AND name = 'Messages_PostTime_AfterInsert'
            )
            GROUP BY "ChannelId", "AuthorId"
            ON CONFLICT ("ChannelId", "UserId") DO UPDATE
            SET "LastPostedAt" = max("LastPostedAt", excluded."LastPostedAt");

            CREATE TRIGGER IF NOT EXISTS "Messages_PostTime_AfterInsert"
            AFTER INSERT ON "Messages" WHEN new."IsSystem" = 0
            BEGIN
                INSERT INTO "ChannelPostTimes" ("ChannelId", "UserId", "LastPostedAt")
                VALUES (new."ChannelId", new."AuthorId", new."CreatedAt")
                ON CONFLICT ("ChannelId", "UserId") DO UPDATE
                SET "LastPostedAt" = max("LastPostedAt", excluded."LastPostedAt");
            END;

            """,
            cancellationToken);
}
