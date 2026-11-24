"""The rows the scenarios leave, in two databases that started as one file.

Ids are replaced by what they belong to, moments and secrets by markers, and
the rows of each table are sorted and compared. Prints what differs.
"""
import re
import sqlite3
import sys

UUID = re.compile(r"[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}")
TIME = re.compile(r"\d{4}-\d\d-\d\d[ T]\d\d:\d\d:\d\d(\.\d+)?Z?")
HASH = re.compile(r"\b[0-9A-F]{64}\b")

QUERIES = {
    "Users": 'SELECT u."Username", u."PasswordHash" IS NOT NULL, u."Email", u."NormalizedEmail", u."ProviderVerifiedEmail", u."SessionVersion" FROM "Users" u',
    "UserSessions": 'SELECT u."Username", s."RevokedAt" IS NOT NULL, s."SecurityVersion", s."ShortLived", s."MfaEvidence", s."MfaCompletedAt" IS NOT NULL FROM "UserSessions" s JOIN "Users" u ON u."Id" = s."UserId"',
    "MfaMethods": 'SELECT u."Username", m."Type", m."Name", m."ConfirmedAt" IS NOT NULL, m."RevokedAt" IS NOT NULL, m."EmailAddress" FROM "MfaMethods" m JOIN "Users" u ON u."Id" = m."UserId"',
    "RecoveryCodes": 'SELECT u."Username", c."UsedAt" IS NOT NULL, COUNT(*) FROM "RecoveryCodes" c JOIN "Users" u ON u."Id" = c."UserId" GROUP BY 1, 2',
    "TrustedDevices": 'SELECT u."Username", d."Name", d."RevokedAt" IS NOT NULL FROM "TrustedDevices" d JOIN "Users" u ON u."Id" = d."UserId"',
    "SecurityEvents": 'SELECT u."Username", e."Kind", e."Detail", e."Device" FROM "SecurityEvents" e JOIN "Users" u ON u."Id" = e."UserId" WHERE e."Kind" NOT IN (\'login-succeeded\', \'login-failed\')',
    "SecurityNotifications": 'SELECT u."Username", e."Kind", n."EmailAddress" FROM "SecurityNotifications" n JOIN "SecurityEvents" e ON e."Id" = n."SecurityEventId" JOIN "Users" u ON u."Id" = n."UserId" WHERE e."Kind" NOT IN (\'login-succeeded\', \'login-failed\')',
    "EmailChangeRequests": 'SELECT u."Username", r."NewEmail", r."NormalizedNewEmail", r."SessionId" IS NOT NULL, r."SecurityVersion", r."UsedAt" IS NOT NULL, r."TokenHash" IS NOT NULL, LENGTH(r."TokenHash"), r."ExpiresAt" > r."CreatedAt" FROM "EmailChangeRequests" r JOIN "Users" u ON u."Id" = r."UserId"',
    "EmailOtpCodes": 'SELECT u."Username", c."Purpose", c."ConsumedAt" IS NOT NULL FROM "EmailOtpCodes" c JOIN "Users" u ON u."Id" = c."UserId"',
    "DesktopBrowserLogins": 'SELECT u."Username", d."RedeemedAt" IS NOT NULL FROM "DesktopBrowserLogins" d JOIN "Users" u ON u."Id" = d."UserId"',
}


def scrub(value):
    if isinstance(value, str):
        value = HASH.sub("<hash>", UUID.sub("<id>", value))
        return TIME.sub("<time>", value)
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
    only_first = [r for r in first[table] if r not in second[table]]
    only_second = [r for r in second[table] if r not in first[table]]
    if only_first or only_second:
        different = True
        print(f"DIFFERENT {table}")
        for row in only_first:
            print("  first server only:", row)
        for row in only_second:
            print("  this server only: ", row)
    else:
        print(f"same      {table} ({len(first[table])} rows)")
sys.exit(1 if different else 0)
