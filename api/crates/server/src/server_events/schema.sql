-- Events of a server, the occurrence each has now, who signed up for it, the
-- messages that announce them, and the inbox entries that are about them.
--
-- An event goes with its server. Its voice channel is a plain column, so that
-- the event outlives the channel and is cancelled with a reason rather than
-- vanishing. "FirstStartsAt" and "TimeZone" are the rule of a series: every
-- occurrence starts at the clock time of the first, so many weeks or months
-- on. "Revision" grows with every change, and orders what clients are told.
CREATE TABLE IF NOT EXISTS "ServerEvents" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_ServerEvents" PRIMARY KEY,
    "ServerId" TEXT NOT NULL,
    "Name" TEXT NOT NULL,
    "Description" TEXT NULL,
    "VoiceChannelId" TEXT NULL,
    "PlaceText" TEXT NULL,
    "TimeZone" TEXT NOT NULL,
    "FirstStartsAt" TEXT NOT NULL,
    "DurationSeconds" INTEGER NULL,
    "Repeat" TEXT NOT NULL,
    "Seats" INTEGER NULL,
    "CreatedById" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "CancelledAt" TEXT NULL,
    "Revision" INTEGER NOT NULL DEFAULT 0,
    CONSTRAINT "FK_ServerEvents_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_ServerEvents_Users_CreatedById" FOREIGN KEY ("CreatedById") REFERENCES "Users" ("Id") ON DELETE CASCADE,
    CONSTRAINT "CK_ServerEvents_Place" CHECK (("VoiceChannelId" IS NULL) <> ("PlaceText" IS NULL))
);
CREATE INDEX IF NOT EXISTS "IX_ServerEvents_ServerId" ON "ServerEvents" ("ServerId");

-- The one occurrence an event has: the next, the running one, or the last
-- once a series is over. "Sequence" counts the occurrences from the first
-- start. "Phase" is what the worker last told clients, so that it tells each
-- change once.
CREATE TABLE IF NOT EXISTS "ServerEventOccurrences" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_ServerEventOccurrences" PRIMARY KEY,
    "EventId" TEXT NOT NULL,
    "Sequence" INTEGER NOT NULL,
    "StartsAt" TEXT NOT NULL,
    "EndsAt" TEXT NULL,
    "LiveSince" TEXT NULL,
    "EmptySince" TEXT NULL,
    "EndedAt" TEXT NULL,
    "Phase" TEXT NOT NULL DEFAULT 'upcoming',
    CONSTRAINT "FK_ServerEventOccurrences_ServerEvents_EventId" FOREIGN KEY ("EventId") REFERENCES "ServerEvents" ("Id") ON DELETE CASCADE
);
CREATE UNIQUE INDEX IF NOT EXISTS "IX_ServerEventOccurrences_EventId" ON "ServerEventOccurrences" ("EventId");
CREATE INDEX IF NOT EXISTS "IX_ServerEventOccurrences_StartsAt" ON "ServerEventOccurrences" ("StartsAt") WHERE "EndedAt" IS NULL;

-- A sign-up, and whether it holds a seat. "Number" is the order people
-- signed up in, which the waitlist goes by. A sign-up goes with its
-- membership, so leaving the server, a kick or a ban takes it.
CREATE TABLE IF NOT EXISTS "ServerEventSignups" (
    "Number" INTEGER NOT NULL CONSTRAINT "PK_ServerEventSignups" PRIMARY KEY AUTOINCREMENT,
    "OccurrenceId" TEXT NOT NULL,
    "ServerId" TEXT NOT NULL,
    "UserId" TEXT NOT NULL,
    "Seated" INTEGER NOT NULL,
    "SignedUpAt" TEXT NOT NULL,
    "RemindedAt" TEXT NULL,
    CONSTRAINT "FK_ServerEventSignups_ServerEventOccurrences_OccurrenceId" FOREIGN KEY ("OccurrenceId") REFERENCES "ServerEventOccurrences" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_ServerEventSignups_ServerMembers_ServerId_UserId" FOREIGN KEY ("ServerId", "UserId") REFERENCES "ServerMembers" ("ServerId", "UserId") ON DELETE CASCADE
);
CREATE UNIQUE INDEX IF NOT EXISTS "IX_ServerEventSignups_OccurrenceId_UserId" ON "ServerEventSignups" ("OccurrenceId", "UserId");
CREATE INDEX IF NOT EXISTS "IX_ServerEventSignups_ServerId_UserId" ON "ServerEventSignups" ("ServerId", "UserId");

-- The message that announces an event and shows its card.
CREATE TABLE IF NOT EXISTS "ServerEventAnnouncements" (
    "MessageId" TEXT NOT NULL CONSTRAINT "PK_ServerEventAnnouncements" PRIMARY KEY,
    "EventId" TEXT NOT NULL,
    CONSTRAINT "FK_ServerEventAnnouncements_Messages_MessageId" FOREIGN KEY ("MessageId") REFERENCES "Messages" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_ServerEventAnnouncements_ServerEvents_EventId" FOREIGN KEY ("EventId") REFERENCES "ServerEvents" ("Id") ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS "IX_ServerEventAnnouncements_EventId" ON "ServerEventAnnouncements" ("EventId");

-- The event an inbox entry is about, which an entry has no column for.
CREATE TABLE IF NOT EXISTS "ServerEventNotices" (
    "NotificationId" TEXT NOT NULL CONSTRAINT "PK_ServerEventNotices" PRIMARY KEY,
    "EventId" TEXT NOT NULL,
    CONSTRAINT "FK_ServerEventNotices_UserNotifications_NotificationId" FOREIGN KEY ("NotificationId") REFERENCES "UserNotifications" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_ServerEventNotices_ServerEvents_EventId" FOREIGN KEY ("EventId") REFERENCES "ServerEvents" ("Id") ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS "IX_ServerEventNotices_EventId" ON "ServerEventNotices" ("EventId");

-- Owners and administrators hold every permission, and their roles were
-- stored before the two of events existed: they are stored with them now,
-- so that the lists of roles show what such a role holds. Running it again
-- changes nothing.
UPDATE "ServerRoles" SET "Permissions" = "Permissions" | 98304
WHERE "IsOwner" = 1 OR ("Permissions" & 256) <> 0;
