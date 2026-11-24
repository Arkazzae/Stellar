#!/usr/bin/env python3
"""Compares what two servers left in their data folders after the same requests.

  custom-data-compare.py <first-server-data-folder> <this-server-data-folder>

Each folder holds `stellarr.db` and `files/`. Object keys are compared with
their random parts replaced, the bytes of every object by digest, and the rows
of the tables the custom emoji, the soundboard, the own profile and the client
settings write, with ids and times replaced. Exits 1 when they differ.
"""
import hashlib
import os
import re
import sqlite3
import sys
from collections import Counter

UUID = re.compile(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
HEX32 = re.compile(r"[0-9a-f]{32}")
MONTH = re.compile(r"/\d{4}/\d{2}/")
TIME = re.compile(r"\d{4}-\d\d-\d\d[ T]\d\d:\d\d:\d\d(\.\d+)?Z?")


def scrub(value):
    if not isinstance(value, str):
        return value
    value = UUID.sub("<id>", value)
    value = HEX32.sub("<n>", value)
    value = MONTH.sub("/<month>/", value)
    return TIME.sub("<time>", value)


def objects(folder):
    found = Counter()
    root = os.path.join(folder, "files")
    for base, _, names in os.walk(root):
        for name in names:
            path = os.path.join(base, name)
            key = scrub(os.path.relpath(path, root))
            with open(path, "rb") as f:
                found[(key, hashlib.sha256(f.read()).hexdigest())] += 1
    return found


TABLES = {
    "ServerEmoji": 'SELECT "Shortcode", "ObjectKey", "ContentType", "SizeBytes", "CreatedAt" IS NULL FROM "ServerEmoji"',
    "SoundboardSounds": 'SELECT "Name", "Glyph", "TrimStartSeconds", "TrimEndSeconds", "Position", typeof("TrimStartSeconds") FROM "SoundboardSounds"',
    "ImageAttachments": 'SELECT "ObjectKey", "FileName", "ContentType", "SizeBytes", "DurationSeconds" FROM "ImageAttachments"',
    "ObjectDeletions": 'SELECT "ObjectKey" FROM "ObjectDeletions"',
    "UserPreferences": 'SELECT u."Username", p."DesktopNotifications", p."MentionPopups", p."CallRingtone", p."EmailNotifications", p."PushNotifications", p."MentionsOnly",'
                       ' p."MessageSound", p."JoinLeaveSound", p."AllowDirectMessages", p."AllowFriendRequests", p."ShowPresence", p."DeveloperMode", p."ReducedMotion",'
                       ' p."MutedChannelIdsJson", p."VadSensitivity", p."JoinSoundId" IS NULL, p."AppearanceSettingsJson", p."LanguageSettingsJson", p."KeybindSettingsJson",'
                       ' p."VoiceSettingsJson", p."PrivacySettingsJson" FROM "UserPreferences" p JOIN "Users" u ON u."Id" = p."UserId"',
    "Users": 'SELECT "Username", "DisplayName", "CustomStatus", "Bio", "Pronouns" FROM "Users"',
    "Servers": 'SELECT "SoundboardEveryoneCanPlay", "SoundboardRateLimited" FROM "Servers"',
    "ServerAuditEntries": 'SELECT "Action", "Summary", "TargetType", "TargetName", "ChangesJson" FROM "ServerAuditEntries"'
                          ' WHERE "Action" LIKE \'emoji.%\' OR "Action" LIKE \'sound%\'',
}


def rows(folder):
    db = sqlite3.connect(os.path.join(folder, "stellarr.db"))
    out = {}
    for table, sql in TABLES.items():
        out[table] = Counter(tuple(scrub(v) for v in row) for row in db.execute(sql))
    return out


def main():
    first, ours = sys.argv[1], sys.argv[2]
    bad = 0
    a, b = objects(first), objects(ours)
    for key in sorted(set(a) | set(b)):
        if a[key] != b[key]:
            bad += 1
            print(f"objects: {key}: first {a[key]}, here {b[key]}")
    ra, rb = rows(first), rows(ours)
    for table in TABLES:
        for row in sorted(set(ra[table]) | set(rb[table]), key=repr):
            if ra[table][row] != rb[table][row]:
                bad += 1
                print(f"{table}: {row}: first {ra[table][row]}, here {rb[table][row]}")
    print(f"{sum(a.values())} objects, {sum(sum(c.values()) for c in ra.values())} rows compared; {bad} differ")
    sys.exit(1 if bad else 0)


main()
