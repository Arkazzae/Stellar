"""Every table of two databases that started as one file, row by row.

Ids, moments and the hashes and tokens that differ by nature are replaced, the
rows of each table sorted, and the two compared. Prints the tables that differ
and what differs in them.
"""
import re
import sqlite3
import sys

first, second, seeded = sys.argv[1:4]
UUID = re.compile(r"[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}")
TIME = re.compile(r"\d{4}-\d\d-\d\d[ T]\d\d:\d\d:\d\d(\.\d+)?Z?")
CODE = re.compile(r"\b[0-9a-f]{16}\b")
# Written afresh by every sign-in, whichever server it is.
SKIP = {"UserSessions", "SecurityEvents", "TrustedDevices", "sqlite_sequence"}


def ids_of(path):
    """Every id the seeded file holds: those are the same in both databases."""
    connection = sqlite3.connect(path)
    found = set()
    for (table,) in connection.execute("SELECT name FROM sqlite_master WHERE type = 'table'").fetchall():
        try:
            for row in connection.execute(f'SELECT * FROM "{table}"'):
                for value in row:
                    if isinstance(value, str):
                        found.update(m.upper() for m in UUID.findall(value))
        except sqlite3.Error:
            pass
    return found


OLD = ids_of(seeded)


def scrub(value):
    if isinstance(value, bytes):
        return "<bytes>"
    if not isinstance(value, str):
        return value
    value = UUID.sub(lambda m: m.group(0).upper() if m.group(0).upper() in OLD else "<new>", value)
    value = CODE.sub("<code>", value)
    return TIME.sub("<time>", value)


def dump(path):
    connection = sqlite3.connect(path)
    tables = [r[0] for r in connection.execute("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")]
    result = {}
    for table in tables:
        if table in SKIP or table.startswith("MessageSearch") or table.startswith("sqlite_"):
            continue
        try:
            rows = connection.execute(f'SELECT * FROM "{table}"').fetchall()
        except sqlite3.Error:
            continue
        result[table] = sorted((tuple(scrub(v) for v in row) for row in rows), key=repr)
    return result


a, b = dump(first), dump(second)
different = 0
for table in sorted(set(a) | set(b)):
    left, right = a.get(table, []), b.get(table, [])
    if left == right:
        continue
    different += 1
    print(f"{table}: {len(left)} rows in the first server, {len(right)} in this one")
    only_left = [r for r in left if r not in right][:5]
    only_right = [r for r in right if r not in left][:5]
    for row in only_left:
        print("  first :", row)
    for row in only_right:
        print("  this  :", row)
print(f"{different} tables differ of {len(set(a) | set(b))}")
