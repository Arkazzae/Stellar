CREATE TABLE IF NOT EXISTS "DiscordImportJobs" (
    "ImportId" TEXT NOT NULL PRIMARY KEY REFERENCES "DiscordImports" ("Id") ON DELETE CASCADE,
    "Options" TEXT NOT NULL,
    "RunId" TEXT NOT NULL,
    "Status" TEXT NOT NULL,
    "TotalSources" INTEGER NULL,
    "Report" TEXT NULL,
    "Error" TEXT NULL
);
