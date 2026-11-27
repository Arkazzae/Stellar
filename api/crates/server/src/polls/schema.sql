-- The poll a message carries, its answers and who chose which.
--
-- A poll belongs to its message and goes with it, as the message's
-- attachment links do; deleting a channel or a server deletes its messages,
-- and with them their polls. Counts are not stored: they are counted from
-- the votes whenever a poll is read, so nothing can drift from them.
-- "EndedAt" is set when the poll is ended early, or by the worker once
-- "EndsAt" has passed; until then a reader takes "EndsAt" for it.
CREATE TABLE IF NOT EXISTS "Polls" (
    "MessageId" TEXT NOT NULL CONSTRAINT "PK_Polls" PRIMARY KEY,
    "Question" TEXT NOT NULL,
    "Multiple" INTEGER NOT NULL,
    "EndsAt" TEXT NOT NULL,
    "EndedAt" TEXT NULL,
    "EndedById" TEXT NULL,
    CONSTRAINT "FK_Polls_Messages_MessageId" FOREIGN KEY ("MessageId") REFERENCES "Messages" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_Polls_Users_EndedById" FOREIGN KEY ("EndedById") REFERENCES "Users" ("Id") ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS "IX_Polls_EndsAt" ON "Polls" ("EndsAt") WHERE "EndedAt" IS NULL;

-- Answers are numbered from 1 in the order they were written, and never
-- change: a poll is not edited.
CREATE TABLE IF NOT EXISTS "PollAnswers" (
    "MessageId" TEXT NOT NULL,
    "AnswerId" INTEGER NOT NULL,
    "Text" TEXT NOT NULL,
    "Emoji" TEXT NULL,
    CONSTRAINT "PK_PollAnswers" PRIMARY KEY ("MessageId", "AnswerId"),
    CONSTRAINT "FK_PollAnswers_Polls_MessageId" FOREIGN KEY ("MessageId") REFERENCES "Polls" ("MessageId") ON DELETE CASCADE
);

-- One row for each answer a person chose. "CreatedAt" is when they first
-- chose it, which orders the list of who chose an answer.
CREATE TABLE IF NOT EXISTS "PollVotes" (
    "MessageId" TEXT NOT NULL,
    "AnswerId" INTEGER NOT NULL,
    "UserId" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    CONSTRAINT "PK_PollVotes" PRIMARY KEY ("MessageId", "AnswerId", "UserId"),
    CONSTRAINT "FK_PollVotes_PollAnswers_MessageId_AnswerId" FOREIGN KEY ("MessageId", "AnswerId") REFERENCES "PollAnswers" ("MessageId", "AnswerId") ON DELETE CASCADE,
    CONSTRAINT "FK_PollVotes_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS "IX_PollVotes_MessageId_UserId" ON "PollVotes" ("MessageId", "UserId");
CREATE INDEX IF NOT EXISTS "IX_PollVotes_MessageId_AnswerId_CreatedAt" ON "PollVotes" ("MessageId", "AnswerId", "CreatedAt", "UserId");
