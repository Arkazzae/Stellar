#!/usr/bin/env python3
"""The same security alert from the first server and from this one, side by side.

Each scenario founds an instance, makes the owner's address verified, signs the
owner in again (which queues an alert), points the instance's mail at a
server and lets the worker send. Both servers send to the same Mailpit; the
two messages are compared as Mailpit received them, and so are the rows each
leaves in the queue's table.

  oracle start mailer 5205 Production      # not needed: this script starts its own
  docker run -d --name stellarr-port-mailer-mailpit -p 127.0.0.1:15205:1025 -p 127.0.0.1:15206:8025 axllent/mailpit:v1.34.0
  STELLARR_API=path/to/stellarr-api MAILPIT=15205,15206 ORACLE=mailer ORACLE_PORT=5205 \
      python3 -I oracle-mail.py

Needs docker, the `oracle` script on the PATH (or ORACLE_SCRIPT), and the
first server's container name `stellarr-port-oracle-$ORACLE`. The first
server reaches Mailpit as host.docker.internal; this one as 127.0.0.1. The
SMTP server is a setting in the database, so the scripts stop each server,
edit its file and start it again, as oracle-shared-database.sh does.

What differs by nature is replaced before comparing: the moment, the address
the sign-in came from, ids. The wording of the error a failed try keeps is the
mail library's own and is only shown.
"""
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request

API = os.environ["STELLARR_API"]
ORACLE = os.environ.get("ORACLE", "mailer")
ORACLE_PORT = int(os.environ.get("ORACLE_PORT", "5205"))
ORACLE_SCRIPT = os.environ.get("ORACLE_SCRIPT", "oracle")
SMTP_PORT, MAILPIT_HTTP = (int(part) for part in os.environ.get("MAILPIT", "15205,15206").split(","))
CONTAINER = "stellarr-port-oracle-" + ORACLE
PUBLIC_URL = "http://127.0.0.1:%d" % ORACLE_PORT
PASSWORD = "Corona!2026-long"
AGENT = "Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Gecko/20100101 Firefox/146.0"
CLOSED_PORT = 59999
WORK = tempfile.mkdtemp(prefix="oracle-mail-")


def run(*command, **kwargs):
    return subprocess.run(command, check=True, capture_output=True, text=True, **kwargs).stdout


def http(method, url, body=None, headers=None):
    request = urllib.request.Request(url, method=method, data=body, headers=headers or {})
    with urllib.request.urlopen(request, timeout=20) as answer:
        return answer.status, answer.read()


def mailpit(method, path):
    status, body = http(method, "http://127.0.0.1:%d%s" % (MAILPIT_HTTP, path))
    try:
        return json.loads(body)
    except ValueError:
        return body


def post_json(base, path, value):
    headers = {
        "Content-Type": "application/json",
        "X-Stellarr-Client": "web",
        "X-Stellarr-Client-Version": "2.0.0",
        "User-Agent": AGENT,
    }
    return http("POST", base + path, json.dumps(value).encode(), headers)


def wait_ready(base):
    for _ in range(120):
        try:
            if http("GET", base + "/health/ready")[0] == 200:
                return
        except Exception:
            pass
        time.sleep(0.5)
    raise SystemExit("%s did not become ready" % base)


def now_text():
    return time.strftime("%Y-%m-%d %H:%M:%S", time.gmtime())


# ---- what is done to the database file of either server ----

def verify_address(db):
    db.execute('UPDATE "Users" SET "ProviderVerifiedEmail" = "NormalizedEmail"')


def point_mail_at(host, port):
    def edit(db):
        server = db.execute('SELECT "Id" FROM "Servers" ORDER BY "CreatedAt", "Id" LIMIT 1').fetchone()[0]
        db.execute(
            'INSERT OR REPLACE INTO "OperationsSettings" ("ServerId", "PublicUrl", "TimeZone", "MailingEnabled", '
            '"MailFromAddress", "MailSmtpHost", "MailSmtpPort", "MailSmtpUsername", "MailSmtpSecurity", '
            '"MailResendEnabled", "InviteMailEnabled", "RecoveryMailEnabled", "AlertMailEnabled", '
            '"BackupRetentionDays", "LogRetentionDays", "OrphanObjectRetentionDays", "AutomaticCleanupEnabled", '
            '"UpdatedAt") VALUES (?, "", "Europe/Warsaw", 1, "noreply@chat.test", ?, ?, "", "none", 0, 1, 1, 1, '
            '14, 30, 1, 1, ?)',
            (server, host, port, now_text()),
        )

    return edit


# ---- the first server ----

class Oracle:
    name = "first server"
    smtp_host = "host.docker.internal"

    def __init__(self, language):
        run(ORACLE_SCRIPT, "stop", ORACLE)
        env = os.path.join(WORK, "oracle.env")
        with open(env, "w") as handle:
            handle.write("Stellarr__Language=%s\n" % language)
        out = subprocess.run(
            [ORACLE_SCRIPT, "start", ORACLE, str(ORACLE_PORT), "Production"],
            check=True, capture_output=True, text=True, env=dict(os.environ, ORACLE_ENV_FILE=env),
        ).stdout
        self.base = PUBLIC_URL
        self.setup = re.search(r"Setup code: ([A-Z0-9-]+)\.", out).group(1)

    def stop(self):
        run("docker", "stop", "-t", "10", CONTAINER)

    def start(self):
        run("docker", "start", CONTAINER)
        wait_ready(self.base)

    def edit(self, change):
        self.stop()
        copy = os.path.join(WORK, "oracle.db")
        run("docker", "cp", CONTAINER + ":/tmp/data/stellarr.db", copy)
        db = sqlite3.connect(copy)
        change(db)
        db.commit()
        db.close()
        run("docker", "cp", copy, CONTAINER + ":/tmp/data/stellarr.db")
        self.start()

    def row(self):
        self.stop()
        copy = os.path.join(WORK, "oracle-after.db")
        run("docker", "cp", CONTAINER + ":/tmp/data/stellarr.db", copy)
        return read_row(copy)

    def close(self):
        run(ORACLE_SCRIPT, "stop", ORACLE)


# ---- this server ----

class Rust:
    name = "this server"
    smtp_host = "127.0.0.1"

    def __init__(self, language):
        self.folder = tempfile.mkdtemp(prefix="rust-", dir=WORK)
        self.language = language
        self.process = None
        self.start()
        self.base = self.address
        self.setup = re.search(
            r"Setup code: ([A-Z0-9-]+)\.", open(os.path.join(self.folder, "log1.txt")).read()
        ).group(1)

    def start(self):
        env = dict(
            ASPNETCORE_ENVIRONMENT="Production",
            ASPNETCORE_URLS="http://127.0.0.1:0",
            Rtc__TurnSharedSecret="oracle-turn-secret-oracle-turn-secret-0123",
            STELLARR_DATA_ROOT=self.folder,
            Operations__PublicUrl=PUBLIC_URL,
            Stellarr__Language=self.language,
        )
        env["ConnectionStrings__Database"] = "Data Source=%s;Pooling=True" % os.path.join(self.folder, "stellarr.db")
        self.starts = getattr(self, "starts", 0) + 1
        path = os.path.join(self.folder, "log%d.txt" % self.starts)
        log = open(path, "w")
        self.process = subprocess.Popen([API], env=env, stdout=log, stderr=subprocess.STDOUT)
        for _ in range(120):
            text = open(path).read()
            found = re.findall(r"Now listening on: (http://127.0.0.1:\d+)", text)
            if found:
                self.address = found[-1]
                wait_ready(self.address)
                self.base = self.address
                return
            time.sleep(0.25)
        raise SystemExit("this server did not start")

    def stop(self):
        self.process.terminate()
        self.process.wait(timeout=40)

    def edit(self, change):
        self.stop()
        db = sqlite3.connect(os.path.join(self.folder, "stellarr.db"))
        change(db)
        db.commit()
        db.close()
        self.start()

    def row(self):
        self.stop()
        return read_row(os.path.join(self.folder, "stellarr.db"))

    def close(self):
        if self.process and self.process.poll() is None:
            self.stop()


def read_row(path):
    db = sqlite3.connect(path)
    attempts, last, sent, error, created, due = db.execute(
        'SELECT "Attempts", "LastAttemptAt", "SentAt", "LastError", "CreatedAt", "NextAttemptAt" FROM "SecurityNotifications"'
    ).fetchone()
    db.close()

    def seconds(text):
        return time.mktime(time.strptime(text[:19], "%Y-%m-%d %H:%M:%S"))

    return {
        "attempts": attempts,
        "last attempt recorded": last is not None,
        "sent recorded": sent is not None,
        "error": error,
        "next attempt minus created (minutes)": round((seconds(due) - seconds(created)) / 60),
    }


# ---- one scenario ----

def scenario(kind, language, port):
    server = kind(language)
    try:
        status, _ = post_json(server.base, "/api/auth/register", {
            "username": "owner", "displayName": "Owner", "email": "owner@example.test",
            "password": PASSWORD, "setupCode": server.setup,
        })
        assert status in (200, 201), status
        server.edit(verify_address)
        assert post_json(server.base, "/api/auth/login", {"username": "owner", "password": PASSWORD})[0] == 200
        mailpit("DELETE", "/api/v1/messages")
        server.edit(point_mail_at(server.smtp_host, port))
        message = None
        for _ in range(240 if port == SMTP_PORT else 40):
            found = mailpit("GET", "/api/v1/messages")["messages"]
            if found:
                message = mailpit("GET", "/api/v1/message/" + found[0]["ID"])
                break
            time.sleep(0.25)
        return message, server.row()
    finally:
        server.close()


def scrub(text):
    text = re.sub(r"\d{1,2} [^\W\d_]+ \d{4}, \d{2}:\d{2}", "<moment>", text)
    text = re.sub(r"\b\d{1,3}(\.\d{1,3}){3}\b", "<ip>", text)
    text = re.sub(r"[0-9a-f]{32}", "<id>", text)
    return text


def describe(message):
    parts = lambda items: sorted(
        (item["ContentType"], item["FileName"], item["ContentID"], item["Size"]) for item in items
    )
    return {
        "subject": message["Subject"],
        "from": message["From"],
        "to": message["To"],
        "text": scrub(message["Text"]),
        "html": scrub(message["HTML"]),
        "inline images": parts(message["Inline"]),
        "attachments": parts(message["Attachments"]),
    }


def compare(label, first, second, shown=()):
    different = 0
    for key in sorted(set(first) | set(second)):
        if first.get(key) == second.get(key):
            print("  same      %s" % key)
        elif key in shown:
            print("  differs   %s (by nature)\n      first server: %r\n      this server:  %r" % (key, first.get(key), second.get(key)))
        else:
            different += 1
            print("  DIFFERS   %s\n      first server: %r\n      this server:  %r" % (key, first.get(key), second.get(key)))
    return different


def main():
    differences = 0
    try:
        for language in ("en", "pl"):
            print("== success, language %s" % language)
            sent = [scenario(kind, language, SMTP_PORT) for kind in (Oracle, Rust)]
            assert all(message for message, _ in sent), "a message did not arrive: %r" % [bool(m) for m, _ in sent]
            differences += compare("message", describe(sent[0][0]), describe(sent[1][0]))
            print(" rows left in the queue after success")
            differences += compare("row", sent[0][1], sent[1][1])
        print("== failure: nothing listens where the mail is sent")
        failed = [scenario(kind, "en", CLOSED_PORT) for kind in (Oracle, Rust)]
        print(" rows left in the queue after failure")
        differences += compare("row", failed[0][1], failed[1][1], shown=("error",))
    finally:
        shutil.rmtree(WORK, ignore_errors=True)
    print("\n%d difference(s)" % differences)
    sys.exit(1 if differences else 0)


if __name__ == "__main__":
    main()
