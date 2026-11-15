using Microsoft.EntityFrameworkCore;

namespace Stellarr.Api.Data.Schema;

/// <summary>
/// Builds the channel catalogue behind search, media and links. The model
/// holds the ordinary table that pages stably; this adds the FTS5 table that
/// holds the tokens of the text, the author's names, the attachment names and
/// the link metadata. The triggers on the source tables keep both in step
/// with them, including embeds, which arrive after their message is stored
/// and would be missed for good by indexing in the request that stores it.
/// </summary>
/// <remarks>
/// A trigger body or a tokenizer option changed here changes what search
/// finds, and a trigger that already exists is not replaced.
/// </remarks>
internal static class MessageIndexSchema
{
    private const string SearchSql =
        """
        CREATE VIRTUAL TABLE IF NOT EXISTS "MessageSearch" USING fts5(
            "MessageId" UNINDEXED,
            "ChannelId" UNINDEXED,
            "SearchText",
            tokenize = 'unicode61 remove_diacritics 2',
            prefix = '2 3 4'
        );

        CREATE TRIGGER IF NOT EXISTS "MessageIndexEntries_Search_AfterInsert"
        AFTER INSERT ON "MessageIndexEntries"
        BEGIN
            INSERT INTO "MessageSearch" (rowid, "MessageId", "ChannelId", "SearchText")
            VALUES (new.rowid, new."MessageId", new."ChannelId", new."SearchText");
        END;

        CREATE TRIGGER IF NOT EXISTS "MessageIndexEntries_Search_AfterUpdate"
        AFTER UPDATE OF "MessageId", "ChannelId", "SearchText" ON "MessageIndexEntries"
        BEGIN
            UPDATE "MessageSearch"
            SET "MessageId" = new."MessageId",
                "ChannelId" = new."ChannelId",
                "SearchText" = new."SearchText"
            WHERE rowid = old.rowid;
        END;

        CREATE TRIGGER IF NOT EXISTS "MessageIndexEntries_Search_AfterDelete"
        AFTER DELETE ON "MessageIndexEntries"
        BEGIN
            DELETE FROM "MessageSearch" WHERE rowid = old.rowid;
        END;
        """;

    private const string SourceTriggersSql =
        """
        CREATE TRIGGER IF NOT EXISTS "Messages_Index_AfterInsert"
        AFTER INSERT ON "Messages"
        BEGIN
            INSERT INTO "MessageIndexEntries" (
                "MessageId", "ChannelId", "CreatedAt", "SearchText",
                "HasAttachments", "HasLinks"
            )
            SELECT
                new."Id",
                new."ChannelId",
                new."CreatedAt",
                trim(
                    coalesce(new."Content", '') || ' ' ||
                    coalesce((
                        SELECT u."DisplayName" || ' ' || u."Username"
                        FROM "Users" u WHERE u."Id" = new."AuthorId"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                        FROM "MessageAttachments" ma
                        JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                        WHERE ma."MessageId" = new."Id"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(
                            e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                            coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                            ' '
                        )
                        FROM "MessageEmbeds" e WHERE e."MessageId" = new."Id"
                    ), '')
                ),
                EXISTS(SELECT 1 FROM "MessageAttachments" ma WHERE ma."MessageId" = new."Id"),
                CASE WHEN
                    instr(lower(coalesce(new."Content", '')), 'https://') > 0 OR
                    instr(lower(coalesce(new."Content", '')), 'http://') > 0 OR
                    EXISTS(SELECT 1 FROM "MessageEmbeds" e WHERE e."MessageId" = new."Id")
                THEN 1 ELSE 0 END;
        END;

        CREATE TRIGGER IF NOT EXISTS "Messages_Index_AfterUpdate"
        AFTER UPDATE OF "ChannelId", "AuthorId", "Content", "CreatedAt" ON "Messages"
        BEGIN
            UPDATE "MessageIndexEntries"
            SET "ChannelId" = new."ChannelId",
                "CreatedAt" = new."CreatedAt",
                "SearchText" = trim(
                    coalesce(new."Content", '') || ' ' ||
                    coalesce((
                        SELECT u."DisplayName" || ' ' || u."Username"
                        FROM "Users" u WHERE u."Id" = new."AuthorId"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                        FROM "MessageAttachments" ma
                        JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                        WHERE ma."MessageId" = new."Id"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(
                            e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                            coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                            ' '
                        )
                        FROM "MessageEmbeds" e WHERE e."MessageId" = new."Id"
                    ), '')
                ),
                "HasLinks" = CASE WHEN
                    instr(lower(coalesce(new."Content", '')), 'https://') > 0 OR
                    instr(lower(coalesce(new."Content", '')), 'http://') > 0 OR
                    EXISTS(SELECT 1 FROM "MessageEmbeds" e WHERE e."MessageId" = new."Id")
                THEN 1 ELSE 0 END
            WHERE "MessageId" = new."Id";
        END;

        CREATE TRIGGER IF NOT EXISTS "MessageAttachments_Index_AfterInsert"
        AFTER INSERT ON "MessageAttachments"
        BEGIN
            UPDATE "MessageIndexEntries"
            SET "SearchText" = trim(
                    coalesce((SELECT m."Content" FROM "Messages" m WHERE m."Id" = new."MessageId"), '') || ' ' ||
                    coalesce((
                        SELECT u."DisplayName" || ' ' || u."Username"
                        FROM "Messages" m JOIN "Users" u ON u."Id" = m."AuthorId"
                        WHERE m."Id" = new."MessageId"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                        FROM "MessageAttachments" ma
                        JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                        WHERE ma."MessageId" = new."MessageId"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(
                            e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                            coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                            ' '
                        )
                        FROM "MessageEmbeds" e WHERE e."MessageId" = new."MessageId"
                    ), '')
                ),
                "HasAttachments" = 1
            WHERE "MessageId" = new."MessageId";
        END;

        CREATE TRIGGER IF NOT EXISTS "MessageAttachments_Index_AfterDelete"
        AFTER DELETE ON "MessageAttachments"
        BEGIN
            UPDATE "MessageIndexEntries"
            SET "SearchText" = trim(
                    coalesce((SELECT m."Content" FROM "Messages" m WHERE m."Id" = old."MessageId"), '') || ' ' ||
                    coalesce((
                        SELECT u."DisplayName" || ' ' || u."Username"
                        FROM "Messages" m JOIN "Users" u ON u."Id" = m."AuthorId"
                        WHERE m."Id" = old."MessageId"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                        FROM "MessageAttachments" ma
                        JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                        WHERE ma."MessageId" = old."MessageId"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(
                            e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                            coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                            ' '
                        )
                        FROM "MessageEmbeds" e WHERE e."MessageId" = old."MessageId"
                    ), '')
                ),
                "HasAttachments" = EXISTS(
                    SELECT 1 FROM "MessageAttachments" ma WHERE ma."MessageId" = old."MessageId"
                )
            WHERE "MessageId" = old."MessageId";
        END;

        CREATE TRIGGER IF NOT EXISTS "MessageEmbeds_Index_AfterInsert"
        AFTER INSERT ON "MessageEmbeds"
        BEGIN
            UPDATE "MessageIndexEntries"
            SET "SearchText" = trim(
                    coalesce((SELECT m."Content" FROM "Messages" m WHERE m."Id" = new."MessageId"), '') || ' ' ||
                    coalesce((
                        SELECT u."DisplayName" || ' ' || u."Username"
                        FROM "Messages" m JOIN "Users" u ON u."Id" = m."AuthorId"
                        WHERE m."Id" = new."MessageId"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                        FROM "MessageAttachments" ma
                        JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                        WHERE ma."MessageId" = new."MessageId"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(
                            e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                            coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                            ' '
                        )
                        FROM "MessageEmbeds" e WHERE e."MessageId" = new."MessageId"
                    ), '')
                ),
                "HasLinks" = 1
            WHERE "MessageId" = new."MessageId";
        END;

        CREATE TRIGGER IF NOT EXISTS "MessageEmbeds_Index_AfterUpdate"
        AFTER UPDATE OF "Url", "Domain", "Provider", "Title", "Description", "MessageId" ON "MessageEmbeds"
        BEGIN
            UPDATE "MessageIndexEntries"
            SET "SearchText" = trim(
                    coalesce((SELECT m."Content" FROM "Messages" m WHERE m."Id" = new."MessageId"), '') || ' ' ||
                    coalesce((
                        SELECT u."DisplayName" || ' ' || u."Username"
                        FROM "Messages" m JOIN "Users" u ON u."Id" = m."AuthorId"
                        WHERE m."Id" = new."MessageId"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                        FROM "MessageAttachments" ma
                        JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                        WHERE ma."MessageId" = new."MessageId"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(
                            e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                            coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                            ' '
                        )
                        FROM "MessageEmbeds" e WHERE e."MessageId" = new."MessageId"
                    ), '')
                ),
                "HasLinks" = 1
            WHERE "MessageId" = new."MessageId";
        END;

        CREATE TRIGGER IF NOT EXISTS "MessageEmbeds_Index_AfterDelete"
        AFTER DELETE ON "MessageEmbeds"
        BEGIN
            UPDATE "MessageIndexEntries"
            SET "SearchText" = trim(
                    coalesce((SELECT m."Content" FROM "Messages" m WHERE m."Id" = old."MessageId"), '') || ' ' ||
                    coalesce((
                        SELECT u."DisplayName" || ' ' || u."Username"
                        FROM "Messages" m JOIN "Users" u ON u."Id" = m."AuthorId"
                        WHERE m."Id" = old."MessageId"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                        FROM "MessageAttachments" ma
                        JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                        WHERE ma."MessageId" = old."MessageId"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(
                            e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                            coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                            ' '
                        )
                        FROM "MessageEmbeds" e WHERE e."MessageId" = old."MessageId"
                    ), '')
                ),
                "HasLinks" = CASE WHEN
                    instr(lower(coalesce((
                        SELECT m."Content" FROM "Messages" m WHERE m."Id" = old."MessageId"
                    ), '')), 'https://') > 0 OR
                    instr(lower(coalesce((
                        SELECT m."Content" FROM "Messages" m WHERE m."Id" = old."MessageId"
                    ), '')), 'http://') > 0 OR
                    EXISTS(SELECT 1 FROM "MessageEmbeds" e WHERE e."MessageId" = old."MessageId")
                THEN 1 ELSE 0 END
            WHERE "MessageId" = old."MessageId";
        END;

        CREATE TRIGGER IF NOT EXISTS "ImageAttachments_Index_AfterUpdate"
        AFTER UPDATE OF "FileName", "ContentType" ON "ImageAttachments"
        BEGIN
            UPDATE "MessageIndexEntries"
            SET "SearchText" = trim(
                    coalesce((SELECT m."Content" FROM "Messages" m WHERE m."Id" = "MessageIndexEntries"."MessageId"), '') || ' ' ||
                    coalesce((
                        SELECT u."DisplayName" || ' ' || u."Username"
                        FROM "Messages" m JOIN "Users" u ON u."Id" = m."AuthorId"
                        WHERE m."Id" = "MessageIndexEntries"."MessageId"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                        FROM "MessageAttachments" ma
                        JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                        WHERE ma."MessageId" = "MessageIndexEntries"."MessageId"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(
                            e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                            coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                            ' '
                        )
                        FROM "MessageEmbeds" e WHERE e."MessageId" = "MessageIndexEntries"."MessageId"
                    ), '')
                )
            WHERE "MessageId" IN (
                SELECT ma."MessageId" FROM "MessageAttachments" ma
                WHERE ma."AttachmentId" = new."Id"
            );
        END;

        CREATE TRIGGER IF NOT EXISTS "Users_MessageIndex_AfterUpdate"
        AFTER UPDATE OF "DisplayName", "Username" ON "Users"
        BEGIN
            UPDATE "MessageIndexEntries"
            SET "SearchText" = trim(
                    coalesce((SELECT m."Content" FROM "Messages" m WHERE m."Id" = "MessageIndexEntries"."MessageId"), '') || ' ' ||
                    new."DisplayName" || ' ' || new."Username" || ' ' ||
                    coalesce((
                        SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                        FROM "MessageAttachments" ma
                        JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                        WHERE ma."MessageId" = "MessageIndexEntries"."MessageId"
                    ), '') || ' ' ||
                    coalesce((
                        SELECT group_concat(
                            e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                            coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                            ' '
                        )
                        FROM "MessageEmbeds" e WHERE e."MessageId" = "MessageIndexEntries"."MessageId"
                    ), '')
                )
            WHERE "MessageId" IN (
                SELECT m."Id" FROM "Messages" m WHERE m."AuthorId" = new."Id"
            );
        END;
        """;

    internal static async Task CreateAsync(AppDbContext db, CancellationToken cancellationToken)
    {
        await db.Database.ExecuteSqlRawAsync(SearchSql, cancellationToken);
        await db.Database.ExecuteSqlRawAsync(SourceTriggersSql, cancellationToken);
    }
}
