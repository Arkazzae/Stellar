"""What the desktop sign-in left in two databases that started as one file.

Prints the requests and the security events of each in terms that do not
differ by nature (rows in the order they were made, accounts by name, sessions
by their order within the account, moments and hashes as present or not), and
the differences between the two. Exits with 1 when there are any.
"""
import re
import sqlite3
import sys

first, second = sys.argv[1:3]
TIME = re.compile(r"\d{4}-\d\d-\d\d[ T]\d\d:\d\d:\d\d(\.\d+)?Z?")


def dump(path):
    db = sqlite3.connect(path)
    users = dict(db.execute('SELECT "Id", "Username" FROM "Users"'))
    sessions = {}
    seen = {}
    for sid, uid in db.execute('SELECT "Id", "UserId" FROM "UserSessions" ORDER BY "CreatedAt", "Id"'):
        seen[uid] = seen.get(uid, 0) + 1
        sessions[sid] = f"{users.get(uid, '?')}#{seen[uid]}"
    present = lambda value: None if value is None else "<set>"
    requests = []
    for row in db.execute(
        'SELECT "Id", "SecretHash", "ApprovalTokenHash", "ApprovalUserId", "ApprovalSessionId", "UserId", "ApprovedSessionId", '
        '"PrimarySource", "MfaCompletedAt", "MfaEvidence", "SecurityVersion", "CreatedAt", "ExpiresAt", "ApprovedAt", "RedeemedAt" '
        'FROM "DesktopBrowserLogins" ORDER BY "CreatedAt", "Id"'
    ):
        (_, secret, token, a_user, a_session, user, s_session, source, mfa_at, mfa_ev, version, created, expires, approved, redeemed) = row
        requests.append(
            {
                "SecretHash": f"<{len(secret)} chars, upper hex: {secret == secret.upper()}>",
                "ApprovalTokenHash": present(token),
                "ApprovalUser": users.get(a_user),
                "ApprovalSession": sessions.get(a_session),
                "User": users.get(user),
                "ApprovedSession": sessions.get(s_session),
                "PrimarySource": source,
                "MfaCompletedAt": present(mfa_at),
                "MfaEvidence": mfa_ev,
                "SecurityVersion": version,
                "Lifetime": "<2 minutes>" if _minutes(created, expires) == 2 else f"<{_minutes(created, expires)} minutes>",
                "Approved": present(approved),
                "Redeemed": present(redeemed),
            }
        )
    events = sorted(
        (
            users.get(uid, "?"),
            kind,
            detail,
            device,
            "<ip>" if ip else "",
        )
        for uid, kind, detail, device, ip in db.execute(
            'SELECT "UserId", "Kind", "Detail", "Device", "IpAddress" FROM "SecurityEvents" WHERE "Kind" = \'desktop-approved\''
        )
    )
    # What the exchange made: the sessions it gave, and the sign-in it recorded.
    made = []
    for row in db.execute(
        'SELECT "UserId", "PrimarySource", "AuthenticatedAt", "MfaCompletedAt", "MfaEvidence", "SecurityVersion", "ShortLived", "RevokedAt", "CreatedAt", "ExpiresAt", "AbsoluteExpiresAt", "Device" '
        'FROM "UserSessions" ORDER BY "CreatedAt", "Id"'
    ):
        uid, source, authed, mfa_at, mfa_ev, version, short, revoked, created, expires, absolute, device = row
        if not device.startswith("node"):
            continue
        made.append((users.get(uid), source, present(authed), present(mfa_at), mfa_ev, version, short, present(revoked), f"{_minutes(created, expires) // 1440} days", expires == absolute))
    logins = sorted(
        (users.get(uid, "?"), detail, device)
        for uid, detail, device in db.execute(
            'SELECT "UserId", "Detail", "Device" FROM "SecurityEvents" WHERE "Kind" = \'login-succeeded\' AND "Detail" = \'desktop\''
        )
    )
    return requests, events, made, logins


def _minutes(created, expires):
    def seconds(text):
        match = re.match(r"(\d+)-(\d+)-(\d+)[ T](\d+):(\d+):(\d+)", text)
        y, mo, d, h, mi, s = map(int, match.groups())
        return ((((y * 12 + mo) * 31 + d) * 24 + h) * 60 + mi) * 60 + s

    return round((seconds(expires) - seconds(created)) / 60)


(a_requests, a_events, a_made, a_logins), (b_requests, b_events, b_made, b_logins) = dump(first), dump(second)
print(f"requests: {len(a_requests)} and {len(b_requests)}; approval events: {len(a_events)} and {len(b_events)}")
differs = False
for index in range(max(len(a_requests), len(b_requests))):
    left = a_requests[index] if index < len(a_requests) else None
    right = b_requests[index] if index < len(b_requests) else None
    if left != right:
        differs = True
        print(f"request {index + 1} differs:")
        for key in (left or right):
            if (left or {}).get(key) != (right or {}).get(key):
                print(f"  {key}: first {(left or {}).get(key)!r}, this {(right or {}).get(key)!r}")
if a_events != b_events:
    differs = True
    print("approval events differ:")
    print(f"  first: {a_events}")
    print(f"  this:  {b_events}")
if sorted(a_made, key=repr) != sorted(b_made, key=repr) or a_logins != b_logins:
    differs = True
    print("sessions or sign-in events differ:")
    print(f"  first: {sorted(a_made, key=repr)} {a_logins}")
    print(f"  this:  {sorted(b_made, key=repr)} {b_logins}")
print(f"sessions of node: {len(a_made)} and {len(b_made)}; desktop sign-ins: {len(a_logins)} and {len(b_logins)}")
if not differs:
    print("the rows are the same")
sys.exit(1 if differs else 0)
