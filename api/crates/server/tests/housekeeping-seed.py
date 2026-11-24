"""Seeds and reads the database the housekeeping comparison runs on.

  python3 -I housekeeping-seed.py seed DB WORK NOW   write the rows and files
  python3 -I housekeeping-seed.py dump DB DATA_ROOT  print what is left

NOW is the moment, in Unix seconds, that the runs are expected to take place.
Every age is written relative to it, with rows a MARGIN seconds either side of
each cutoff, so that the seconds that pass between seeding and running (the
first server's start, ours) do not move a row across its line. Every row has a
label, its id is made from it, and the dump names rows by label.
"""
import json, os, sqlite3, sys, uuid
from datetime import datetime, timedelta, timezone

MARGIN = 120
DAY = 86400
NAMESPACE = uuid.UUID("5d0f6a4e-0000-4000-8000-00000000c0de")
# What the second server's settings (the newest) say, in days.
BACKUP_DAYS, LOG_DAYS, ORPHAN_DAYS = 7, 10, 2


def ident(label):
    return str(uuid.uuid5(NAMESPACE, label)).upper()


def stamp(epoch):
    return datetime.fromtimestamp(epoch, timezone.utc).strftime("%Y-%m-%d %H:%M:%S")


def seed(db, work, now):
    connection = sqlite3.connect(db)
    connection.execute("PRAGMA foreign_keys = OFF")
    labels = {}

    def new(label):
        labels[ident(label)] = label
        return ident(label)

    def around(base_seconds):
        """(inside, outside) moments of a cutoff `base_seconds` before now."""
        return now - base_seconds + MARGIN, now - base_seconds - MARGIN

    def run(sql, *args):
        connection.execute(sql, args)

    # The first server removes journal entries of channels that do not exist
    # when it starts, so the channels are made.
    instance_server = connection.execute('SELECT "Id" FROM "Servers" ORDER BY "CreatedAt","Id" LIMIT 1').fetchone()[0]

    def make_channel(label):
        cid = new(label)
        run('INSERT INTO "Channels" ("Id","ServerId","Name","Kind","Position","SlowModeSeconds","HideAfterInactivityHours","UserLimit") VALUES (?,?,?,0,0,0,0,0)',
            cid, instance_server, label)
        return cid

    # Journal: three channels with old and young entries, one with young only.
    for channel in ("busy", "mixed", "young-only"):
        cid = make_channel("channel-" + channel)
        for kind, base in (("inside", 30 * DAY), ("outside", 30 * DAY)):
            if channel == "young-only" and kind == "outside":
                continue
            when = around(base)[0 if kind == "inside" else 1]
            run('INSERT INTO "MessageChanges" ("ChannelId","MessageId","Kind","CreatedAt") VALUES (?,?,0,?)',
                cid, new(f"change-{channel}-{kind}"), stamp(when))
    clamp_channel = make_channel("channel-clamp")
    # The clamps: 1 day setting means 7, 1000 means 365.
    for days in (6, 8, 364, 366):
        run('INSERT INTO "MessageChanges" ("ChannelId","MessageId","Kind","CreatedAt") VALUES (?,?,0,?)',
            clamp_channel, new(f"change-clamp-{days}"), stamp(now - days * DAY))

    user = new("user")
    for kind, grace in (("short", DAY), ("long", 30 * DAY)):
        inside, outside = around(grace)
        for where, when in (("inside", inside), ("outside", outside)):
            at = stamp(when)
            n = lambda table: new(f"{table}-{where}")
            if kind == "short":
                run('INSERT INTO "AuthChallenges" ("Id","UserId","Purpose","SecretHash","PrimarySource","SecurityVersion","ShortLived","Attempts","MaxAttempts","CreatedAt","ExpiresAt") VALUES (?,?,?,?,?,1,0,0,5,?,?)',
                    n("AuthChallenges"), user, "login", "h" + where, "password", at, at)
                run('INSERT INTO "EmailOtpCodes" ("Id","UserId","Purpose","EmailAddress","ProtectedCode","Attempts","MaxAttempts","CreatedAt","ExpiresAt") VALUES (?,?,?,?,?,0,5,?,?)',
                    n("EmailOtpCodes"), user, "login", "a@b.c", "p", at, at)
                run('INSERT INTO "CliRecoveryCredentials" ("Id","UserId","TokenHash","Reason","SecurityVersion","CreatedAt","ExpiresAt") VALUES (?,?,?,?,1,?,?)',
                    n("CliRecoveryCredentials"), user, "h" + where, "r", at, at)
                run('INSERT INTO "PendingExternalIdentities" ("Id","SecretHash","ProviderId","Issuer","Subject","EmailVerified","Amr","ShortLived","CreatedAt","ExpiresAt") VALUES (?,?,?,?,?,0,?,0,?,?)',
                    n("PendingExternalIdentities"), "h" + where, "p", "i", "s", "", at, at)
            else:
                run('INSERT INTO "TrustedDevices" ("Id","UserId","TokenHash","Name","Device","IpAddress","CreatedAt","ExpiresAt") VALUES (?,?,?,?,?,?,?,?)',
                    n("TrustedDevices"), user, "h" + where, "n", "d", "1.1.1.1", at, at)
                run('INSERT INTO "RecoveryCodes" ("Id","UserId","BatchId","CodeHash","CreatedAt","UsedAt") VALUES (?,?,?,?,?,?)',
                    n("RecoveryCodes"), user, user, "h" + where, at, at)
                run('INSERT INTO "SecurityNotifications" ("Id","SecurityEventId","UserId","EmailAddress","Attempts","CreatedAt","NextAttemptAt","SentAt","LastError") VALUES (?,?,?,?,0,?,?,?,?)',
                    n("SecurityNotifications"), user, user, "a@b.c", at, at, at, "")
    old = stamp(now - 400 * DAY)
    run('INSERT INTO "RecoveryCodes" ("Id","UserId","BatchId","CodeHash","CreatedAt","UsedAt") VALUES (?,?,?,?,?,NULL)',
        new("RecoveryCodes-unused"), user, user, "unused", old)
    run('INSERT INTO "SecurityNotifications" ("Id","SecurityEventId","UserId","EmailAddress","Attempts","CreatedAt","NextAttemptAt","SentAt","LastError") VALUES (?,?,?,?,0,?,?,NULL,?)',
        new("SecurityNotifications-unsent"), user, user, "a@b.c", old, old, "")

    # Uploads: old enough or not, used or not.
    inside, outside = around(ORPHAN_DAYS * DAY)
    files = []

    def attachment(label, when, poster=True, preview=True):
        key = f"images/{label}"
        files.append(key)
        extra = []
        if poster:
            extra.append(("PosterObjectKey", key + ".poster"))
            files.append(key + ".poster")
        if preview:
            extra.append(("PreviewObjectKey", key + ".preview"))
            files.append(key + ".preview")
        columns = ["Id", "UploadedById", "ObjectKey", "FileName", "ContentType", "SizeBytes", "IsEncrypted", "CreatedAt"] + [c for c, _ in extra]
        values = [new("att-" + label), user, key, "f", "image/png", 1, 0, stamp(when)] + [v for _, v in extra]
        run(f'INSERT INTO "ImageAttachments" ({",".join(chr(34)+c+chr(34) for c in columns)}) VALUES ({",".join("?" * len(values))})', *values)
        return ident("att-" + label)

    attachment("young-unused", inside)
    for at in range(7):
        attachment(f"old-unused-{at}", outside - at, poster=at % 2 == 0, preview=at % 3 == 0)
    message = new("message")
    run('INSERT INTO "MessageAttachments" ("MessageId","AttachmentId") VALUES (?,?)', message, attachment("in-message", outside))
    run('INSERT INTO "DirectMessageAttachments" ("DirectMessageId","AttachmentId") VALUES (?,?)', new("dm"), attachment("in-dm", outside))
    run('INSERT INTO "MessageEmbeds" ("Id","MessageId","Url","Kind","Provider","Domain","ThumbnailAttachmentId","Position","CreatedAt") VALUES (?,?,?,0,?,?,?,0,?)',
        new("embed"), message, "u", "p", "d", attachment("in-embed", outside), stamp(outside))
    run('INSERT INTO "SoundboardSounds" ("Id","ServerId","AttachmentId","CreatedById","Name","Glyph","TrimStartSeconds","TrimEndSeconds","Position","CreatedAt") VALUES (?,?,?,?,?,?,0,1,0,?)',
        new("sound"), new("server-1"), attachment("in-sound", outside), user, "n", "g", stamp(outside))
    for column in ("AvatarUrl", "BannerUrl", "WallpaperUrl"):
        owner = new("user-" + column)
        attachment_id = attachment("in-" + column, outside)
        run('INSERT INTO "Users" ("Id","Username","NormalizedUsername","DisplayName","PasswordHash","IsDisabled","SessionVersion","SecurityRecoveryRequired","AvatarColor","CreatedAt",' + f'"{column}") VALUES (?,?,?,?,?,0,1,0,?,?,?)',
            owner, owner, owner, "x", "x", "c", stamp(outside), "/assets/" + attachment_id.lower())
    run('INSERT INTO "Servers" ("Id","Name","CreatedAt","SoundboardEveryoneCanPlay","SoundboardRateLimited","IconUrl") VALUES (?,?,?,1,0,?)',
        ident("server-icon"), "Icon", stamp(now - 300 * DAY), "/assets/" + attachment("in-icon", outside).lower())

    # The queue: some objects are in the store, some are not.
    for at in range(8):
        key = f"queued/{at}"
        if at % 2 == 0:
            files.append(key)
        run('INSERT INTO "ObjectDeletions" ("ObjectKey","QueuedAt") VALUES (?,?)', key, stamp(now - 3600 + at))


    # Rows of kinds the first server's housekeeping never prunes (some are
    # pruned when a request asks, which no request does here): old as can be,
    # and past every retention. Both servers must leave every one alone.
    ancient = stamp(now - 900 * DAY)
    run('INSERT INTO "PasswordResetTokens" ("Id","UserId","TokenHash","CreatedAt","ExpiresAt","UsedAt") VALUES (?,?,?,?,?,?)',
        new("keep-PasswordResetTokens"), user, "keep-reset", ancient, ancient, ancient)
    run('INSERT INTO "EmailChangeRequests" ("Id","UserId","NewEmail","NormalizedNewEmail","TokenHash","CreatedAt","ExpiresAt","UsedAt") VALUES (?,?,?,?,?,?,?,?)',
        new("keep-EmailChangeRequests"), user, "n@b.c", "N@B.C", "keep-mail", ancient, ancient, ancient)
    run('INSERT INTO "PasskeySignIns" ("Id","SecretHash","Options","CreatedAt","ExpiresAt","ConsumedAt") VALUES (?,?,?,?,?,?)',
        new("keep-PasskeySignIns"), "keep-passkey", "{}", ancient, ancient, ancient)
    run('INSERT INTO "DesktopBrowserLogins" ("Id","SecretHash","SecurityVersion","CreatedAt","ExpiresAt") VALUES (?,?,1,?,?)',
        new("keep-DesktopBrowserLogins"), "keep-desktop", ancient, ancient)
    run('INSERT INTO "UserSessions" ("Id","UserId","Device","IpAddress","CreatedAt","LastSeenAt","ExpiresAt","SecurityVersion","ShortLived","RevokedAt") VALUES (?,?,?,?,?,?,?,1,0,?)',
        new("keep-UserSessions"), user, "d", "1.1.1.1", ancient, ancient, ancient, ancient)
    run('INSERT INTO "ServerInvites" ("Id","ServerId","CreatedById","Code","Uses","ExpiresAt","CreatedAt") VALUES (?,?,?,?,0,?,?)',
        new("keep-ServerInvites"), instance_server, user, "keepinvite", ancient, ancient)

    # Link-preview pictures: kept under the all-zero uploader when a card shows
    # them, swept with their objects when nothing does.
    zero = "00000000-0000-0000-0000-000000000000"
    for label, shown in (("preview-shown", True), ("preview-orphan", False)):
        key = f"embeds/{label}"
        files.append(key)
        aid = new("att-" + label)
        run('INSERT INTO "ImageAttachments" ("Id","UploadedById","ObjectKey","FileName","ContentType","SizeBytes","IsEncrypted","CreatedAt") VALUES (?,?,?,?,?,1,0,?)',
            aid, zero, key, "f", "image/png", stamp(around(ORPHAN_DAYS * DAY)[1]))
        if shown:
            run('INSERT INTO "MessageEmbeds" ("Id","MessageId","Url","Kind","Provider","Domain","ThumbnailAttachmentId","Position","CreatedAt") VALUES (?,?,?,0,?,?,?,0,?)',
                new("embed-" + label), message, "u", "p", "d", aid, stamp(around(ORPHAN_DAYS * DAY)[1]))
    # Objects queued by uploads, servers, messages, emoji and profile pictures.
    for key in ("emoji/queued-a", "profile/queued-b", "servers/queued-c", "messages/queued-d"):
        files.append(key)
        run('INSERT INTO "ObjectDeletions" ("ObjectKey","QueuedAt") VALUES (?,?)', key, stamp(now - 7200))

    # Settings of two servers: the one written last decides.
    other = ident("server-2")
    labels[other] = "server-2"
    run('INSERT OR IGNORE INTO "Servers" ("Id","Name","CreatedAt","SoundboardEveryoneCanPlay","SoundboardRateLimited") VALUES (?,?,?,1,0)', other, "Two", stamp(now - 100 * DAY))
    instance = connection.execute('SELECT "Id" FROM "Servers" ORDER BY "CreatedAt","Id" LIMIT 1').fetchone()[0]
    columns = '"ServerId","PublicUrl","TimeZone","MailingEnabled","MailFromAddress","InviteMailEnabled","RecoveryMailEnabled","AlertMailEnabled","BackupRetentionDays","LogRetentionDays","OrphanObjectRetentionDays","AutomaticCleanupEnabled","UpdatedAt"'
    cleanup = int(os.environ.get("HOUSEKEEPING_CLEANUP", "1"))
    connection.execute("DELETE FROM \"OperationsSettings\"")
    run(f'INSERT INTO "OperationsSettings" ({columns}) VALUES (?,?,?,0,?,1,1,0,400,400,400,1,?)', instance, "", "UTC", "", stamp(now - 50 * DAY))
    run(f'INSERT INTO "OperationsSettings" ({columns}) VALUES (?,?,?,0,?,1,1,0,?,?,?,?,?)', other, "", "UTC", "", BACKUP_DAYS, LOG_DAYS, ORPHAN_DAYS, cleanup, stamp(now - 10 * DAY))
    connection.commit()
    connection.close()

    # Files of the data root, with their ages.
    root = os.path.join(work, "root")
    for key in files:
        path = os.path.join(root, "files", key)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        open(path, "w").write("x")
    for folder, days in (("backups", BACKUP_DAYS), ("logs", LOG_DAYS)):
        inside, outside = around(days * DAY)
        for name, when in (("inside.zip", inside), ("outside.zip", outside), ("inside.zip.verified", inside),
                           ("outside.zip.verified", outside), (".outside.zip.partial", outside), ("sub/outside", outside)):
            path = os.path.join(root, folder, name)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            open(path, "w").write("x")
            os.utime(path, (when, when))
    json.dump(labels, open(os.path.join(work, "labels.json"), "w"))


def dump(db, data_root, work):
    labels = json.load(open(os.path.join(work, "labels.json")))
    connection = sqlite3.connect(db)
    out = []
    for table, key in (("MessageChanges", "MessageId"), ("AuthChallenges", "Id"), ("EmailOtpCodes", "Id"),
                       ("CliRecoveryCredentials", "Id"), ("PendingExternalIdentities", "Id"), ("TrustedDevices", "Id"),
                       ("RecoveryCodes", "Id"), ("SecurityNotifications", "Id"), ("ImageAttachments", "Id"),
                       ("ObjectDeletions", "ObjectKey"), ("PasswordResetTokens", "Id"),
                       ("EmailChangeRequests", "Id"), ("PasskeySignIns", "Id"), ("DesktopBrowserLogins", "Id"),
                       ("UserSessions", "Id"), ("ServerInvites", "Id")):
        names = sorted(labels.get(row[0], "other:" + ("reset" if table == "MessageChanges" else "?"))
                       for row in connection.execute(f'SELECT "{key}" FROM "{table}"'))
        out.append(f"{table}: {names}")
    resets = connection.execute('SELECT COUNT(*), COUNT(DISTINCT "ChannelId") FROM "MessageChanges" WHERE "Kind" = 2').fetchone()
    out.append(f"resets (rows, channels): {resets}")
    for folder in ("backups", "logs", "files"):
        found = []
        for base, _, names in os.walk(os.path.join(data_root, folder)):
            found += [os.path.relpath(os.path.join(base, name), data_root) for name in names]
        out.append(f"{folder}: {sorted(found)}")
    print("\n".join(out))


if sys.argv[1] == "seed":
    seed(sys.argv[2], sys.argv[3], int(sys.argv[4]))
elif sys.argv[1] == "dump":
    dump(sys.argv[2], sys.argv[3], sys.argv[4])
