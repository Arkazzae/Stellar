-- Messages an author asked to send later, seen by nobody else until they go.
--
-- Only the author cascades. A row whose channel, conversation or answered
-- message is deleted stays, and fails with that reason when its time comes;
-- the author is told why instead of finding it gone. A row is pending while
-- "FailedAt" is empty. "DueAt" is when the worker next takes it: the time the
-- author chose, or later while slow mode holds it back. "Revision" counts the
-- author's edits, so that a send can tell that the row changed under it.
CREATE TABLE IF NOT EXISTS "ScheduledMessages" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_ScheduledMessages" PRIMARY KEY,
    "AuthorId" TEXT NOT NULL,
    "ServerId" TEXT NULL,
    "ChannelId" TEXT NULL,
    "ConversationId" TEXT NULL,
    "Content" TEXT NOT NULL,
    "ReplyToMessageId" TEXT NULL,
    "SendAt" TEXT NOT NULL,
    "DueAt" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "Revision" INTEGER NOT NULL DEFAULT 0,
    "FailureCode" TEXT NULL,
    "FailedAt" TEXT NULL,
    CONSTRAINT "FK_ScheduledMessages_Users_AuthorId" FOREIGN KEY ("AuthorId") REFERENCES "Users" ("Id") ON DELETE CASCADE,
    CONSTRAINT "CK_ScheduledMessages_Target" CHECK (
        ("ChannelId" IS NULL) <> ("ConversationId" IS NULL) AND ("ServerId" IS NULL) = ("ChannelId" IS NULL)
    )
);
CREATE INDEX IF NOT EXISTS "IX_ScheduledMessages_AuthorId_SendAt" ON "ScheduledMessages" ("AuthorId", "SendAt");
CREATE INDEX IF NOT EXISTS "IX_ScheduledMessages_DueAt" ON "ScheduledMessages" ("DueAt") WHERE "FailedAt" IS NULL;

-- The files of a scheduled message. They are held here as a message holds its
-- own, so that housekeeping does not take them for uploads nobody uses.
CREATE TABLE IF NOT EXISTS "ScheduledMessageAttachments" (
    "ScheduledMessageId" TEXT NOT NULL,
    "AttachmentId" TEXT NOT NULL,
    CONSTRAINT "PK_ScheduledMessageAttachments" PRIMARY KEY ("ScheduledMessageId", "AttachmentId"),
    CONSTRAINT "FK_ScheduledMessageAttachments_ScheduledMessages_ScheduledMessageId" FOREIGN KEY ("ScheduledMessageId") REFERENCES "ScheduledMessages" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_ScheduledMessageAttachments_ImageAttachments_AttachmentId" FOREIGN KEY ("AttachmentId") REFERENCES "ImageAttachments" ("Id") ON DELETE RESTRICT
);
CREATE INDEX IF NOT EXISTS "IX_ScheduledMessageAttachments_AttachmentId" ON "ScheduledMessageAttachments" ("AttachmentId");
