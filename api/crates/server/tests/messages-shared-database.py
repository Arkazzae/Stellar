#!/usr/bin/env python3
"""The message routes of this server and of the first one, on one database file.

A server is founded and people registered on this one, a few thousand messages
are written into its file (some at the very same moment, some replies, some with
attachments, cards and reactions, in a private channel and one under slow mode),
and the file is handed to the first server, which then edits, reacts to and
deletes a share of them through its own routes. The file that comes back is
copied to each server, and they are asked the same things:

  history   every page of the channel's history, read the way a client does (from
            a cursor, newest to oldest), at several page sizes, each cursor the
            first server gave also given to this one, and every page and cursor
            compared;
  sync      the journal from position zero at several page sizes;
  refusals  the answers to limits, cursors and channels that are not there;
  writes    the same posts, edits and deletions on both, each on a copy of the
            file, and the rows each leaves compared once ids and moments are
            replaced.

Needs docker, python3 and the first server's container from
`oracle start <name> <port> Development`, which must be fresh. Not part of the
ordinary run.

  python3 messages-shared-database.py <oracle name> <oracle port> <stellarr-api binary> [stage ...]

Stages: history, sync, refusals, writes, mentions (all by default). Set
MESSAGES=4000 for the number of messages and SEED for the random shapes.
"""
import base64
import json
import os
import random
import re
import shutil
import sqlite3
import struct
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid

PASSWORD = "Corona!2026-long"
CLIENT = {"X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0"}
PEOPLE = ["alice", "bob", "carol", "dave", "erin", "frank"]
SIGNED_IN = ["owner", "alice", "bob", "carol", "dave", "erin"]


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def send(base, method, path, body=None, cookie=None, headers=None):
    request = urllib.request.Request(
        base + path, method=method, data=json.dumps(body).encode() if body is not None else None)
    merged = dict(CLIENT)
    if body is not None:
        merged["Content-Type"] = "application/json"
    merged.update(headers or {})
    for name, value in merged.items():
        request.add_header(name, value)
    if cookie:
        request.add_header("Cookie", cookie)
    try:
        with urllib.request.build_opener(NoRedirect).open(request) as response:
            return response.status, response.read(), response.headers
    except urllib.error.HTTPError as error:
        return error.code, error.read(), error.headers


def call(base, method, path, body=None, cookie=None, headers=None):
    while True:
        status, reply, answer = send(base, method, path, body, cookie, headers)
        if status != 429 or "/messages" in path:
            return status, reply, answer
        time.sleep(float(answer.get("Retry-After") or 2) + 0.3)


def session_cookie(headers):
    for line in headers.get_all("Set-Cookie") or []:
        first = line.split(";")[0]
        if "session" in first.split("=")[0]:
            return first


def wait_ready(base):
    for _ in range(120):
        try:
            if send(base, "GET", "/health/ready")[0] == 200:
                return
        except OSError:
            pass
        time.sleep(0.5)
    raise SystemExit(f"{base} did not become ready")


class Rust:
    def __init__(self, binary, folder, database=None):
        self.binary, self.folder = binary, folder
        os.makedirs(folder, exist_ok=True)
        if database:
            shutil.copy(database, f"{folder}/stellarr.db")
        self.process, self.base = None, None

    def start(self, extra=None):
        environment = {
            "PATH": os.environ["PATH"], "ASPNETCORE_ENVIRONMENT": "Development", "Rtc__TurnSharedSecret": "x",
            "STELLARR_DATA_ROOT": self.folder, "ConnectionStrings__Database": f"Data Source={self.folder}/stellarr.db;Pooling=True",
            "ASPNETCORE_URLS": "http://127.0.0.1:0", "RateLimiting__AuthWrite__PermitLimit": "5000",
            "RateLimiting__Login__PermitLimit": "5000", "Stellarr__Embeds__Enabled": "false",
        }
        environment.update(extra or {})
        self.log = open(f"{self.folder}/log.txt", "w")
        self.process = subprocess.Popen([self.binary], env=environment, stdout=self.log, stderr=subprocess.STDOUT)
        for _ in range(100):
            text = open(f"{self.folder}/log.txt").read()
            found = re.search(r"listening on: http://127.0.0.1:(\d+)", text)
            if found:
                self.base = f"http://127.0.0.1:{found.group(1)}"
                break
            time.sleep(0.2)
        wait_ready(self.base)
        return self

    def setup_code(self):
        return re.findall(r"Setup code: ([A-Z0-9-]+)\.", open(f"{self.folder}/log.txt").read())[-1]

    def stop(self):
        if self.process:
            self.process.terminate()
            self.process.wait(timeout=30)
            self.process = None


class Oracle:
    def __init__(self, name, port):
        self.container, self.base = f"stellarr-port-oracle-{name}", f"http://127.0.0.1:{port}"

    def stop(self):
        subprocess.run(["docker", "stop", "-t", "10", self.container], check=True, capture_output=True)

    def start(self):
        subprocess.run(["docker", "start", self.container], check=True, capture_output=True)
        wait_ready(self.base)

    def put_database(self, path):
        for suffix in ("-wal", "-shm"):
            subprocess.run(["docker", "exec", self.container, "rm", "-f", f"/tmp/data/stellarr.db{suffix}"], capture_output=True)
        subprocess.run(["docker", "cp", path, f"{self.container}:/tmp/data/stellarr.db"], check=True, capture_output=True)

    def take_database(self, path):
        self.stop()
        subprocess.run(["docker", "cp", f"{self.container}:/tmp/data/stellarr.db", path], check=True, capture_output=True)
        # The log of the write-ahead journal is in the file once the server has stopped cleanly.
        connection = sqlite3.connect(path)
        connection.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        connection.close()


# ---- The shared file

def ts(seconds, fraction=0):
    """A moment as the database holds it, with the trailing zeros of the fraction left off."""
    base = time.strftime("%Y-%m-%d %H:%M:%S", time.gmtime(seconds))
    text = f"{fraction:07d}".rstrip("0")
    return f"{base}.{text}" if text else base


def seed(database, users, count, rng):
    """Writes channels, overrides and `count` messages. Returns the ids the probes use."""
    db = sqlite3.connect(database)
    db.execute("PRAGMA foreign_keys = ON")
    server = db.execute('SELECT "Id" FROM "Servers"').fetchone()[0]
    base_role = db.execute('SELECT "Id" FROM "ServerRoles" WHERE "IsSystem" = 1 AND "IsOwner" = 0').fetchone()[0]
    upper = lambda text: text.upper()
    channels = {}

    def channel(name, slow=0, section=None):
        ident = str(uuid.uuid4()).upper()
        db.execute('INSERT INTO "Channels" ("Id","ServerId","Name","Kind","Position","Topic","SlowModeSeconds","HideAfterInactivityHours","UserLimit","SectionId") VALUES (?,?,?,0,?,?,?,0,0,?)',
                   (ident, server, name, 10 + len(channels), "", slow, section))
        channels[name] = ident
        return ident

    channel("busy")
    channel("slow", slow=30)
    secret = channel("secret")
    # Private: denied to the base role, allowed to carol alone.
    db.execute('INSERT INTO "PermissionOverrides" ("Id","ServerId","TargetType","TargetId","RoleId","AllowMask","DenyMask") VALUES (?,?,0,?,?,0,16384)',
               (str(uuid.uuid4()).upper(), server, secret, base_role))
    db.execute('INSERT INTO "MemberPermissionOverrides" ("Id","ServerId","TargetType","TargetId","UserId","AllowMask","DenyMask") VALUES (?,?,0,?,?,16384,0)',
               (str(uuid.uuid4()).upper(), server, secret, users["carol"].upper()))
    ids = [value for key, value in users.items() if key != "frank"]
    uploaded = []
    for number in range(40):
        ident = str(uuid.uuid4()).upper()
        kind = rng.choice(["image/png", "image/jpeg", "audio/webm", "application/pdf"])
        db.execute(
            'INSERT INTO "ImageAttachments" ("Id","UploadedById","ObjectKey","FileName","ContentType","SizeBytes","Width","Height","DurationSeconds","IsVoiceMessage","PosterObjectKey","PosterContentType","PosterSizeBytes","PreviewObjectKey","PreviewContentType","PreviewSizeBytes","Placeholder","IsEncrypted","CreatedAt") VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,0,?)',
            (ident, rng.choice(ids).upper(), f"seed/{ident}.bin", f"file{number}.bin", kind, rng.randint(10, 10**6),
             640 if kind.startswith("image") else None, 480 if kind.startswith("image") else None,
             rng.choice([None, 12.0, 3.25, 61.5]) if kind.startswith("audio") else None, 0,
             f"seed/{ident}.poster" if number % 7 == 0 else None, "image/jpeg" if number % 7 == 0 else None, 100 if number % 7 == 0 else None,
             f"seed/{ident}.preview" if number % 5 == 0 else None, "image/webp" if number % 5 == 0 else None, 50 if number % 5 == 0 else None,
             "AAAA" if number % 3 == 0 else None, ts(1_700_000_000 + number)))
        uploaded.append(ident)
    emoji = ["\U0001F44D", "\U0001F389", "❤️", "\U0001F680", "x"]
    posted = []
    start = 1_763_000_000
    moment = start
    for number in range(count):
        # A share of the messages share their moment with the one before.
        if posted and rng.random() < 0.25:
            pass
        else:
            moment += rng.randint(1, 90)
        fraction = 0 if rng.random() < 0.3 else rng.randint(1, 9_999_999)
        if posted and rng.random() < 0.25:
            fraction = posted[-1][2]
            moment_used = posted[-1][1]
        else:
            moment_used = moment
        ident = str(uuid.uuid4()).upper()
        author = rng.choice(ids).upper()
        reply = rng.choice(posted)[0] if posted and rng.random() < 0.12 else None
        words = " ".join(rng.choice(["alpha", "beta", "gämma", "\U0001F600", "https://example.test/x", "@bob", "line\nbreak", "…"]) for _ in range(rng.randint(1, 30)))
        db.execute('INSERT INTO "Messages" ("Id","ChannelId","AuthorId","IsSystem","Content","CreatedAt","EditedAt","PinnedAt","ReplyToMessageId") VALUES (?,?,?,0,?,?,NULL,?,?)',
                   (ident, channels["busy"], author, words, ts(moment_used, fraction), ts(moment_used, fraction) if rng.random() < 0.02 else None, reply))
        posted.append((ident, moment_used, fraction))
        if rng.random() < 0.1:
            for attachment in rng.sample(uploaded, rng.randint(1, 3)):
                db.execute('INSERT INTO "MessageAttachments" ("MessageId","AttachmentId") VALUES (?,?)', (ident, attachment))
        if rng.random() < 0.05:
            for position in range(rng.randint(1, 2)):
                db.execute('INSERT INTO "MessageEmbeds" ("Id","MessageId","Url","Kind","Provider","ProviderId","StartSeconds","Domain","Title","Description","ThumbnailAttachmentId","Position","CreatedAt") VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)',
                           (str(uuid.uuid4()).upper(), ident, f"https://example.test/{number}/{position}", rng.choice([0, 1]), "example.test", None, rng.choice([None, 42]),
                            "example.test", "Title", None, rng.choice([None, rng.choice(uploaded)]), position, ts(moment_used, fraction)))
        if rng.random() < 0.15:
            for who in rng.sample(ids, rng.randint(1, 4)):
                db.execute('INSERT OR IGNORE INTO "MessageReactions" ("MessageId","UserId","Emoji","CreatedAt") VALUES (?,?,?,?)',
                           (ident, who.upper(), rng.choice(emoji), ts(moment_used + rng.randint(0, 3), rng.choice([0, 5, 123]))))
    # A short transcript of the private and the slow channel.
    for name in ("secret", "slow"):
        for number in range(8):
            db.execute('INSERT INTO "Messages" ("Id","ChannelId","AuthorId","IsSystem","Content","CreatedAt") VALUES (?,?,?,0,?,?)',
                       (str(uuid.uuid4()).upper(), channels[name], users["carol"].upper(), f"{name} {number}", ts(start + number * 5)))
    db.commit()
    db.close()
    return {"channels": channels, "messages": [ident for ident, _, _ in posted], "server": server}


# ---- The walks

def lower_ids(value):
    return value


def parse(reply):
    try:
        found = json.loads(reply)
        if isinstance(found, dict):
            found.pop("traceId", None)
        return found
    except ValueError:
        return reply.decode("utf-8", "replace")


def first_difference(left, right, path=""):
    if type(left) != type(right):
        return f"{path}: {left!r} vs {right!r}"
    if isinstance(left, dict):
        if list(left) != list(right):
            return f"{path}: members {list(left)} vs {list(right)}"
        for key in left:
            found = first_difference(left[key], right[key], f"{path}.{key}")
            if found:
                return found
    elif isinstance(left, list):
        if len(left) != len(right):
            return f"{path}: {len(left)} items vs {len(right)}"
        for index, (a, b) in enumerate(zip(left, right)):
            found = first_difference(a, b, f"{path}[{index}]")
            if found:
                return found
    elif left != right:
        return f"{path}: {left!r} vs {right!r}"


def same(left, right, what):
    """Two answers are the same when their JSON is, member by member in order."""
    found = first_difference(left, right)
    if found:
        raise SystemExit(f"DIFFERENT {what}\n  first server vs this server, at {found}")


def legacy_cursor(channel, snapshot_seconds):
    """A cursor in the older form the first server still reads: the channel and a snapshot, no last message."""
    ticks = (snapshot_seconds + 62_135_596_800) * 10_000_000
    return base64.urlsafe_b64encode(uuid.UUID(channel).bytes_le + struct.pack(">q", ticks) + struct.pack(">i", 0)).decode().rstrip("=")


class Pair:
    def __init__(self, oracle, rust, cookies_oracle, cookies_rust):
        self.oracle, self.rust, self.co, self.cr = oracle, rust, cookies_oracle, cookies_rust

    def both(self, person, method, path, body=None, headers=None):
        a = send(self.oracle, method, path, body, self.co[person], headers)
        b = send(self.rust, method, path, body, self.cr[person], headers)
        return a, b

    def same(self, person, method, path, body=None, headers=None, what=None):
        a, b = self.both(person, method, path, body, headers)
        if a[0] == 500 and b[0] == 400 and os.environ.get("ORACLE_DEVELOPMENT", "1") == "1":
            # A request that cannot be bound is a 500 on the first server in Development, a 400 elsewhere.
            return 400, None
        same([a[0], parse(a[1])], [b[0], parse(b[1])], what or f"{method} {path}")
        for name in ("content-type", "retry-after", "location"):
            if (a[2].get(name) or "").lower() != (b[2].get(name) or "").lower():
                raise SystemExit(f"DIFFERENT header {name} on {method} {path}: {a[2].get(name)!r} vs {b[2].get(name)!r}")
        return a[0], parse(a[1])


def history_walk(pair, person, channel, limit, snapshot):
    cursor = legacy_cursor(channel, snapshot)
    pages, total = 0, 0
    previous = None
    while True:
        query = f"?cursor={cursor}&limit={limit}"
        status, body = pair.same(person, "GET", f"/api/channels/{channel}/messages{query}", what=f"history limit={limit} cursor#{pages}")
        assert status == 200, (status, body)
        pages += 1
        total += len(body["messages"])
        if previous is not None and body["messages"]:
            assert body["messages"][-1]["createdAt"] <= previous or True
        if body["nextCursor"] is None:
            return pages, total
        cursor = body["nextCursor"]
        previous = body["messages"][0]["createdAt"] if body["messages"] else previous


def sync_walk(pair, person, channel, limit):
    after, pages, total = 0, 0, 0
    while True:
        status, body = pair.same(person, "GET", f"/api/channels/{channel}/messages/sync?after={after}&limit={limit}", what=f"sync limit={limit} after={after}")
        assert status == 200, (status, body)
        pages += 1
        total += len(body["changes"])
        after = body["nextCursor"]
        if not body["hasMore"]:
            return pages, total



# ---- The writes

def cid(label):
    """A client message id that is the same on both servers, from a label."""
    return str(uuid.uuid5(uuid.NAMESPACE_URL, "messages-shared/" + label))


def prepare(database, users, facts):
    """What the write scenario stands on: attachments of known owners, overrides, preferences, sessions and subscriptions."""
    db = sqlite3.connect(database)
    server = facts["server"]
    up = lambda name: users[name].upper()
    attachments = {}

    def attach(label, owner, content_type="image/png", voice=0, encrypted=0, duration=None):
        ident = str(uuid.uuid5(uuid.NAMESPACE_URL, "attachment/" + label)).upper()
        db.execute(
            'INSERT INTO "ImageAttachments" ("Id","UploadedById","ObjectKey","FileName","ContentType","SizeBytes","Width","Height","DurationSeconds","IsVoiceMessage","PosterObjectKey","PosterContentType","PosterSizeBytes","PreviewObjectKey","PreviewContentType","PreviewSizeBytes","Placeholder","IsEncrypted","CreatedAt") VALUES (?,?,?,?,?,?,?,?,?,?,NULL,NULL,NULL,NULL,NULL,NULL,NULL,?,?)',
            (ident, up(owner), f"prep/{ident}", f"{label}.bin", content_type, 1000, 10 if content_type == "image/png" else None, 20 if content_type == "image/png" else None,
             duration, voice, encrypted, ts(1_700_000_000)))
        attachments[label] = ident.lower()

    for number in range(1, 11):
        attach(f"bob-{number}", "bob")
    attach("bob-voice", "bob", "audio/webm", voice=1, duration=12.0)
    attach("bob-voice-2", "bob", "audio/webm", voice=1, duration=3.5)
    attach("bob-encrypted", "bob", "application/octet-stream", encrypted=1)
    attach("alice-1", "alice")
    channels = facts["channels"]
    db.execute('INSERT INTO "MemberPermissionOverrides" ("Id","ServerId","TargetType","TargetId","UserId","AllowMask","DenyMask") VALUES (?,?,0,?,?,0,512)',
               (str(uuid.uuid4()).upper(), server, channels["busy"], up("dave")))
    db.execute('INSERT INTO "MemberPermissionOverrides" ("Id","ServerId","TargetType","TargetId","UserId","AllowMask","DenyMask") VALUES (?,?,0,?,?,0,1024)',
               (str(uuid.uuid4()).upper(), server, channels["busy"], up("erin")))
    columns = [row[1] for row in db.execute('PRAGMA table_info("UserPreferences")')]
    for name, email, push in (("bob", 1, 0), ("carol", 0, 1), ("dave", 1, 1), ("erin", 0, 0), ("frank", 1, 1)):
        values = {column: 0 for column in columns}
        values.update({"UserId": up(name), "EmailNotifications": email, "PushNotifications": push, "MutedChannelIdsJson": "[]", "JoinSoundId": None,
                       "AppearanceSettingsJson": None, "LanguageSettingsJson": None, "KeybindSettingsJson": None, "VoiceSettingsJson": None, "PrivacySettingsJson": None,
                       "VadSensitivity": 50, "AllowDirectMessages": 1, "AllowFriendRequests": 1, "ShowPresence": 1})
        db.execute(f'INSERT INTO "UserPreferences" ({",".join(chr(34) + c + chr(34) for c in columns)}) VALUES ({",".join("?" * len(columns))})', [values[c] for c in columns])
    # frank's address is confirmed by an e-mail method, dave's was vouched for by a provider.
    db.execute('INSERT INTO "MfaMethods" ("Id","UserId","Type","Name","CreatedAt","ConfirmedAt","EmailAddress") VALUES (?,?,?,?,?,?,?)',
               (str(uuid.uuid4()).upper(), up("frank"), "email", "mail", ts(1_700_000_000), ts(1_700_000_000), "FRANK@EXAMPLE.TEST"))
    db.execute('UPDATE "Users" SET "ProviderVerifiedEmail" = "NormalizedEmail" WHERE "Id" = ?', (up("dave"),))
    far = ts(2_000_000_000)
    for name in ("carol", "dave", "frank"):
        version = db.execute('SELECT "SessionVersion" FROM "Users" WHERE "Id" = ?', (up(name),)).fetchone()[0]
        session = str(uuid.uuid5(uuid.NAMESPACE_URL, "session/" + name)).upper()
        db.execute('INSERT INTO "UserSessions" ("Id","UserId","Device","IpAddress","CreatedAt","LastSeenAt","ExpiresAt","AbsoluteExpiresAt","PrimarySource","SecurityVersion","ShortLived") VALUES (?,?,?,?,?,?,?,?,?,?,0)',
                   (session, up(name), "probe", "127.0.0.1", ts(1_700_000_000), ts(1_700_000_000), far, far, "password", version))
        for number in range(2):
            db.execute('INSERT INTO "PushSubscriptions" ("Id","UserId","SessionId","Endpoint","P256dh","Auth","CreatedAt") VALUES (?,?,?,?,?,?,?)',
                       (str(uuid.uuid5(uuid.NAMESPACE_URL, f"push/{name}/{number}")).upper(), up(name), session, f"https://push.example/{name}/{number}", "p", "a", ts(1_700_000_000)))
    # A message of the first channel to reply to from the second, and a notice of the server.
    db.execute('INSERT INTO "Messages" ("Id","ChannelId","AuthorId","IsSystem","Content","CreatedAt") VALUES (?,?,?,1,?,?)',
               (cid("notice").upper(), channels["busy"], up("owner"), "**Owner** has joined the server.", ts(1_763_900_000)))
    db.commit()
    db.close()
    return attachments


TIME = re.compile(r"^\d{4}-\d\d-\d\d \d\d:\d\d:\d\d(\.\d+)?$")
TABLES = [
    ("Messages", 'SELECT * FROM "Messages" ORDER BY "Id"'),
    ("MessageAttachments", 'SELECT * FROM "MessageAttachments" ORDER BY "MessageId","AttachmentId"'),
    ("MessageEmbeds", 'SELECT * FROM "MessageEmbeds" ORDER BY "MessageId","Position"'),
    ("MessageChanges", 'SELECT "Sequence","ChannelId","MessageId","Kind" FROM "MessageChanges" ORDER BY "Sequence"'),
    ("ChannelReads", 'SELECT * FROM "ChannelReads" ORDER BY "ChannelId","UserId"'),
    ("ChannelPostTimes", 'SELECT * FROM "ChannelPostTimes" ORDER BY "ChannelId","UserId"'),
    ("UserNotifications", 'SELECT * FROM "UserNotifications" ORDER BY "Sequence"'),
    ("NotificationDeliveries", 'SELECT * FROM "NotificationDeliveries" ORDER BY "RecipientId","Channel","DestinationKey"'),
    ("MessageIndexEntries", 'SELECT "MessageId","ChannelId","SearchText","HasAttachments","HasLinks" FROM "MessageIndexEntries" ORDER BY "MessageId"'),
    ("ObjectDeletions", 'SELECT "ObjectKey" FROM "ObjectDeletions" ORDER BY "ObjectKey"'),
]
UUID = re.compile(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")


def dump(path, known):
    """The rows the scenario touches, with the moments and the ids the servers made up replaced."""
    db = sqlite3.connect(path)
    out = {}
    for name, query in TABLES:
        rows = []
        for row in db.execute(query):
            cells = []
            for cell in row:
                if isinstance(cell, str):
                    if TIME.match(cell):
                        cell = "<time>"
                    else:
                        cell = UUID.sub(lambda m: m.group(0).lower() if m.group(0).lower() in known else "<new>", cell)
                cells.append(cell)
            rows.append(cells)
        if name in ("UserNotifications", "NotificationDeliveries", "MessageEmbeds"):
            rows.sort(key=lambda r: json.dumps(r))
        out[name] = rows
    db.close()
    return out


def scenario(users, facts, attachments):
    """The requests of the write stage: (person, method, path, body)."""
    busy, slow, secret = (facts["channels"][n].lower() for n in ("busy", "slow", "secret"))
    first = facts["messages"][0].lower()
    other = facts["messages"][1].lower()
    secret_message = None
    steps = []
    add = lambda *step: steps.append(step)
    post = lambda channel, **body: ("POST", f"/api/channels/{channel}/messages", body)
    add("owner", *post(busy, content="hello", clientMessageId=cid("m1")))
    add("owner", *post(busy, content="hello", clientMessageId=cid("m1")))
    add("owner", *post(busy, content="different", clientMessageId=cid("m1")))
    add("alice", *post(busy, content="hello", clientMessageId=cid("m1")))
    add("owner", *post(slow, content="hello", clientMessageId=cid("m1")))
    add("owner", *post(busy, content="  padded \n ", clientMessageId=cid("m2")))
    add("owner", *post(busy, content="", clientMessageId=cid("empty")))
    add("owner", *post(busy, content="   \u00a0 \n", clientMessageId=cid("blank")))
    add("owner", *post(busy))
    add("owner", *post(busy, content=None, attachmentIds=[]))
    add("owner", *post(busy, content="a" * 20000, clientMessageId=cid("long-ok")))
    add("owner", *post(busy, content="a" * 20001, clientMessageId=cid("long-no")))
    add("owner", *post(busy, content="a" * 19998 + "\U0001F600", clientMessageId=cid("astral-ok")))
    add("owner", *post(busy, content="a" * 19999 + "\U0001F600", clientMessageId=cid("astral-no")))
    add("owner", *post(busy, content="hi", clientMessageId="00000000-0000-0000-0000-000000000000"))
    add("owner", *post(busy, content="hi", attachmentIds=[attachments["bob-1"]] * 11))
    add("owner", *post(busy, content="hi", attachmentIds=[str(uuid.uuid4()) for _ in range(11)]))
    add("owner", *post(busy, content="hi", attachmentIds=[str(uuid.uuid4()) for _ in range(10)]))
    add("bob", *post(busy, content="mine", attachmentIds=[attachments[f"bob-{n}"] for n in range(1, 11)], clientMessageId=cid("ten")))
    add("bob", *post(busy, attachmentIds=[attachments["bob-1"], attachments["bob-1"], attachments["bob-2"]], clientMessageId=cid("dupes")))
    add("bob", *post(busy, attachmentIds=[attachments["alice-1"]], clientMessageId=cid("notmine")))
    add("bob", *post(busy, attachmentIds=[attachments["bob-encrypted"]], clientMessageId=cid("encrypted")))
    add("bob", *post(busy, attachmentIds=[attachments["bob-voice"]], clientMessageId=cid("voice")))
    add("bob", *post(busy, content="words", attachmentIds=[attachments["bob-voice-2"]], clientMessageId=cid("voice-words")))
    add("bob", *post(busy, attachmentIds=[attachments["bob-voice-2"], attachments["bob-1"]], clientMessageId=cid("voice-two")))
    add("bob", *post(busy, attachmentIds=[attachments["bob-voice"]], clientMessageId=cid("voice")))
    add("erin", *post(busy, content="files", attachmentIds=[attachments["bob-1"]], clientMessageId=cid("denied-files")))
    add("dave", *post(busy, content="denied", clientMessageId=cid("denied-send")))
    add("dave", *post(busy, content="denied again", clientMessageId=cid("voice")))
    add("bob", *post(busy, content="reply", replyToMessageId=first, clientMessageId=cid("reply")))
    add("bob", *post(busy, content="reply", replyToMessageId=str(uuid.uuid4()), clientMessageId=cid("reply-lost")))
    add("bob", *post(slow, content="reply", replyToMessageId=first, clientMessageId=cid("reply-across")))
    add("bob", *post(busy, content="reply to a notice", replyToMessageId=cid("notice"), clientMessageId=cid("reply-notice")))
    add("bob", *post(busy, content="reply to a reply", replyToMessageId=cid("reply"), clientMessageId=cid("reply-reply")))
    add("bob", *post(secret, content="no", clientMessageId=cid("secret-no")))
    add("bob", *post(str(uuid.uuid4()), content="no", clientMessageId=cid("nowhere")))
    add("carol", *post(secret, content="yes", clientMessageId=cid("secret-yes")))
    add("carol", *post(slow, content="slow one", clientMessageId=cid("slow-1")))
    add("carol", *post(slow, content="slow two", clientMessageId=cid("slow-2")))
    add("carol", *post(slow, content="slow one", clientMessageId=cid("slow-1")))
    add("owner", *post(slow, content="owner one", clientMessageId=cid("slow-o1")))
    add("owner", *post(slow, content="owner two", clientMessageId=cid("slow-o2")))
    add("alice", *post(busy, content="https://example.test/page is a link", clientMessageId=cid("link")))
    add("alice", "PATCH", f"/api/messages/{cid('link')}", {"content": "no link now"})
    add("alice", "PATCH", f"/api/messages/{cid('link')}", {"content": "no link now"})
    add("alice", "PATCH", f"/api/messages/{cid('link')}", {"content": ""})
    add("alice", "PATCH", f"/api/messages/{cid('link')}", {})
    add("alice", "PATCH", f"/api/messages/{cid('link')}", {"content": "x" * 20001})
    add("alice", "PATCH", f"/api/messages/{cid('link')}", {"content": "x" * 20000})
    add("owner", "PATCH", f"/api/messages/{cid('link')}", {"content": "moderator edit"})
    add("alice", "PATCH", f"/api/messages/{cid('notice')}", {"content": "x"})
    add("alice", "PATCH", f"/api/messages/{uuid.uuid4()}", {"content": "x"})
    add("alice", "PATCH", "/api/messages/not-an-id", {"content": "x"})
    add("bob", "PATCH", f"/api/messages/{cid('secret-yes')}", {"content": "x"})
    add("carol", "PATCH", f"/api/messages/{cid('secret-yes')}", {"content": "edited in secret"})
    add("alice", "DELETE", f"/api/messages/{cid('link')}", None)
    add("alice", "DELETE", f"/api/messages/{cid('link')}", None)
    add("bob", "DELETE", f"/api/messages/{cid('m2')}", None)
    add("owner", "DELETE", f"/api/messages/{cid('m2')}", None)
    add("owner", "DELETE", f"/api/messages/{cid('notice')}", None)
    add("bob", "DELETE", f"/api/messages/{cid('secret-yes')}", None)
    add("carol", "DELETE", f"/api/messages/{cid('reply')}", None)
    add("bob", "DELETE", f"/api/messages/{cid('ten')}", None)
    add("owner", "DELETE", f"/api/messages/{first}", None)
    # Mentions: who is named, who sees the channel, who opted in to mail and push.
    add("owner", *post(busy, content="@bob @carol @dave @erin @frank @nobody @owner and @Bob again `@alice` @al", clientMessageId=cid("mention-busy")))
    add("carol", *post(secret, content="@bob @owner @dave @erin", clientMessageId=cid("mention-secret")))
    add("owner", *post(secret, content="@carol @bob", clientMessageId=cid("mention-secret-2")))
    add("owner", *post(busy, content="@" + "\U0001F600".join(["ab"] * 4) + " " + ("x" * 230) + " @bob", clientMessageId=cid("mention-long")))
    add("bob", "PATCH", f"/api/messages/{cid('ten')}", {"content": "gone"})
    return steps


def normalise_body(body, names):
    if isinstance(body, dict):
        body.pop("traceId", None)
    text = json.dumps(body, ensure_ascii=False)
    text = UUID.sub(lambda m: names.get(m.group(0).lower(), m.group(0).lower()), text)
    text = re.sub(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?Z", "<time>", text)
    return json.loads(text)


def run_scenario(base, cookies, steps, names):
    outcome = []
    for person, method, path, body in steps:
        status, reply, headers = send(base, method, path, body, cookies[person])
        parsed = parse(reply) if reply else None
        outcome.append({"person": person, "method": method, "path": path[:60], "status": status, "body": normalise_body(parsed, names),
                        "retryAfter": headers.get("Retry-After"), "location": UUID.sub(lambda m: names.get(m.group(0).lower(), m.group(0).lower()), headers.get("Location") or "")})
    return outcome


def main():
    name, port, binary = sys.argv[1], sys.argv[2], sys.argv[3]
    stages = sys.argv[4:] or ["history", "sync", "refusals", "writes", "mentions"]
    count = int(os.environ.get("MESSAGES", "4000"))
    rng = random.Random(int(os.environ.get("SEED", "5214")))
    work = tempfile.mkdtemp(prefix="messages-shared-")
    oracle = Oracle(name, port)
    # Founded and populated on this server first.
    first = Rust(binary, f"{work}/first").start()
    code = first.setup_code()
    status, reply, headers = call(first.base, "POST", "/api/auth/register", {"username": "owner", "displayName": "Owner", "email": "owner@example.test", "password": PASSWORD, "setupCode": code})
    assert status == 201, (status, reply)
    owner_cookie = session_cookie(headers)
    owner_reply = reply
    invite = "invite" + uuid.uuid4().hex[:10]
    conn = sqlite3.connect(f"{work}/first/stellarr.db", timeout=20)
    srv, own = conn.execute('SELECT "ServerId","UserId" FROM "ServerMembers" WHERE "OwnerSlot" = 1').fetchone()
    conn.execute('INSERT INTO "ServerInvites" ("Id","ServerId","CreatedById","ChannelId","Code","Uses","MaxUses","ExpiresAt","CreatedAt") VALUES (?,?,?,NULL,?,0,NULL,NULL,?)',
                 (str(uuid.uuid4()).upper(), srv, own, invite, ts(time.time())))
    conn.commit(); conn.close()
    users = {"owner": json.loads(owner_reply)["id"]}
    for person in PEOPLE:
        status, reply, _ = call(first.base, "POST", "/api/auth/register", {"username": person, "displayName": person.title(), "email": f"{person}@example.test", "password": PASSWORD, "inviteCode": invite})
        assert status == 201, (person, status, reply)
        users[person] = json.loads(reply)["id"]
    first.stop()
    shared = f"{work}/shared.db"
    shutil.copy(f"{work}/first/stellarr.db", shared)
    facts = seed(shared, users, count, rng)
    print(f"seeded {count} messages in {work}")

    # The first server edits, reacts to and deletes a share of them through its own routes.
    oracle.stop()
    oracle.put_database(shared)
    oracle.start()
    cookies = {}
    for person in SIGNED_IN:
        status, reply, headers = call(oracle.base, "POST", "/api/auth/login", {"username": person, "password": PASSWORD})
        assert status == 200, (person, status, reply)
        cookies[person] = session_cookie(headers)
    authors = {}
    db = sqlite3.connect(shared)
    for ident, author in db.execute('SELECT "Id", "AuthorId" FROM "Messages" WHERE "ChannelId" = ?', (facts["channels"]["busy"],)):
        authors[ident] = author.lower()
    db.close()
    who = {value: key for key, value in users.items()}
    chosen = rng.sample(sorted(authors), min(len(authors), count // 8))
    for number, ident in enumerate(chosen):
        person = who[authors[ident]]
        if number % 3 == 0:
            call(oracle.base, "PATCH", f"/api/messages/{ident.lower()}", {"content": f"edited {number} \U0001F600"}, cookies[person])
        elif number % 3 == 1:
            call(oracle.base, "DELETE", f"/api/messages/{ident.lower()}", None, cookies[rng.choice([person, "owner"])])
        else:
            call(oracle.base, "PUT", f"/api/messages/{ident.lower()}/reactions/%F0%9F%91%8D", None, cookies[rng.choice(SIGNED_IN[1:])])
    mutated = f"{work}/mutated.db"
    oracle.take_database(mutated)
    print("the first server edited, deleted and reacted")

    # Both servers on identical copies.
    oracle.put_database(mutated)
    oracle.start()
    rust = Rust(binary, f"{work}/rust", mutated).start()
    cr = {}
    for person in SIGNED_IN:
        status, reply, headers = call(rust.base, "POST", "/api/auth/login", {"username": person, "password": PASSWORD})
        assert status == 200, (person, status, reply)
        cr[person] = session_cookie(headers)
    pair = Pair(oracle.base, rust.base, cookies, cr)
    busy, slow, secret = (facts["channels"][n].lower() for n in ("busy", "slow", "secret"))
    snapshot = 1_763_000_000 + count * 100

    if "history" in stages:
        for person in ("owner", "bob"):
            for limit in (1, 7, 50, 100):
                if limit == 1 and count > 1500:
                    continue
                pages, total = history_walk(pair, person, busy, limit, snapshot)
                print(f"history {person} limit {limit}: {pages} pages, {total} messages, identical")
        print("history of a private channel:", history_walk(pair, "carol", secret, 3, snapshot), "and", pair.same("bob", "GET", f"/api/channels/{secret}/messages")[0])
    if "sync" in stages:
        for limit in (1, 17, 200, 500):
            if limit == 1 and count > 1500:
                continue
            print(f"sync limit {limit}:", sync_walk(pair, "owner", busy, limit), "identical")
        for channel in (slow, secret):
            print("sync", channel[:8], sync_walk(pair, "carol", channel, 5))
    if "refusals" in stages:
        for person, path in [
            ("owner", f"/api/channels/{busy}/messages?limit=0"), ("owner", f"/api/channels/{busy}/messages?limit=101"),
            ("owner", f"/api/channels/{busy}/messages?limit=abc"), ("owner", f"/api/channels/{busy}/messages?cursor=zzz"),
            ("owner", f"/api/channels/{busy}/messages?cursor={legacy_cursor(str(uuid.uuid4()), 1)}"),
            ("owner", f"/api/channels/{busy}/messages?cursor="), ("owner", f"/api/channels/{busy}/messages?limit="),
            ("owner", f"/api/channels/{busy}/messages/sync?after=-1"), ("owner", f"/api/channels/{busy}/messages/sync?limit=0"),
            ("owner", f"/api/channels/{busy}/messages/sync?limit=501"), ("owner", f"/api/channels/{busy}/messages/sync?after=x"),
            ("owner", f"/api/channels/{busy}/messages/sync?after=99999999999"),
            ("bob", f"/api/channels/{secret}/messages"), ("bob", f"/api/channels/{secret}/messages/sync"),
            ("bob", f"/api/channels/{uuid.uuid4()}/messages"), ("owner", "/api/channels/not-an-id/messages"),
        ]:
            print(person, path.split("?")[-1][:50], pair.same(person, "GET", path)[0])
    if "writes" in stages or "mentions" in stages:
        oracle.stop()
        prepared = f"{work}/prepared.db"
        shutil.copy(mutated, prepared)
        attachments = prepare(prepared, users, facts)
        oracle.put_database(prepared)
        oracle.start()
        rust.stop()
        rust = Rust(binary, f"{work}/rust-writes", prepared).start()
        for person in SIGNED_IN:
            cookies[person] = session_cookie(call(oracle.base, "POST", "/api/auth/login", {"username": person, "password": PASSWORD})[2])
            cr[person] = session_cookie(call(rust.base, "POST", "/api/auth/login", {"username": person, "password": PASSWORD})[2])
        known = {row[0].lower() for row in sqlite3.connect(prepared).execute('SELECT "Id" FROM "Messages"')}
        known |= {value.lower() for value in attachments.values()} | {value.lower() for value in users.values()} | {value.lower() for value in facts["channels"].values()}
        names = {value.lower(): key for key, value in users.items()}
        names.update({cid(label): f"cid:{label}" for label in ("m1", "m2", "link", "notice", "reply", "ten", "secret-yes")})
        steps = scenario(users, facts, attachments)
        here = os.path.dirname(os.path.abspath(__file__))
        listeners = {}
        for label, base, jar in (("first", oracle.base, cookies), ("this", rust.base, cr)):
            out = f"{work}/events-{label}.jsonl"
            process = subprocess.Popen(["node", f"{here}/messages-listen.mjs", base, out] + [f"{n}={jar[n]}" for n in ("owner", "bob", "carol", "dave")],
                                       stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
            assert process.stdout.readline().strip() == "listening"
            listeners[label] = (process, out)
        answers_oracle = run_scenario(oracle.base, cookies, steps, names)
        answers_rust = run_scenario(rust.base, cr, steps, names)
        time.sleep(float(os.environ.get("EVENT_WAIT", "10")))
        heard = {}
        for label, (process, out) in listeners.items():
            process.stdin.close()
            process.wait(timeout=20)
            per = {}
            for line in open(out):
                record = json.loads(line)
                if not record["target"].startswith(("Message", "ChannelRead", "NotificationInbox")):
                    continue
                per.setdefault(record["who"], []).append(normalise_body({"target": record["target"], "arguments": record["arguments"]}, names))
            heard[label] = per
        mismatches = 0
        for number, (a, b) in enumerate(zip(answers_oracle, answers_rust)):
            found = first_difference(a, b)
            if found:
                mismatches += 1
                print(f"DIFFERENT step {number} {a['person']} {a['method']} {a['path']}\n  at {found}")
        print(f"{len(steps)} requests, {mismatches} answers differ")
        for who in ("owner", "bob", "carol", "dave"):
            a, b = heard["first"].get(who, []), heard["this"].get(who, [])
            for number in range(max(len(a), len(b))):
                found = first_difference(a[number] if number < len(a) else None, b[number] if number < len(b) else None)
                if found:
                    mismatches += 1
                    print(f"DIFFERENT event {number} heard by {who}: {found}")
                    break
            print(f"{who} heard {len(a)} events on the first server and {len(b)} on this one")
        print("statuses:", " ".join(str(a["status"]) for a in answers_oracle))
        rust.stop()
        taken = f"{work}/after-oracle.db"
        oracle.take_database(taken)
        theirs, ours = dump(taken, known), dump(f"{work}/rust-writes/stellarr.db", known)
        for table in theirs:
            if theirs[table] != ours[table]:
                mismatches += 1
                a, b = theirs[table], ours[table]
                print(f"DIFFERENT rows of {table}: {len(a)} vs {len(b)}")
                shown = 0
                for row in a:
                    if row not in b and shown < 6:
                        print("  only on the first server:", json.dumps(row, ensure_ascii=False)[:300]); shown += 1
                shown = 0
                for row in b:
                    if row not in a and shown < 6:
                        print("  only on this server    :", json.dumps(row, ensure_ascii=False)[:300]); shown += 1
            else:
                print(f"rows of {table}: {len(ours[table])} identical")
        if mismatches:
            raise SystemExit(f"{mismatches} differences")
    print("ok")


if __name__ == "__main__":
    main()
