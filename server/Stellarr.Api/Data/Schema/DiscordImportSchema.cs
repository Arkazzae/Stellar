using Microsoft.EntityFrameworkCore;

namespace Stellarr.Api.Data.Schema;

/// <summary>
/// Gives a database made before the Discord import what it needs: the
/// column that marks a placeholder account, and the import's bookkeeping.
/// The tables are written as the model creates them, so an upgraded database
/// and a new one cannot drift apart.
/// </summary>
internal static class DiscordImportSchema
{
    private const string TablesSql =
        """
        CREATE TABLE IF NOT EXISTS "DiscordImports" (
            "Id" TEXT NOT NULL CONSTRAINT "PK_DiscordImports" PRIMARY KEY,
            "ServerId" TEXT NULL,
            "DiscordServerId" TEXT NOT NULL,
            "DiscordServerName" TEXT NOT NULL,
            "StartedById" TEXT NULL,
            "StartedAt" TEXT NOT NULL,
            CONSTRAINT "FK_DiscordImports_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE SET NULL,
            CONSTRAINT "FK_DiscordImports_Users_StartedById" FOREIGN KEY ("StartedById") REFERENCES "Users" ("Id") ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS "IX_DiscordImports_ServerId" ON "DiscordImports" ("ServerId");
        CREATE INDEX IF NOT EXISTS "IX_DiscordImports_StartedById" ON "DiscordImports" ("StartedById");

        CREATE TABLE IF NOT EXISTS "DiscordImportAccounts" (
            "DiscordId" TEXT NOT NULL CONSTRAINT "PK_DiscordImportAccounts" PRIMARY KEY,
            "UserId" TEXT NOT NULL,
            "ImportId" TEXT NULL,
            "IsWebhook" INTEGER NOT NULL,
            "IsBot" INTEGER NOT NULL,
            "DiscordUsername" TEXT NOT NULL,
            "DiscordAvatarUrl" TEXT NULL,
            "CreatedAt" TEXT NOT NULL,
            CONSTRAINT "FK_DiscordImportAccounts_DiscordImports_ImportId" FOREIGN KEY ("ImportId") REFERENCES "DiscordImports" ("Id") ON DELETE SET NULL,
            CONSTRAINT "FK_DiscordImportAccounts_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS "IX_DiscordImportAccounts_ImportId" ON "DiscordImportAccounts" ("ImportId");
        CREATE UNIQUE INDEX IF NOT EXISTS "IX_DiscordImportAccounts_UserId" ON "DiscordImportAccounts" ("UserId");

        CREATE TABLE IF NOT EXISTS "DiscordImportSections" (
            "ServerId" TEXT NOT NULL,
            "DiscordId" TEXT NOT NULL,
            "ImportId" TEXT NOT NULL,
            "SectionId" TEXT NULL,
            CONSTRAINT "PK_DiscordImportSections" PRIMARY KEY ("ServerId", "DiscordId"),
            CONSTRAINT "FK_DiscordImportSections_DiscordImports_ImportId" FOREIGN KEY ("ImportId") REFERENCES "DiscordImports" ("Id") ON DELETE CASCADE,
            CONSTRAINT "FK_DiscordImportSections_Sections_SectionId" FOREIGN KEY ("SectionId") REFERENCES "Sections" ("Id") ON DELETE SET NULL,
            CONSTRAINT "FK_DiscordImportSections_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS "IX_DiscordImportSections_ImportId" ON "DiscordImportSections" ("ImportId");
        CREATE INDEX IF NOT EXISTS "IX_DiscordImportSections_SectionId" ON "DiscordImportSections" ("SectionId");

        CREATE TABLE IF NOT EXISTS "DiscordImportChannels" (
            "ServerId" TEXT NOT NULL,
            "DiscordId" TEXT NOT NULL,
            "ImportId" TEXT NOT NULL,
            "Kind" INTEGER NOT NULL,
            "ChannelId" TEXT NULL,
            "DiscordType" INTEGER NOT NULL,
            "ParentDiscordId" TEXT NULL,
            "Name" TEXT NULL,
            "StarterMessageId" TEXT NULL,
            "IsPrivate" INTEGER NOT NULL,
            "OldestMessageId" TEXT NULL,
            "CompletedAt" TEXT NULL,
            "RootMessageId" TEXT NULL,
            "LinkedAt" TEXT NULL,
            CONSTRAINT "PK_DiscordImportChannels" PRIMARY KEY ("ServerId", "DiscordId"),
            CONSTRAINT "FK_DiscordImportChannels_Channels_ChannelId" FOREIGN KEY ("ChannelId") REFERENCES "Channels" ("Id") ON DELETE SET NULL,
            CONSTRAINT "FK_DiscordImportChannels_DiscordImports_ImportId" FOREIGN KEY ("ImportId") REFERENCES "DiscordImports" ("Id") ON DELETE CASCADE,
            CONSTRAINT "FK_DiscordImportChannels_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS "IX_DiscordImportChannels_ChannelId" ON "DiscordImportChannels" ("ChannelId");
        CREATE INDEX IF NOT EXISTS "IX_DiscordImportChannels_ImportId" ON "DiscordImportChannels" ("ImportId");
        CREATE INDEX IF NOT EXISTS "IX_DiscordImportChannels_ServerId_ParentDiscordId" ON "DiscordImportChannels" ("ServerId", "ParentDiscordId");

        CREATE TABLE IF NOT EXISTS "DiscordImportMessages" (
            "ServerId" TEXT NOT NULL,
            "DiscordId" TEXT NOT NULL,
            "ImportId" TEXT NOT NULL,
            "ChannelDiscordId" TEXT NOT NULL,
            "Outcome" INTEGER NOT NULL,
            "Reason" INTEGER NOT NULL,
            "DiscordType" INTEGER NOT NULL,
            "CreatedAt" TEXT NOT NULL,
            "MessageId" TEXT NULL,
            "ReplyToDiscordId" TEXT NULL,
            "AttachmentsPending" INTEGER NOT NULL,
            CONSTRAINT "PK_DiscordImportMessages" PRIMARY KEY ("ServerId", "DiscordId"),
            CONSTRAINT "FK_DiscordImportMessages_DiscordImports_ImportId" FOREIGN KEY ("ImportId") REFERENCES "DiscordImports" ("Id") ON DELETE CASCADE,
            CONSTRAINT "FK_DiscordImportMessages_Messages_MessageId" FOREIGN KEY ("MessageId") REFERENCES "Messages" ("Id") ON DELETE SET NULL,
            CONSTRAINT "FK_DiscordImportMessages_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS "IX_DiscordImportMessages_ImportId" ON "DiscordImportMessages" ("ImportId");
        CREATE INDEX IF NOT EXISTS "IX_DiscordImportMessages_MessageId" ON "DiscordImportMessages" ("MessageId");
        CREATE INDEX IF NOT EXISTS "IX_DiscordImportMessages_ServerId_ChannelDiscordId_Outcome" ON "DiscordImportMessages" ("ServerId", "ChannelDiscordId", "Outcome");
        """;

    internal static async Task CreateAsync(AppDbContext db, CancellationToken cancellationToken)
    {
        await db.Database.OpenConnectionAsync(cancellationToken);
        try
        {
            var columns = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            await using (var command = db.Database.GetDbConnection().CreateCommand())
            {
                command.CommandText = "PRAGMA table_info(\"Users\");";
                await using var reader = await command.ExecuteReaderAsync(cancellationToken);
                while (await reader.ReadAsync(cancellationToken))
                {
                    columns.Add(reader.GetString(1));
                }
            }

            if (!columns.Contains("Origin"))
            {
                await db.Database.ExecuteSqlRawAsync(
                    "ALTER TABLE \"Users\" ADD COLUMN \"Origin\" TEXT NULL;", cancellationToken);
            }

            await db.Database.ExecuteSqlRawAsync(TablesSql, cancellationToken);
        }
        finally
        {
            await db.Database.CloseConnectionAsync();
        }
    }
}
