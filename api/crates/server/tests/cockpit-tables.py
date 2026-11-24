"""Every table of two databases that started as one file, row by row, for the
operator panel's comparison (cockpit-shared-database.sh).

  python3 -I cockpit-tables.py first.db this.db seeded.db

As servers-tables.py, with what the panel writes kept in: the sessions and
the security history are compared too (both servers saw the same sign-ins),
and so are the tables the panel names below, which are printed even when
they are the same. Ids that are new, hashes, sealed values and the address a
request came from are replaced; a moment the seeded file holds is kept as it
is, a new one keeps only its shape (whether it has a fraction, and a Z).
A sealed secret differs by nature: only its name and that it is there count.
"""
import re
import sqlite3
import sys
from collections import Counter

first, second, seeded = sys.argv[1:4]
UUID = re.compile(r"[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}")
TIME = re.compile(r"(\d{4}-\d\d-\d\d[ T]\d\d:\d\d:\d\d)(\.\d+)?(Z|[+-]\d\d:\d\d)?")
HUE = re.compile(r"oklch\(0\.58 0\.14 \d+\)")
SKIP = {"sqlite_sequence"}
# Values that differ by nature, by column.
HIDDEN = {"PasswordHash", "TokenHash", "CodeHash", "ProtectedSecret", "ProtectedValue", "SecretHash", "Hash"}
NAMED = ["Users", "ServerMembers", "ServerMemberRoles", "ServerRoles", "ServerAuditEntries", "SecurityEvents",
         "UserSessions", "OperationsSettings", "InstanceSecrets", "ExternalIdentities", "MfaMethods",
         "CliRecoveryCredentials", "IdentityProviderSettings", "TrustedDevices", "RecoveryCodes", "Servers"]


def seeded_values(path):
    """Every id and moment the seeded file holds: those are the same in both databases."""
    connection = sqlite3.connect(path)
    ids, moments = set(), set()
    for (table,) in connection.execute("SELECT name FROM sqlite_master WHERE type = 'table'").fetchall():
        try:
            for row in connection.execute(f'SELECT * FROM "{table}"'):
                for value in row:
                    if isinstance(value, str):
                        ids.update(m.upper() for m in UUID.findall(value))
                        moments.update(m.group(0) for m in TIME.finditer(value))
        except sqlite3.Error:
            pass
    return ids, moments


OLD_IDS, OLD_MOMENTS = seeded_values(seeded)


def moment(match):
    if match.group(0) in OLD_MOMENTS:
        return match.group(0)
    # A fresh moment loses trailing zeros by chance: only whether it has a fraction is kept.
    return f"<time{'.f' if match.group(2) else ''}{match.group(3) or ''}>"


def scrub(column, value):
    if isinstance(value, bytes):
        return "<bytes>"
    if column in HIDDEN or column.endswith("Hash"):
        return None if value is None else "<hidden>"
    if column == "IpAddress":
        return "<ip>"
    if not isinstance(value, str):
        return value
    value = UUID.sub(lambda m: m.group(0).upper() if m.group(0).upper() in OLD_IDS else "<new>", value)
    value = HUE.sub("oklch(<hue>)", value)
    return TIME.sub(moment, value)


def dump(path):
    connection = sqlite3.connect(path)
    tables = [r[0] for r in connection.execute("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")]
    result = {}
    for table in tables:
        if table in SKIP or table.startswith("MessageSearch") or table.startswith("sqlite_"):
            continue
        try:
            cursor = connection.execute(f'SELECT * FROM "{table}"')
        except sqlite3.Error:
            continue
        columns = [d[0] for d in cursor.description]
        rows = [tuple(scrub(c, v) for c, v in zip(columns, row)) for row in cursor.fetchall()]
        result[table] = (columns, sorted(rows, key=repr))
    return result


def shown(row):
    """A row as printed: a long text keeps its start and its end."""
    return tuple(f"{v[:50]}...({len(v)} characters)...{v[-12:]!r}" if isinstance(v, str) and len(v) > 90 else v for v in row)


a, b = dump(first), dump(second)
different = 0
for table in sorted(set(a) | set(b)):
    columns, left = a.get(table, ([], []))
    _, right = b.get(table, ([], []))
    if left == right:
        if table in NAMED:
            print(f"{table}: the same ({len(left)} rows)")
        continue
    different += 1
    print(f"{table}: {len(left)} rows in the first server, {len(right)} in this one   [{', '.join(columns)}]")
    # Rows that differ, counted: the same row can be there more than once.
    surplus_left = Counter(left) - Counter(right)
    surplus_right = Counter(right) - Counter(left)
    for side, surplus in (("first :", surplus_left), ("this  :", surplus_right)):
        for row, count in list(surplus.items())[:8]:
            print(f"  {side}", shown(row), f"(x{count})" if count > 1 else "")
print(f"{different} tables differ of {len(set(a) | set(b))}")
