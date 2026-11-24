"""Gives the accounts that `acctsec-oracle.mjs prepare` signed in the second
steps, passwords and sessions the scenarios need, by editing a database file
while its server is stopped. Both servers' files are edited the same way.

  acctsec-edit.py <database> <smtp host> <smtp port>      the arrangement
  acctsec-edit.py <database> --mail-port <port>            the mail server moves
"""
import sqlite3
import sys
import uuid
from datetime import datetime, timedelta, timezone

NOW = datetime.now(timezone.utc)


def text(moment):
    return moment.strftime("%Y-%m-%d %H:%M:%S.%f")


def new_id():
    return str(uuid.uuid4()).upper()


def main():
    path = sys.argv[1]
    db = sqlite3.connect(path)
    if sys.argv[2] == "--mail-port":
        db.execute('UPDATE "OperationsSettings" SET "MailSmtpPort" = ?', (int(sys.argv[3]),))
        db.commit()
        return
    host, port = sys.argv[2], int(sys.argv[3])
    future, past = text(NOW + timedelta(hours=1)), text(NOW - timedelta(hours=1))

    def user(name):
        return db.execute('SELECT "Id" FROM "Users" WHERE "Username" = ?', (name,)).fetchone()[0]

    def sessions(name):
        # The first is the one registering made, which no browser of the
        # scenarios holds.
        return [row[0] for row in db.execute(
            'SELECT "Id" FROM "UserSessions" WHERE "UserId" = ? ORDER BY "CreatedAt", rowid', (user(name),))][1:]

    def method(name, kind, label, address=None, age=0):
        db.execute(
            'INSERT INTO "MfaMethods" ("Id", "UserId", "Type", "Name", "CreatedAt", "ConfirmedAt", "EmailAddress") VALUES (?,?,?,?,?,?,?)',
            (new_id(), user(name), kind, label, text(NOW - timedelta(days=10 - age)), text(NOW - timedelta(days=10 - age)), address))

    def codes(name, count):
        batch = new_id()
        for _ in range(count):
            db.execute('INSERT INTO "RecoveryCodes" ("Id", "UserId", "BatchId", "CodeHash", "CreatedAt") VALUES (?,?,?,?,?)',
                       (new_id(), user(name), batch, uuid.uuid4().hex.upper() * 2, text(NOW)))

    def device(name, label):
        db.execute('INSERT INTO "TrustedDevices" ("Id", "UserId", "TokenHash", "Name", "Device", "IpAddress", "CreatedAt", "ExpiresAt") VALUES (?,?,?,?,?,?,?,?)',
                   (new_id(), user(name), uuid.uuid4().hex.upper() * 2, label, "Firefox on Linux", "203.0.113.5",
                    text(NOW - timedelta(days=1)), text(NOW + timedelta(days=13))))

    def mark(name, tab, fresh=None, primary="fresh"):
        """The session of a tab: it passed a second step lately (`fresh`), or
        only signed in lately (`primary` fresh) or long ago."""
        sid = sessions(name)[tab]
        db.execute('UPDATE "UserSessions" SET "AuthenticatedAt" = ?, "MfaCompletedAt" = ?, "MfaEvidence" = ? WHERE "Id" = ?',
                   (future if primary == "fresh" else past,
                    future if fresh else None, "totp" if fresh else None, sid))

    def no_password(name):
        db.execute('UPDATE "Users" SET "PasswordHash" = NULL WHERE "Id" = ?', (user(name),))

    def identity(name, issuer="https://idp.example.test/realms/main"):
        db.execute('INSERT INTO "ExternalIdentities" ("Id", "UserId", "Issuer", "Subject", "ProviderId", "CreatedAt") VALUES (?,?,?,?,NULL,?)',
                   (new_id(), user(name), issuer, uuid.uuid4().hex, text(NOW - timedelta(days=2))))

    server = db.execute('SELECT "Id" FROM "Servers" ORDER BY "CreatedAt", "Id" LIMIT 1').fetchone()[0]
    db.execute(
        'INSERT INTO "OperationsSettings" ("ServerId","PublicUrl","TimeZone","MailingEnabled","MailFromAddress","InviteMailEnabled","RecoveryMailEnabled","AlertMailEnabled",'
        '"BackupRetentionDays","LogRetentionDays","OrphanObjectRetentionDays","AutomaticCleanupEnabled","UpdatedAt","MailSmtpHost","MailSmtpPort","MailSmtpSecurity","MailResendEnabled","MfaRequirement") '
        "VALUES (?,'','UTC',1,'noreply@example.test',0,0,1,7,7,7,0,?,?,?,'none',0,'admins')",
        (server, text(NOW), host, port))

    # Everybody who is not special: signed in just now, in every browser.
    for (name,) in db.execute('SELECT "Username" FROM "Users"').fetchall():
        for tab in range(len(sessions(name))):
            mark(name, tab, primary="fresh")

    mark("old", 0, primary="old")
    mark("old", 1, primary="old")

    method("armed", "totp", "Phone")
    method("armed", "email", "Mail", "ARMED@EXAMPLE.TEST", 1)
    codes("armed", 3)
    device("armed", "Kitchen laptop")
    device("armed", "Work desktop")
    mark("armed", 0, fresh=True)
    mark("armed", 1, fresh=False)
    for n in range(25):
        db.execute('INSERT INTO "SecurityEvents" ("Id","UserId","Kind","Detail","IpAddress","Device","CreatedAt") VALUES (?,?,?,?,?,?,?)',
                   (new_id(), user("armed"), "login-succeeded", f"password {n}", "203.0.113.7", "Firefox", text(NOW - timedelta(minutes=30 - n))))

    method("armed2", "totp", "Phone")
    method("armed2", "email", "Mail", "ARMED2@EXAMPLE.TEST", 1)
    codes("armed2", 4)
    device("armed2", "Laptop")
    mark("armed2", 0, fresh=True)

    method("sole", "totp", "Phone")
    codes("sole", 4)
    device("sole", "Laptop")
    mark("sole", 0, fresh=True)

    method("owner", "totp", "Phone")
    mark("owner", 0, fresh=True)

    no_password("ppk")
    method("ppk", "passkey", "Laptop key")
    identity("ppk")
    mark("ppk", 0, fresh=True)

    no_password("ppk2")
    method("ppk2", "passkey", "Laptop key")
    method("ppk2", "totp", "Phone", None, 1)
    mark("ppk2", 0, fresh=True)

    method("pwpk", "passkey", "Laptop key")
    mark("pwpk", 0, fresh=True)

    no_password("pk2")
    method("pk2", "passkey", "Laptop key")
    method("pk2", "passkey", "Phone key", None, 1)
    mark("pk2", 0, fresh=True)

    no_password("nopass")
    identity("nopass")
    no_password("nopassold")
    mark("nopassold", 0, primary="old")
    no_password("nopassm")
    method("nopassm", "totp", "Phone")
    mark("nopassm", 0, fresh=False)

    method("mailer", "totp", "Phone")
    method("mailer", "email", "Mail", "MAILER@EXAMPLE.TEST", 1)
    mark("mailer", 0, fresh=True)
    mark("mailer", 1, fresh=True)
    db.commit()


main()
