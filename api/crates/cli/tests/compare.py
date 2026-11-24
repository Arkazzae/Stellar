#!/usr/bin/env python3
"""The first server's tool and this one, on copies of one data folder.

For every command and option, in plain and `--json` form, and for every wrong
use: the standard output, the standard error and the exit status of both, and
the rows each leaves in the database. What differs by nature (the data root in a
message, the version, the moment a command ran, a credential nobody can repeat)
is replaced before the two are compared; what is left, and is not listed in
KNOWN with its reason, is a difference.

    DN_DLL=.../Stellarr.Cli.dll STELLARR_BIN=.../stellarr BASE=<data root> \\
        python3 compare.py [filter]

BASE is a data root with `db/stellarr.db` (the first server's schema, with the
rows of fixture.sql) and a `keys` folder. `compare.sh` makes one.
"""
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile

DOTNET = os.environ.get("DOTNET", os.path.expanduser("~/.dotnet/dotnet"))
DN_DLL = os.environ["DN_DLL"]
RS_BIN = os.environ["STELLARR_BIN"]
BASE = os.environ["BASE"]
FILTER = sys.argv[1] if len(sys.argv) > 1 else ""

OWNER = "owner"
MAYA = "maya"
BASE_ENV = {
    "PATH": os.environ["PATH"],
    "HOME": os.environ.get("HOME", "/tmp"),
    "STELLARR_API_URL": "http://127.0.0.1:1",
    "DOTNET_CLI_TELEMETRY_OPTOUT": "1",
    "DOTNET_NOLOGO": "1",
    "TZ": "UTC",
}

results = {"same": 0, "known": 0, "different": 0}
differences = []


def tools():
    return {"first": [DOTNET, DN_DLL], "this": [RS_BIN]}


def fresh(tool, source=BASE):
    root = tempfile.mkdtemp(prefix=f"cli-{tool}-")
    shutil.copytree(source, root, dirs_exist_ok=True)
    return root


def run(tool, root, args, stdin="", env=None, tty_answers=None):
    environment = dict(BASE_ENV)
    environment.update(env or {})
    command = tools()[tool] + ["--data-root", root] + args
    if tty_answers is not None:
        return run_tty(command, environment, tty_answers)
    done = subprocess.run(command, input=stdin.encode(), capture_output=True, env=environment, timeout=120, cwd=root)
    return done.returncode, done.stdout.decode(errors="replace"), done.stderr.decode(errors="replace")


def run_tty(command, environment, answers):
    """With a terminal as input, so that the tool asks."""
    import pty
    import select

    master, slave = pty.openpty()
    process = subprocess.Popen(command, stdin=slave, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=environment)
    os.close(slave)
    for answer in answers:
        select.select([], [master], [], 5)
        os.write(master, answer.encode())
    out, err = process.communicate(timeout=60)
    os.close(master)
    return process.returncode, out.decode(errors="replace"), err.decode(errors="replace")


def dump(root):
    """The rows a command may have left, with what differs by nature taken out."""
    path = os.path.join(root, "db", "stellarr.db")
    if not os.path.exists(path):
        return ["(no database)"]
    connection = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    rows = []

    def table(name, columns, where=""):
        try:
            for row in connection.execute(f'SELECT {columns} FROM "{name}" {where} ORDER BY 1,2,3'):
                rows.append(f"{name}: {row!r}")
        except sqlite3.Error as error:
            rows.append(f"{name}: {error}")

    table("OperatorAuditEntries", "Actor, Operation, Target")
    table("OperationsSettings", "ServerId, PublicUrl, TimeZone, MailingEnabled, MailFromAddress, MailSmtpHost, MailSmtpPort, MailSmtpUsername, MailSmtpSecurity, MailResendEnabled, GiphyEnabled, InviteMailEnabled, RecoveryMailEnabled, AlertMailEnabled, BackupRetentionDays, LogRetentionDays, OrphanObjectRetentionDays, AutomaticCleanupEnabled, DesktopMinimumVersion, DesktopRecommendedVersion, MfaRequirement, TrustedDeviceDays, AuthChallengeMinutes, EmailCodeMinutes, EmailCodeCooldownSeconds, EmailCodeMaxAttempts, EmailCodeSendsPerHour, CliRecoveryMinutes, UpdatedAt IS NOT NULL")
    table("Servers", "Id, Name")
    table("InstanceSecrets", "Name, length(ProtectedValue) > 0")
    table("UserSessions", "Id, UserId, RevokedAt IS NOT NULL, SecurityVersion")
    table("DesktopBrowserLogins", "Id, UserId, RedeemedAt IS NOT NULL")
    table("MfaMethods", "Id, UserId, Type, RevokedAt IS NOT NULL")
    table("TrustedDevices", "Id, UserId, RevokedAt IS NOT NULL")
    table("RecoveryCodes", "Id, UserId, UsedAt IS NOT NULL")
    table("CliRecoveryCredentials", "UserId, Reason, SecurityVersion, UsedAt IS NOT NULL, round((julianday(ExpiresAt) - julianday(CreatedAt)) * 1440)")
    table("SecurityEvents", "UserId, Kind, Detail, IpAddress, Device")
    table("SecurityNotifications", "UserId, EmailAddress, Attempts")
    table("Users", "Id, SessionVersion, SecurityRecoveryRequired, IsDisabled")
    table("ExternalIdentities", "Id, UserId, Issuer")
    connection.close()
    return rows


MOMENT = re.compile(r"\d{4}-\d\d-\d\d[T ]\d\d:\d\d:\d\d(\.\d+)?Z?")


def normalise(text, root, volatile):
    text = text.replace(root, "<ROOT>")
    text = re.sub(r"stellarr \S+ - operator", "stellarr <V> - operator", text)
    text = re.sub(r'("version": ")[^"]+"', r'\1<V>"', text)
    text = re.sub(r"^\d+\.\d+\.\d+\S*$", "<V>", text, flags=re.M)
    if volatile:
        text = MOMENT.sub("<MOMENT>", text)
        text = re.sub(r"[A-Za-z0-9_-]{43}", "<CREDENTIAL>", text)
    return text


# What differs on purpose, by the label of the case, and why.
KNOWN = []


def known(label, first, this):
    for pattern, reason in KNOWN:
        if re.search(pattern, label):
            return reason
    return None


def case(label, args, setup=None, stdin="", env=None, mutating=False, volatile=False, tty=None, source=BASE):
    if FILTER and FILTER not in label:
        return
    answers = {}
    roots = {}
    for tool in tools():
        root = fresh(tool, source)
        roots[tool] = root
        if setup:
            setup(root)
        code, out, err = run(tool, root, args, stdin, env, tty)
        answers[tool] = (code, normalise(out, root, volatile), normalise(err, root, volatile))
    state = {tool: dump(roots[tool]) for tool in tools()} if mutating else None
    for root in roots.values():
        shutil.rmtree(root, ignore_errors=True)
    parts = []
    for index, name in enumerate(["exit status", "stdout", "stderr"]):
        if answers["first"][index] != answers["this"][index]:
            parts.append((name, answers["first"][index], answers["this"][index]))
    if state and state["first"] != state["this"]:
        parts.append(("rows", "\n".join(state["first"]), "\n".join(state["this"])))
    if not parts:
        results["same"] += 1
        print(f"same       {label}")
        return
    reason = known(label, answers["first"], answers["this"])
    if reason:
        results["known"] += 1
        print(f"known      {label} ({reason})")
        return
    results["different"] += 1
    print(f"DIFFERENT  {label}")
    for name, first, this in parts:
        differences.append(f"--- {label}: {name}\nfirst:\n{first}\nthis:\n{this}\n")


def sql(statement):
    def apply(root):
        connection = sqlite3.connect(os.path.join(root, "db", "stellarr.db"))
        connection.executescript(statement)
        connection.commit()
        connection.close()

    return apply


def both(*steps):
    def apply(root):
        for step in steps:
            step(root)

    return apply


def one_of_each(prefix, args, **options):
    case(f"{prefix}", args, **options)
    case(f"{prefix} --json", ["--json"] + args, **options)


# --------------------------------------------------------------------------------------
# What is read.
# --------------------------------------------------------------------------------------
for word in ["", "help", "--help", "-h", "help status", "help doctor", "help config", "help secrets", "help users",
             "help sessions", "help security", "help backup", "help nonsense", "help version", "status --help",
             "config --help", "backup -h"]:
    one_of_each(f"help: {word!r}", word.split() if word else [])
one_of_each("version", ["version"])
one_of_each("--version", ["--version"])
one_of_each("version with an extra word", ["version", "x"])
case("a write command never runs after --help", ["config", "set", "time-zone", "Europe/Warsaw", "--yes", "--help"], mutating=True)
case("a write command never runs after --version", ["config", "set", "time-zone", "Europe/Warsaw", "--yes", "--version"], mutating=True)

for env_name, env in [("production", {}), ("testing", {"ASPNETCORE_ENVIRONMENT": "Testing"}), ("development", {"ASPNETCORE_ENVIRONMENT": "Development"})]:
    one_of_each(f"status [{env_name}]", ["status"], env=env)
    one_of_each(f"doctor [{env_name}]", ["doctor"], env=env)
    one_of_each(f"config list [{env_name}]", ["config", "list"], env=env)
    one_of_each(f"config validate [{env_name}]", ["config", "validate"], env=env)

one_of_each("secrets list", ["secrets", "list"])
one_of_each("users list", ["users", "list"])
for who in [OWNER, MAYA, "dora", "imported-1", "6da0d88b-3d29-47a6-b01b-ebfe943238c2", "OWNER", "  maya  ", "nobody", "", "{ff591e72-0514-41e3-9e71-4668a3978948}", "ff591e725141e3-9e71"]:
    one_of_each(f"users show {who!r}", ["users", "show", who])
one_of_each("sessions list", ["sessions", "list"])
for who in [OWNER, MAYA, "nobody", "--user="]:
    one_of_each(f"sessions list --user {who!r}", ["sessions", "list", "--user", who] if who != "--user=" else ["sessions", "list", "--user="])
for who in [OWNER, MAYA, "dora", "imported-1", "nobody", "NOBODY"]:
    one_of_each(f"security show {who!r}", ["security", "show", who])
    one_of_each(f"security methods {who!r}", ["security", "methods", who])

# Every setting, read.
KEYS = ["instance.name", "time-zone", "mail.enabled", "mail.from-address", "mail.smtp.host", "mail.smtp.port",
        "mail.smtp.username", "mail.smtp.security", "mail.resend.enabled", "mail.invites.enabled",
        "mail.recovery.enabled", "mail.alerts.enabled", "giphy.enabled", "retention.backups-days",
        "retention.logs-days", "retention.orphans-days", "cleanup.enabled", "mfa.requirement",
        "mfa.trusted-device-days", "mfa.challenge-minutes", "mfa.email-code-minutes",
        "mfa.email-cooldown-seconds", "mfa.email-attempts", "mfa.email-sends-per-hour",
        "mfa.cli-recovery-minutes", "public-url", "tls.domain", "https-port", "oidc.enabled", "oidc.authority",
        "oidc.client-id", "oidc.trust-provider-mfa", "oidc.provider-mfa-amr", "oidc.providers",
        "oidc.callback-path", "rtc.public-host", "rtc.turn-port", "rtc.turns-port", "rtc.turns-enabled",
        "rtc.turn-tcp-enabled", "rtc.media-port"]
for key in KEYS:
    one_of_each(f"config get {key}", ["config", "get", key])
one_of_each("config get unknown", ["config", "get", "nope"])
one_of_each("config get a secret's name", ["config", "get", "mail.smtp-password"])

ENVIRONMENTS = {
    "a domain and a published port": {"STELLARR_DOMAIN": "chat.example.com", "STELLARR_HTTPS_PORT": "8443", "Operations__PublicUrl": "https://chat.example.com:8443"},
    "a domain alone": {"STELLARR_DOMAIN": "chat.example.com"},
    "blank variables": {"STELLARR_HTTPS_PORT": "  ", "STELLARR_DOMAIN": ""},
    "turn and media": {"STELLARR_TURN_TCP_ENABLED": "true", "STELLARR_MEDIA_PORT": "49010", "Rtc__TurnsPort": "5350"},
    "oidc": {"STELLARR_OIDC_ENABLED": "true", "STELLARR_OIDC_ISSUER": "https://id.example.test", "STELLARR_OIDC_CLIENT_ID": "abc",
             "Oidc__Providers__work__Authority": "https://work.example.test", "Oidc__Providers__work__TrustProviderMfa": "true",
             "Oidc__Providers__lab__Authority": "https://lab.example.test", "Oidc__CallbackPath": "/cb"},
    "mail from the environment": {"STELLARR_MAIL_ENABLED": "false", "STELLARR_MAIL_FROM": "noreply@example.test", "STELLARR_TIME_ZONE": "Asia/Tokyo"},
    "keys in the environment": {"STELLARR_GIPHY_API_KEY": "env-giphy", "Mail__ResendApiKey": "re_env", "STELLARR_SMTP_PASSWORD": " spaced "},
}
for name, env in ENVIRONMENTS.items():
    one_of_each(f"config list with {name}", ["config", "list"], env=env)
    one_of_each(f"config validate with {name}", ["config", "validate"], env=env)
    one_of_each(f"secrets list with {name}", ["secrets", "list"], env=env)
    one_of_each(f"status with {name}", ["status"], env=env)

# --------------------------------------------------------------------------------------
# Wrong use.
# --------------------------------------------------------------------------------------
WRONG = [
    ["nonsense"], ["Status"], ["STATUS"], ["status", "extra"], ["doctor", "extra"], ["status", "--yes"], ["status", "--stdin"],
    ["status", "--all"], ["status", "--user", "x"], ["status", "--reason", "x"], ["status", "--nope"], ["status", "--nope=1"],
    ["status", "--json=1"], ["status", "--data-root"], ["config"], ["config", "nope"], ["config", "list", "x"], ["config", "list", "x", "y"],
    ["config", "get"], ["config", "get", "a", "b"], ["config", "set"], ["config", "set", "k"], ["config", "set", "k", "v", "x"],
    ["config", "set", "time-zone", "UTC", "--stdin"], ["config", "set", "time-zone", "UTC", "--all"], ["config", "set", "time-zone", "UTC", "--user", "x"],
    ["config", "set", "time-zone", "UTC", "--reason", "x"], ["config", "validate", "x"], ["secrets"], ["secrets", "nope"], ["secrets", "list", "x"],
    ["secrets", "set"], ["secrets", "set", "mail.smtp-password", "value"], ["secrets", "set", "a", "b", "c"], ["secrets", "set", "nope", "--stdin", "--yes"],
    ["secrets", "set", "mail.smtp-password", "--all"], ["secrets", "clear"], ["secrets", "clear", "nope"], ["secrets", "clear", "mail.smtp-password", "x"],
    ["users"], ["users", "nope"], ["users", "list", "x"], ["users", "show"], ["users", "show", "a", "b"], ["users", "show", "x", "--yes"],
    ["sessions"], ["sessions", "nope"], ["sessions", "list", "x"], ["sessions", "list", "--yes"], ["sessions", "list", "--all"],
    ["sessions", "revoke"], ["sessions", "revoke", "x"], ["sessions", "revoke", "x", "y"], ["sessions", "revoke", "--all"],
    ["sessions", "revoke", "--all", "--user", "x", "extra"], ["sessions", "revoke", "x", "--user", "y"], ["sessions", "revoke", "x", "--stdin"],
    ["security"], ["security", "nope"], ["security", "show"], ["security", "show", "a", "b"], ["security", "recover"], ["security", "recover", "x"],
    ["security", "recover", "x", "--reason"], ["security", "recover", "x", "--reason", "ab", "--yes"], ["security", "recover", "x", "y", "--reason", "abc"],
    ["security", "unlink"], ["security", "unlink", "x"], ["security", "unlink", "x", "y", "z"], ["security", "unlink", "x", "not-a-guid", "--yes"],
    ["backup"], ["backup", "nope"], ["backup", "verify"], ["backup", "verify", "a", "b"], ["backup", "verify", "nothing.zip"], ["backup", "restore"],
    ["backup", "cancel", "x"], ["backup", "cancel"], ["backup", "apply", "x"], ["backup", "apply", "--yes"],
    ["--data-root"], ["--data-root", "--json"], ["--data-root", "", "version"], ["--data-root", "   ", "version"], ["--data-root=", "version"],
    ["--data-root=--virtual", "version"], ["--user"], ["--reason", ""], ["--"], ["--", "status"], ["-"], ["-x"], ["--json", "--nope"],
]
for args in WRONG:
    one_of_each(f"wrong: {' '.join(args)!r}", args)

# A data folder that is not there, or not a database, or locked.
def empty_root(root):
    shutil.rmtree(os.path.join(root, "db"))


def garbage_database(root):
    with open(os.path.join(root, "db", "stellarr.db"), "wb") as handle:
        handle.write(b"this is not a database at all" * 200)


def empty_database(root):
    os.remove(os.path.join(root, "db", "stellarr.db"))
    sqlite3.connect(os.path.join(root, "db", "stellarr.db")).execute("CREATE TABLE T (x)").connection.commit()


def drop_audit(root):
    sqlite3.connect(os.path.join(root, "db", "stellarr.db")).executescript('DROP TABLE "OperatorAuditEntries";')


def unreadable(root):
    os.chmod(os.path.join(root, "db", "stellarr.db"), 0)


for name, setup in [("no database", empty_root), ("a file that is not a database", garbage_database), ("a database without the schema", empty_database),
                    ("no audit table", drop_audit)]:
    for command in (["status"], ["doctor"], ["config", "list"], ["config", "get", "time-zone"], ["config", "get", "public-url"], ["users", "list"],
                    ["secrets", "list"], ["sessions", "list"], ["security", "show", MAYA],
                    ["config", "set", "time-zone", "Europe/Warsaw", "--yes"], ["secrets", "clear", "giphy.api-key", "--yes"],
                    ["sessions", "revoke", "--user", MAYA, "--all", "--yes"]):
        one_of_each(f"{name}: {' '.join(command)}", command, setup=setup, mutating=name == "no audit table")

# The database locked by another process: a writer holds it for as long as the commands run.
import threading


class Holder:
    """Holds the write lock of a copy until released."""

    def __init__(self):
        self.connection = None


def locked_setup(root):
    connection = sqlite3.connect(os.path.join(root, "db", "stellarr.db"), isolation_level=None, check_same_thread=False)
    connection.execute("PRAGMA journal_mode=WAL")
    connection.execute("BEGIN EXCLUSIVE")
    HOLD.append(connection)


HOLD = []
for command in (["config", "set", "time-zone", "Europe/Warsaw", "--yes"], ["sessions", "revoke", "--user", MAYA, "--all", "--yes"], ["secrets", "clear", "giphy.api-key", "--yes"]):
    for flavour in ["write-locked"]:
        label = f"{flavour}: {' '.join(command)}"
        if FILTER and FILTER not in label:
            continue
        answers = {}
        for tool in tools():
            root = fresh(tool)
            locked_setup(root)
            answers[tool] = run(tool, root, command)
            for connection in HOLD:
                connection.execute("ROLLBACK")
                connection.close()
            HOLD.clear()
            shutil.rmtree(root, ignore_errors=True)
        same = all(answers["first"][i] == answers["this"][i] for i in range(3))
        results["same" if same else "different"] += 1
        print(("same       " if same else "DIFFERENT  ") + label)
        if not same:
            differences.append(f"--- {label}\nfirst: {answers['first']}\nthis: {answers['this']}\n")

# --------------------------------------------------------------------------------------
# What is changed.
# --------------------------------------------------------------------------------------
VALUES = {
    "instance.name": ["Orbit", "  Orbit  ", "", "x" * 80, "x" * 81, "Zażółć <&> 'q' + `t`"],
    "time-zone": ["Europe/Warsaw", "europe/warsaw", "UTC", "utc", "Poland", "Asia/Kolkata", "Nowhere/Land", "../etc/passwd", "", "Local", " Europe/London "],
    "mail.enabled": ["true", "false", "yes", "NO", "on", "Off", "1", "0", "maybe", "", " true "],
    "mail.from-address": ["a@example.test", "", "Name <a@example.test>", '"Q, R" <a@example.test>', "nope", "a@", "@b.test", "<a@example.test>", "a@example.test, b@example.test", "x" * 321, "a b@c.test", "Name <a@example.test"],
    "mail.smtp.host": ["smtp.example.test", "", "h" * 255, "h" * 256],
    "mail.smtp.port": ["2525", "1", "65535", "0", "65536", "99999", "-1", "abc", "+25", " 25 ", "1e3", "2147483648", "25.5", ""],
    "mail.smtp.username": ["user", "", "u" * 320, "u" * 321],
    "mail.smtp.security": ["tls", "TLS", "none", "starttls", "StartTLS", "ssl", ""],
    "mail.resend.enabled": ["true", "false"],
    "mail.invites.enabled": ["false"], "mail.recovery.enabled": ["false"], "mail.alerts.enabled": ["true"],
    "giphy.enabled": ["true", "false"], "cleanup.enabled": ["false"],
    "retention.backups-days": ["1", "3650", "0", "3651"], "retention.logs-days": ["7", "0"], "retention.orphans-days": ["3", "3651"],
    "mfa.requirement": ["everyone", "Admins", "cockpit", "optional", "sometimes", ""],
    "mfa.trusted-device-days": ["365", "366", "0"], "mfa.challenge-minutes": ["60", "61", "0"], "mfa.email-code-minutes": ["1", "61"],
    "mfa.email-cooldown-seconds": ["0", "600", "601", "-1"], "mfa.email-attempts": ["10", "11", "0"],
    "mfa.email-sends-per-hour": ["60", "61"], "mfa.cli-recovery-minutes": ["60", "61", "5"],
}
for key, values in VALUES.items():
    for value in values:
        one_of_each(f"config set {key} {value[:24]!r}", ["config", "set", key, value, "--yes"], mutating=True)
for key in ["public-url", "tls.domain", "https-port", "oidc.enabled", "oidc.providers", "rtc.media-port"]:
    one_of_each(f"config set a startup setting {key}", ["config", "set", key, "x", "--yes"], mutating=True)
for key in ["mail.smtp-password", "mail.resend-api-key", "giphy.api-key", "cloudflare.r2.access-key"]:
    one_of_each(f"config set a secret {key}", ["config", "set", key, "x", "--yes"], mutating=True)
one_of_each("config set unknown", ["config", "set", "nope", "x", "--yes"], mutating=True)
one_of_each("config set without --yes, no terminal", ["config", "set", "time-zone", "Europe/Warsaw"], mutating=True)
one_of_each("config set with mail on and no sender", ["config", "set", "mail.enabled", "true", "--yes"], mutating=True,
            env={"ASPNETCORE_ENVIRONMENT": "Testing"})
one_of_each("config set in testing", ["config", "set", "time-zone", "Europe/Warsaw", "--yes"], mutating=True, env={"ASPNETCORE_ENVIRONMENT": "Testing"})
one_of_each("config set from a given environment", ["config", "set", "mail.smtp.port", "2525", "--yes"], mutating=True, env=ENVIRONMENTS["mail from the environment"])
case("config set twice: the second leaves the first", ["config", "set", "mail.smtp.port", "2525", "--yes"], mutating=True,
     setup=sql("INSERT INTO OperationsSettings (ServerId,PublicUrl,TimeZone,MailingEnabled,MailFromAddress,MailSmtpHost,MailSmtpPort,MailSmtpUsername,MailSmtpSecurity,InviteMailEnabled,RecoveryMailEnabled,AlertMailEnabled,BackupRetentionDays,LogRetentionDays,OrphanObjectRetentionDays,AutomaticCleanupEnabled,MfaRequirement,TrustedDeviceDays,AuthChallengeMinutes,EmailCodeMinutes,EmailCodeCooldownSeconds,EmailCodeMaxAttempts,EmailCodeSendsPerHour,CliRecoveryMinutes,UpdatedAt) SELECT Id,'','Asia/Tokyo',1,'a@b.test','smtp.x',25,'u','tls',1,1,0,5,6,7,1,'admins',3,4,5,6,7,8,9,'2026-01-01 00:00:00' FROM Servers;"))
for key in ["time-zone", "mail.resend.enabled", "giphy.enabled", "mail.smtp.security", "mfa.requirement"]:
    one_of_each(f"config get {key} on stored settings", ["config", "get", key],
                setup=sql("INSERT INTO OperationsSettings (ServerId,PublicUrl,TimeZone,MailingEnabled,MailFromAddress,MailSmtpHost,MailSmtpPort,MailSmtpUsername,MailSmtpSecurity,MailResendEnabled,GiphyEnabled,InviteMailEnabled,RecoveryMailEnabled,AlertMailEnabled,BackupRetentionDays,LogRetentionDays,OrphanObjectRetentionDays,AutomaticCleanupEnabled,MfaRequirement,TrustedDeviceDays,AuthChallengeMinutes,EmailCodeMinutes,EmailCodeCooldownSeconds,EmailCodeMaxAttempts,EmailCodeSendsPerHour,CliRecoveryMinutes,UpdatedAt) SELECT Id,'','Asia/Tokyo',1,'a@b.test','smtp.x',25,'u','TLS',1,1,1,1,0,5,6,7,1,'Admins',3,4,5,6,7,8,9,'2026-01-01 00:00:00' FROM Servers;"))
for env_name, env in [("with a giphy key", {"STELLARR_GIPHY_API_KEY": "k"}), ("with a resend key", {"Mail__ResendApiKey": "re_x"})]:
    for key in ["giphy.enabled", "mail.resend.enabled"]:
        one_of_each(f"config set {key} {env_name}", ["config", "set", key, "true", "--yes"], mutating=True, env=env)

# Confirmation.
for answers, label in [(["n\n"], "n"), (["y\n"], "y"), (["yes\n"], "yes"), (["Y\n"], "Y"), (["\n"], "nothing"), (["maybe\n"], "maybe")]:
    case(f"config set asked, answered {label}", ["config", "set", "time-zone", "Europe/Warsaw"], mutating=True, tty=answers)
    case(f"config set --json asked, answered {label}", ["--json", "config", "set", "time-zone", "Europe/Warsaw"], mutating=True, tty=answers)

# Sessions.
SESSIONS = {"owner-a": "9f053d1d-f7ae-41cf-a968-cece47a42020", "maya": "6d7dee2a-18bb-40c0-8f4e-830c1e84383d"}
one_of_each("sessions revoke one", ["sessions", "revoke", SESSIONS["maya"], "--yes"], mutating=True)
one_of_each("sessions revoke one, upper case", ["sessions", "revoke", SESSIONS["maya"].upper(), "--yes"], mutating=True)
one_of_each("sessions revoke one, with spaces", ["sessions", "revoke", f"  {SESSIONS['maya']} ", "--yes"], mutating=True)
one_of_each("sessions revoke unknown id", ["sessions", "revoke", "00000000-0000-4000-8000-000000000000", "--yes"], mutating=True)
one_of_each("sessions revoke a bad id", ["sessions", "revoke", "nope", "--yes"], mutating=True)
one_of_each("sessions revoke without --yes", ["sessions", "revoke", SESSIONS["maya"]], mutating=True)
one_of_each("sessions revoke twice", ["sessions", "revoke", SESSIONS["maya"], "--yes"], mutating=True,
            setup=sql(f"UPDATE UserSessions SET RevokedAt = '2026-11-24 11:00:00' WHERE Id = '{SESSIONS['maya']}';"))
for who in [OWNER, MAYA, "dora", "nobody"]:
    one_of_each(f"sessions revoke all of {who}", ["sessions", "revoke", "--user", who, "--all", "--yes"], mutating=True)
one_of_each("sessions revoke all of maya, nothing to do", ["sessions", "revoke", "--user", MAYA, "--all", "--yes"], mutating=True,
            setup=sql("UPDATE UserSessions SET RevokedAt = '2026-11-24 11:00:00'; UPDATE DesktopBrowserLogins SET RedeemedAt = '2026-11-24 11:00:00';"))
one_of_each("sessions revoke all, without --yes", ["sessions", "revoke", "--user", MAYA, "--all"], mutating=True)
case("sessions revoke all, asked", ["sessions", "revoke", "--user", MAYA, "--all"], mutating=True, tty=["y\n"])
case("sessions revoke one, asked, answered no", ["sessions", "revoke", SESSIONS["maya"]], mutating=True, tty=["n\n"])

# Security.
case("security recover", ["--json", "security", "recover", MAYA, "--reason", "lost phone", "--yes"], mutating=True, volatile=True)
case("security recover, plain", ["security", "recover", MAYA, "--reason", "lost phone", "--yes"], mutating=True, volatile=True)
case("security recover by id", ["security", "recover", "ff591e72-0514-41e3-9e71-4668a3978948", "--reason", "  lost phone  ", "--yes"], mutating=True, volatile=True)
case("security recover, the owner", ["security", "recover", OWNER, "--reason", "abc", "--yes"], mutating=True, volatile=True)
case("security recover, 500 characters", ["security", "recover", MAYA, "--reason", "r" * 500, "--yes"], mutating=True, volatile=True)
case("security recover, 501 characters", ["security", "recover", MAYA, "--reason", "r" * 501, "--yes"], mutating=True, volatile=True)
case("security recover, 2 characters", ["security", "recover", MAYA, "--reason", "ab", "--yes"], mutating=True, volatile=True)
case("security recover, a disabled account", ["--json", "security", "recover", "dora", "--reason", "lost phone", "--yes"], mutating=True)
case("security recover, nobody", ["--json", "security", "recover", "nobody", "--reason", "lost phone", "--yes"], mutating=True)
case("security recover, a blank name", ["--json", "security", "recover", "  ", "--reason", "lost phone", "--yes"], mutating=True)
case("security recover, no --yes", ["--json", "security", "recover", MAYA, "--reason", "lost phone"], mutating=True)
case("security recover, asked, no", ["security", "recover", MAYA, "--reason", "lost phone"], mutating=True, tty=["n\n"], volatile=True)
case("security recover, asked, yes", ["security", "recover", MAYA, "--reason", "lost phone"], mutating=True, tty=["y\n"], volatile=True)
case("security recover with a shorter window", ["--json", "security", "recover", MAYA, "--reason", "lost phone", "--yes"], mutating=True, volatile=True,
     setup=sql("INSERT INTO OperationsSettings (ServerId,PublicUrl,TimeZone,MailingEnabled,MailFromAddress,MailSmtpHost,MailSmtpPort,MailSmtpUsername,MailSmtpSecurity,InviteMailEnabled,RecoveryMailEnabled,AlertMailEnabled,BackupRetentionDays,LogRetentionDays,OrphanObjectRetentionDays,AutomaticCleanupEnabled,MfaRequirement,TrustedDeviceDays,AuthChallengeMinutes,EmailCodeMinutes,EmailCodeCooldownSeconds,EmailCodeMaxAttempts,EmailCodeSendsPerHour,CliRecoveryMinutes,UpdatedAt) SELECT Id,'','UTC',0,'','',587,'','starttls',1,1,0,14,30,1,1,'everyone',14,5,8,45,5,10,3,'2026-01-01 00:00:00' FROM Servers;"))
VERIFIED = "UPDATE Users SET NormalizedEmail = 'MAYA@EXAMPLE.TEST' WHERE Username = 'maya';"
case("security recover, an account with a verified address", ["--json", "security", "recover", MAYA, "--reason", "lost phone", "--yes"], mutating=True, volatile=True, setup=sql(VERIFIED))
case("security recover twice", ["--json", "security", "recover", MAYA, "--reason", "again", "--yes"], mutating=True, volatile=True,
     setup=sql("INSERT INTO CliRecoveryCredentials (Id,UserId,TokenHash,Reason,SecurityVersion,CreatedAt,ExpiresAt,UsedAt) SELECT 'cc111111-1111-4111-8111-111111111111', Id, 'old-hash', 'earlier', 2, '2026-11-24 10:00:00', '2030-01-01 00:00:00', NULL FROM Users WHERE Username = 'maya';"))
IDENTITY = {"work": "e1111111-1111-4111-8111-111111111111", "default": "e2222222-2222-4222-8222-222222222222", "lab": "e3333333-3333-4333-8333-333333333333"}
one_of_each("security unlink one of two", ["security", "unlink", MAYA, IDENTITY["work"], "--yes"], mutating=True, volatile=True)
one_of_each("security unlink, upper case id", ["security", "unlink", MAYA, IDENTITY["work"].upper(), "--yes"], mutating=True, volatile=True)
one_of_each("security unlink someone else's", ["security", "unlink", MAYA, IDENTITY["lab"], "--yes"], mutating=True)
one_of_each("security unlink an unknown id", ["security", "unlink", MAYA, "00000000-0000-4000-8000-000000000000", "--yes"], mutating=True)
one_of_each("security unlink of nobody", ["security", "unlink", "nobody", IDENTITY["work"], "--yes"], mutating=True)
one_of_each("security unlink without --yes", ["security", "unlink", MAYA, IDENTITY["work"]], mutating=True)
case("security unlink, asked: the last way in warns (no password, none configured)", ["security", "unlink", MAYA, IDENTITY["default"]], mutating=True, tty=["n\n"],
     setup=sql("UPDATE Users SET PasswordHash = NULL WHERE Username = 'maya'; DELETE FROM ExternalIdentities WHERE Id = 'e1111111-1111-4111-8111-111111111111';"))
case("security unlink, asked: another provider is left", ["security", "unlink", MAYA, IDENTITY["work"]], mutating=True, tty=["n\n"], env={"Oidc__Providers__work__Authority": "https://x.test", "Oidc__Providers__work__ClientId": "c", "Oidc__Providers__work__ClientSecret": "s",
     "STELLARR_OIDC_ENABLED": "true", "STELLARR_OIDC_ISSUER": "https://o.test", "STELLARR_OIDC_CLIENT_ID": "c", "STELLARR_OIDC_CLIENT_SECRET": "s"},
     setup=sql("UPDATE Users SET PasswordHash = NULL WHERE Username = 'maya';"))
case("security unlink, asked: the provider is not configured here", ["security", "unlink", MAYA, IDENTITY["work"]], mutating=True, tty=["n\n"],
     setup=sql("UPDATE Users SET PasswordHash = NULL WHERE Username = 'maya';"))
case("security unlink, with a verified address", ["--json", "security", "unlink", MAYA, IDENTITY["work"], "--yes"], mutating=True, volatile=True, setup=sql(VERIFIED))

# Secrets (what each tool seals is its own: the rows are compared by name).
for value, label in [("super-secret\n", "a line"), (" spaced \n", "spaces"), ("a\r\n", "crlf"), ("", "nothing"), ("   \n", "blanks"), ("x" * 4096, "4096"), ("x" * 4097, "4097"), ("a\n\n", "two lines"), ("é😀\n", "unicode")]:
    one_of_each(f"secrets set via stdin: {label}", ["secrets", "set", "mail.smtp-password", "--stdin", "--yes"], stdin=value, mutating=True)
for name in ["mail.resend-api-key", "giphy.api-key", "cloudflare.r2.access-key", "cloudflare.r2.secret-key", "cloudflare.r2.worker-token"]:
    one_of_each(f"secrets set {name}", ["secrets", "set", name, "--stdin", "--yes"], stdin="value\n", mutating=True)
one_of_each("secrets set --stdin without --yes", ["secrets", "set", "mail.smtp-password", "--stdin"], stdin="v\n", mutating=True)
one_of_each("secrets set, no terminal, no --stdin", ["secrets", "set", "mail.smtp-password", "--yes"], mutating=True)
one_of_each("secrets set, no keys folder", ["secrets", "set", "mail.smtp-password", "--stdin", "--yes"], stdin="v\n", mutating=True, setup=lambda root: shutil.rmtree(os.path.join(root, "keys")))
one_of_each("secrets clear with no override", ["secrets", "clear", "mail.smtp-password", "--yes"], mutating=True)
one_of_each("secrets clear with no override, from the environment", ["secrets", "clear", "giphy.api-key", "--yes"], mutating=True, env={"STELLARR_GIPHY_API_KEY": "k"})
case("secrets set asked, hidden prompt", ["secrets", "set", "mail.smtp-password"], mutating=True, tty=["y\n", "hidden-value\n"])
case("secrets set asked, no", ["secrets", "set", "mail.smtp-password"], mutating=True, tty=["n\n"])

# Backups: the arguments and what is said before the library is asked.
one_of_each("backup cancel with nothing staged", ["backup", "cancel", "--yes"], mutating=True)
one_of_each("backup apply with nothing staged", ["backup", "apply", "--yes"], mutating=True)
one_of_each("backup verify of a file that is not an archive", ["backup", "verify", os.path.join(BASE, "db", "stellarr.db")])

print()
print(f"{results['same']} same, {results['known']} differ by decision, {results['different']} differ")
for text in differences:
    print(text)
sys.exit(1 if results["different"] else 0)
