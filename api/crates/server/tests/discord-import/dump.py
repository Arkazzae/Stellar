#!/usr/bin/env python3
"""Writes a database as text that two imports of the same Discord server into
copies of one database can be compared by: what an import makes at random (its
ids, the moments it ran at, a colour taken from an id) is replaced by what it
stands for, and rows are sorted.

    dump.py DB OUT.txt
"""
import re
import sqlite3
import sys

TIMES = {
    ("Users", "CreatedAt"), ("DiscordImports", "StartedAt"), ("DiscordImportChannels", "CompletedAt"), ("DiscordImportChannels", "LinkedAt"),
    ("DiscordImportAccounts", "CreatedAt"), ("MessageChanges", "CreatedAt"),
    ("Channels", "CreatedAt"), ("Sections", "CreatedAt"),
}
# the journal's sequence counts every insert, which an interrupted run repeats
DROPPED = {("MessageChanges", "Sequence")}
SHADOW = re.compile(r".*_(content|data|idx|docsize|config|segments|segdir|stat)$")


def main(db_path, out_path):
    db = sqlite3.connect(db_path)
    tables = [r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
              if not r[0].startswith("sqlite_") and not SHADOW.match(r[0])]
    names = {}

    def label(sql, prefix):
        for key, name in db.execute(sql):
            if key is not None:
                names[key] = prefix + str(name)

    label("SELECT Id, Username FROM Users WHERE Origin IS NOT NULL", "user:")
    label("SELECT SectionId, DiscordId FROM DiscordImportSections WHERE SectionId IS NOT NULL", "section:")
    label("SELECT ChannelId, DiscordId FROM DiscordImportChannels WHERE ChannelId IS NOT NULL AND Kind = 1", "channel-of-thread:")
    label("SELECT ChannelId, DiscordId FROM DiscordImportChannels WHERE ChannelId IS NOT NULL AND Kind = 0", "channel:")
    for number, (key,) in enumerate(db.execute("SELECT Id FROM DiscordImports ORDER BY StartedAt, Id")):
        names[key] = "import:%d" % number

    lines = []
    for table in tables:
        columns = [r[1] for r in db.execute('PRAGMA table_info("%s")' % table)]
        virtual = db.execute("SELECT sql FROM sqlite_master WHERE name = ?", (table,)).fetchone()[0] or ""
        if "VIRTUAL" in virtual.upper():
            count = db.execute('SELECT COUNT(*) FROM "%s"' % table).fetchone()[0]
            lines.append("== %s (virtual) rows=%d" % (table, count))
            continue
        rows = []
        for row in db.execute('SELECT * FROM "%s"' % table):
            cells = []
            for column, value in zip(columns, row):
                if (table, column) in DROPPED:
                    continue
                if (table, column) in TIMES and value is not None:
                    value = "T"
                elif table == "Users" and column == "AvatarColor" and row[columns.index("Origin")] is not None:
                    value = "avatar"
                elif isinstance(value, str) and value in names:
                    value = names[value]
                elif isinstance(value, bytes):
                    value = value.hex()
                cells.append("%s=%r" % (column, value))
            rows.append(" ".join(cells))
        lines.append("== %s rows=%d" % (table, len(rows)))
        lines.extend(sorted(rows))
    with open(out_path, "w") as handle:
        handle.write("\n".join(lines) + "\n")


main(sys.argv[1], sys.argv[2])
