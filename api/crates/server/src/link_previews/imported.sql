CREATE TABLE IF NOT EXISTS "DiscordImportPreviews" (
    "MessageId" TEXT NOT NULL PRIMARY KEY REFERENCES "Messages" ("Id") ON DELETE CASCADE,
    "Cards" TEXT NULL,
    "AttemptedAt" TEXT NULL
);
