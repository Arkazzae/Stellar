CREATE TABLE IF NOT EXISTS "UploadLimits" (
    "Id" INTEGER NOT NULL PRIMARY KEY CHECK ("Id" = 1),
    "UserBytes" INTEGER NOT NULL CHECK ("UserBytes" >= 0)
);
