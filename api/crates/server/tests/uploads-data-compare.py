#!/usr/bin/env python3
"""Compares what two servers left in their data folders after the same requests.

  uploads-data-compare.py <first-server-data-folder> <this-server-data-folder>

Each folder holds `stellarr.db` and `files/`. Object keys are compared with
their random parts replaced, the bytes of every object by digest, and the rows
of the tables an upload writes with ids and times replaced. Exits 1 when they
differ. To get the first server's folder out of its container:

  docker stop stellarr-port-oracle-<name>; docker cp stellarr-port-oracle-<name>:/tmp/data <folder>
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
    "ImageAttachments": 'SELECT * FROM "ImageAttachments"',
    "ObjectDeletions": 'SELECT "ObjectKey" FROM "ObjectDeletions"',
    "ServerAuditEntries": 'SELECT "Action", "Summary", "TargetType", "TargetName", "ChangesJson" FROM "ServerAuditEntries" WHERE "Action" LIKE \'server.icon%\'',
    "Users": 'SELECT "Username", "AvatarUrl" IS NULL, "BannerUrl" IS NULL, "WallpaperUrl" IS NULL FROM "Users"',
    "Servers": 'SELECT "IconUrl" IS NULL FROM "Servers"',
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
