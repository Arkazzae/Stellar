"""The rows the conversations of driver.mjs left in two databases, one for each
server. Ids are replaced by what they belong to, moments and secrets by
markers, and the rows of each table are compared as sorted lists. Prints what
differs, and exits 1 if anything does.

  python3 tables.py <first server's database> <this server's database>
"""
import re
import sqlite3
from collections import Counter
import sys

UUID = re.compile(r"[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}")
TIME = re.compile(r"\d{4}-\d\d-\d\d[ T]\d\d:\d\d:\d\d(\.\d+)?Z?")
HASH = re.compile(r"\b[0-9A-F]{64}\b")
# Each server has a provider of its own for the key rotation (p44, p45).
PROVIDER = re.compile(r"\bp4[45]\b")
ADDRESS = re.compile(r"https://[0-9.]+:\d+")

QUERIES = {
    "ExternalIdentities": 'SELECT u."Username", i."ProviderId", i."Issuer", i."Subject" FROM "ExternalIdentities" i JOIN "Users" u ON u."Id" = i."UserId"',
    "PendingExternalIdentities": 'SELECT "ProviderId","Issuer","Subject","Email","EmailVerified","DisplayName","PreferredUsername","Amr","AuthTime" IS NOT NULL,"ShortLived",'
                                 'CAST(ROUND((julianday("ExpiresAt") - julianday("CreatedAt")) * 1440) AS INTEGER) FROM "PendingExternalIdentities"',
    "UserSessions": 'SELECT u."Username", s."PrimarySource", s."MfaEvidence", s."MfaCompletedAt" IS NOT NULL, s."ShortLived", s."RevokedAt" IS NOT NULL, s."SecurityVersion", '
                    'CASE WHEN s."AuthenticatedAt" IS NULL THEN \'never\' WHEN ABS(julianday(s."AuthenticatedAt") - julianday(s."CreatedAt")) * 86400 < 5 THEN \'at sign-in\' '
                    'WHEN s."AuthenticatedAt" > s."CreatedAt" THEN \'moved forward\' ELSE \'before\' END '
                    'FROM "UserSessions" s JOIN "Users" u ON u."Id" = s."UserId"',
    "AuthChallenges": 'SELECT u."Username", c."Purpose", c."Attempts", c."MaxAttempts", c."ConsumedAt" IS NOT NULL, c."ShortLived", c."PrimarySource", c."PendingIdentityId" IS NOT NULL '
                      'FROM "AuthChallenges" c JOIN "Users" u ON u."Id" = c."UserId"',
    "SecurityEvents": 'SELECT u."Username", e."Kind", e."Detail", e."Device" FROM "SecurityEvents" e JOIN "Users" u ON u."Id" = e."UserId"',
    "SecurityNotifications": 'SELECT u."Username", e."Kind", n."EmailAddress" FROM "SecurityNotifications" n JOIN "SecurityEvents" e ON e."Id" = n."SecurityEventId" JOIN "Users" u ON u."Id" = n."UserId"',
    "Users": 'SELECT "Username", "PasswordHash" IS NOT NULL, "ProviderVerifiedEmail", "SessionVersion" FROM "Users"',
}


# Rows that differ on purpose (api/README.md, "Single sign-on: where it differs from the first
# server"): the table, the server that has the row, the row's start, and why.
KNOWN = [
    ("PendingExternalIdentities", "first", ("p13", "<provider>/p13", "s-p13"),
     "an unsigned ID token is refused here, so no identity waits for the provider that sends one"),
]


def known(table, side, row):
    for name, owner, start, reason in KNOWN:
        if name == table and owner == side and tuple(row[: len(start)]) == start:
            return reason
    return None


def scrub(value):
    if isinstance(value, str):
        value = HASH.sub("<hash>", UUID.sub("<id>", value))
        value = PROVIDER.sub("pK", value)
        return TIME.sub("<time>", ADDRESS.sub("<provider>", value))
    return value


def dump(path):
    db = sqlite3.connect(path)
    result = {}
    for table, query in QUERIES.items():
        try:
            rows = db.execute(query).fetchall()
        except sqlite3.Error as error:
            result[table] = [(f"unreadable: {error}",)]
            continue
        result[table] = sorted((tuple(scrub(v) for v in row) for row in rows), key=repr)
    return result


first, second = dump(sys.argv[1]), dump(sys.argv[2])
different = False
for table in QUERIES:
    # As multisets: a row that one server wrote twice and the other once is a difference.
    only_first = list((Counter(first[table]) - Counter(second[table])).elements())
    only_second = list((Counter(second[table]) - Counter(first[table])).elements())
    for side, rows in (("first", only_first), ("this", only_second)):
        for row in [r for r in rows if known(table, side, r)]:
            print(f"known     {table}: {side} server only {row}: {known(table, side, row)}")
            rows.remove(row)
    if only_first or only_second:
        different = True
        print(f"DIFFERENT {table}: {len(first[table])} rows on the first server, {len(second[table])} on this one")
        for row in only_first:
            print("  first server only:", row)
        for row in only_second:
            print("  this server only: ", row)
    else:
        print(f"same      {table} ({len(first[table])} rows, {len(first[table]) - len(second[table])} fewer on this server)" if len(first[table]) != len(second[table]) else f"same      {table} ({len(first[table])} rows)")
sys.exit(1 if different else 0)
