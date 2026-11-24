#!/usr/bin/env python3
"""What each server sends when the same notifications are owed, side by side.

Both servers get a copy of one database that holds owed rows (made by the
first server: a mention and a direct message to people who have opted in, with
browsers subscribed and verified addresses). The first server runs in a
container; this one runs its worker in the test `oracle_worker` of the test
binary. Both send to the same fake push service (`fake-push.mjs`, which checks
the signed header and opens the body with the browser's keys) and to the same
Mailpit. What the fake saw, what Mailpit received and the rows afterwards are
compared.

The first server only connects to public addresses, so the container runs
with its Stellarr.Api.dll replaced by one built from the same source in which
only the socket to the name push.example.test is opened to the fake
(`host.docker.internal:5241`); everything it does around that is its own. This
server's guard is on, too: its sender is the one tests wire, with the loopback
as the one address it may reach (see `delivery/sender.rs`).

  PATCHED_DLL=dir with the patched Stellarr.Api.dll   API_TEST=path/to/the api test binary
  STELLARR_FIRST_PUBLISH=ref/publish/out  python3 -I delivery-wire-oracle.py [scenario ...]

LOOPS=n runs the scenarios n times over; DELIVERY_KEEP=dir keeps the full records of a pass that differs.

Containers: stellarr-port-delivery-first and stellarr-port-mail-delivery
(Mailpit); both are removed at the end.
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
import http.client

HERE = os.path.dirname(os.path.abspath(__file__))
WORK = tempfile.mkdtemp(prefix="delivery-wire-")
PATCHED = os.environ["PATCHED_DLL"]
PUBLISH = os.environ["STELLARR_FIRST_PUBLISH"]
API_TEST = os.environ["API_TEST"]
FIXTURES = os.path.join(HERE, "..", "src", "delivery", "tests", "fixtures")
FIRST = "stellarr-port-delivery-first"
MAILPIT = "stellarr-port-mail-delivery"
FIRST_PORT, FAKE_PORT, SMTP_PORT, MAIL_HTTP, REFUSER_PORT = 5221, 5241, 15221, 15222, 5242
PASSWORD = "Corona!2026-long"
HEADERS = {"X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0", "Content-Type": "application/json"}
PEOPLE = ["owner", "ada", "ben", "cleo"]
BROWSERS = {"ada": ["ada-1", "ada-2"], "ben": ["ben-1"], "cleo": ["cleo-1"]}


def run(*command, check=True, **kwargs):
    return subprocess.run(command, check=check, capture_output=True, text=True, **kwargs).stdout


def call(port, method, path, body=None, cookies=None):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=30)
    headers = dict(HEADERS)
    if cookies:
        headers["Cookie"] = "; ".join("%s=%s" % pair for pair in cookies.items())
    connection.request(method, path, json.dumps(body) if body is not None else None, headers)
    response = connection.getresponse()
    text = response.read().decode()
    for name, value in response.getheaders():
        if name.lower() == "set-cookie" and cookies is not None:
            key, _, rest = value.split(";")[0].partition("=")
            cookies[key] = rest
    connection.close()
    return response.status, text


def wait_ready(port, tries=120):
    for _ in range(tries):
        try:
            if call(port, "GET", "/health/ready")[0] == 200:
                return
        except OSError:
            pass
        time.sleep(0.5)
    raise SystemExit("not ready: %d" % port)


def node(script, *args):
    return run("node", "-e", script, *args)


def make_keys():
    out = json.loads(node('''
const c=require("crypto");const out={};
const pair=()=>{const k=c.generateKeyPairSync("ec",{namedCurve:"prime256v1"});const j=k.privateKey.export({format:"jwk"});
 return {d:j.d,x:j.x,y:j.y,pub:Buffer.concat([Buffer.from([4]),Buffer.from(j.x,"base64url"),Buffer.from(j.y,"base64url")]).toString("base64url")};};
out.vapid=pair();out.browsers={};
for(const n of process.argv.slice(1)){const p=pair();p.auth=c.randomBytes(16).toString("base64url");out.browsers["/send/"+n]=p;}
console.log(JSON.stringify(out));''', *[b for names in BROWSERS.values() for b in names]))
    return out


class Fake:
    def __init__(self, subs, vapid_public):
        path = os.path.join(WORK, "subs.json")
        json.dump(subs, open(path, "w"))
        self.process = subprocess.Popen(["node", os.path.join(HERE, "fake-push.mjs"), str(FAKE_PORT), os.path.join(FIXTURES, "server.crt.txt"),
                                         os.path.join(FIXTURES, "server.pk8.txt"), path, vapid_public], stdout=subprocess.DEVNULL)
        time.sleep(1)

    def ask(self, path):
        context = __import__("ssl")._create_unverified_context()
        return urllib.request.urlopen("https://127.0.0.1:%d%s" % (FAKE_PORT, path), context=context, timeout=10).read().decode()

    def behave(self, pass_="idle", **query):
        query["pass"] = pass_.replace("/", "_")
        self.ask("/__behave?" + "&".join("%s=%s" % pair for pair in query.items()))

    def log(self):
        return json.loads(self.ask("/__log"))

    def reset(self):
        self.ask("/__reset")

    def stop(self):
        self.process.terminate()


def first_container(data, extra, worker):
    run("docker", "rm", "-f", FIRST, check=False)
    env = {
        "ASPNETCORE_ENVIRONMENT": "Production", "ASPNETCORE_URLS": "http://0.0.0.0:5000", "STELLARR_DATA_ROOT": "/tmp/data",
        "ConnectionStrings__Database": "Data Source=/tmp/data/stellarr.db;Pooling=True",
        "Rtc__MediaControlUrl": "http://127.0.0.1:5200", "Rtc__MediaControlSecret": "oracle-media-secret-oracle-media-secret",
        "Rtc__TurnSharedSecret": "oracle-turn-secret-oracle-turn-secret-0123",
        "Operations__PublicUrl": "http://127.0.0.1:%d" % FIRST_PORT, "Desktop__CanonicalOrigin": "http://127.0.0.1:%d" % FIRST_PORT,
        "RateLimiting__AuthWrite__PermitLimit": "500", "RateLimiting__Login__PermitLimit": "500",
        "Push__Enabled": "true", "Push__Subject": "mailto:push@example.test",
        "Push__PublicKey": KEYS["vapid"]["pub"], "Push__PrivateKey": KEYS["vapid"]["d"],
        "SSL_CERT_FILE": "/tmp/ca.pem", "Notifications__WorkerEnabled": "true" if worker else "false",
    }
    env.update(extra)
    command = ["docker", "create", "--name", FIRST, "-p", "127.0.0.1:%d:5000" % FIRST_PORT, "-v", PUBLISH + ":/app:ro",
               "-v", PATCHED + ":/patch:ro", "-v", os.path.join(FIXTURES, "ca.crt.txt") + ":/tmp/ca.pem:ro"]
    for key, value in env.items():
        command += ["-e", "%s=%s" % (key, value)]
    command += ["stellarr-api-dev:local", "sh", "-c",
                "mkdir -p /opt/api && cp -r /app/. /opt/api/ && cp /patch/Stellarr.Api.dll /opt/api/ && cd /opt/api && exec dotnet Stellarr.Api.dll"]
    run(*command)
    # The data lives in the container's own file system: SQLite's write-ahead
    # log does not work reliably on a folder shared with the host.
    run("docker", "cp", data + "/.", FIRST + ":/tmp/data")
    run("docker", "start", FIRST)
    try:
        wait_ready(FIRST_PORT)
    except SystemExit:
        print(subprocess.run(["docker", "logs", "--tail", "15", FIRST], stderr=subprocess.STDOUT, stdout=subprocess.PIPE, text=True).stdout[-1500:])
        raise


def stop_first(data=None):
    run("docker", "stop", "-t", "10", FIRST, check=False)
    if data:
        run("docker", "cp", FIRST + ":/tmp/data/.", data)


def arrange():
    """Owed rows made by the first server: people, preferences, addresses,
    browsers, a mention, a direct message."""
    data = os.path.join(WORK, "arrange")
    os.mkdir(data)
    os.chmod(data, 0o777)
    run("docker", "rm", "-f", FIRST, check=False)
    first_container(data, {}, worker=False)
    setup = re.findall(r"Setup code: ([A-Z0-9-]+)\.", (lambda r: r.stdout + r.stderr)(subprocess.run(["docker", "logs", FIRST], capture_output=True, text=True)))[-1]
    owner = {}
    assert call(FIRST_PORT, "POST", "/api/auth/register", {"username": "owner", "displayName": "Owner", "email": "owner@example.test", "password": PASSWORD, "setupCode": setup}, owner)[0] == 201
    status, text = call(FIRST_PORT, "POST", "/api/admin/invites", {}, owner)
    invite = re.search(r'/([^/"]+)"', json.loads(text)["url"] + '"').group(1)
    for name in PEOPLE[1:]:
        status, text = call(FIRST_PORT, "POST", "/api/auth/register", {"username": name, "displayName": name.title(), "email": name + "@example.test", "password": PASSWORD, "inviteCode": invite}, {})
        assert status == 201, text
    sessions = {}
    for name in PEOPLE:
        sessions[name] = {}
        assert call(FIRST_PORT, "POST", "/api/auth/login", {"username": name, "password": PASSWORD}, sessions[name])[0] == 200
    for name, paths in BROWSERS.items():
        for path in paths:
            keys = KEYS["browsers"]["/send/" + path]
            status, text = call(FIRST_PORT, "PUT", "/api/me/push-subscriptions", {"endpoint": "https://push.example.test/send/" + path, "keys": {"p256dh": keys["pub"], "auth": keys["auth"]}}, sessions[name])
            assert status == 200, (status, text, path, sessions[name])
    stop_first(data)
    database = os.path.join(data, "stellarr.db")
    connection = sqlite3.connect(database)
    now = "2026-11-20 10:00:00"
    for name in PEOPLE:
        user = connection.execute('SELECT "Id" FROM "Users" WHERE "Username" = ?', (name,)).fetchone()[0]
        connection.execute('DELETE FROM "UserPreferences" WHERE "UserId" = ?', (user,))
        connection.execute('INSERT INTO "UserPreferences" ("UserId","DesktopNotifications","MentionPopups","CallRingtone","EmailNotifications","PushNotifications","MentionsOnly","MessageSound","JoinLeaveSound","AllowDirectMessages","AllowFriendRequests","ShowPresence","DeveloperMode","ReducedMotion","MutedChannelIdsJson","VadSensitivity") VALUES (?,1,1,1,1,1,1,1,1,1,1,1,0,0,\'[]\',60)', (user,))
        if name != "owner":
          connection.execute('INSERT INTO "MfaMethods" ("Id","UserId","Type","Name","CreatedAt","ConfirmedAt","EmailAddress") VALUES (lower(hex(randomblob(16))),?,\'email\',\'E-mail\',?,?,?)',
                             (user, now, now, (name + "@example.test").upper()))
    server = connection.execute('SELECT "Id" FROM "Servers" ORDER BY "CreatedAt" LIMIT 1').fetchone()[0]
    connection.execute('DELETE FROM "OperationsSettings" WHERE "ServerId" = ?', (server,))
    connection.execute('INSERT INTO "OperationsSettings" ("ServerId","PublicUrl","TimeZone","MailingEnabled","MailFromAddress","MailSmtpHost","MailSmtpPort","MailSmtpUsername","MailSmtpSecurity","MailResendEnabled","InviteMailEnabled","RecoveryMailEnabled","AlertMailEnabled","BackupRetentionDays","LogRetentionDays","OrphanObjectRetentionDays","AutomaticCleanupEnabled","UpdatedAt") VALUES (?,\'\',\'UTC\',1,\'noreply@chat.test\',\'host.docker.internal\',?,\'\',\'none\',NULL,1,1,0,14,30,1,1,?)',
                       (server, SMTP_PORT, now))
    channel = connection.execute('SELECT "Id" FROM "Channels" WHERE "Kind" = 0 ORDER BY "Position" LIMIT 1').fetchone()[0]
    connection.commit()
    connection.close()
    first_container(data, {}, worker=False)
    status, text = call(FIRST_PORT, "POST", "/api/channels/%s/messages" % channel.lower(),
                        {"clientMessageId": guid(os.urandom(16).hex()), "content": "Hej @ada @ben @cleo", "attachmentIds": []}, sessions["owner"])
    if status != 201:
        print(subprocess.run(["docker", "logs", "--tail", "25", FIRST], stderr=subprocess.STDOUT, stdout=subprocess.PIPE, text=True).stdout[-2500:])
    assert status == 201, text
    status, text = call(FIRST_PORT, "POST", "/api/social/conversations", {"userId": user_id(database, "ada")}, sessions["owner"])
    assert status in (200, 201), text
    conversation = json.loads(text)["id"]
    status, text = call(FIRST_PORT, "POST", "/api/social/conversations/%s/messages" % conversation, {"clientMessageId": guid(os.urandom(16).hex()), "content": "sekret", "attachmentIds": []}, sessions["owner"])
    if status != 201:
        print(subprocess.run(["docker", "logs", "--tail", "25", FIRST], stderr=subprocess.STDOUT, stdout=subprocess.PIPE, text=True).stdout[-2500:])
    assert status == 201, text
    stop_first(data)
    baseline = os.path.join(WORK, "baseline.db")
    sqlite3.connect(database).execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchall()
    shutil.copy(database, baseline)
    connection = sqlite3.connect(baseline)
    print("owed rows:", connection.execute('SELECT "Channel", "EventKind", COUNT(*) FROM "NotificationDeliveries" GROUP BY 1, 2').fetchall())
    connection.close()
    return baseline


def guid(text):
    text = text.replace("-", "")
    return "%s-%s-%s-%s-%s" % (text[:8], text[8:12], text[12:16], text[16:20], text[20:]) if len(text) == 32 else text


def user_id(database, name):
    return guid(sqlite3.connect(database).execute('SELECT "Id" FROM "Users" WHERE "Username" = ?', (name,)).fetchone()[0].lower())


def mailpit_messages():
    listing = json.loads(urllib.request.urlopen("http://127.0.0.1:%d/api/v1/messages" % MAIL_HTTP).read())
    out = []
    for item in listing["messages"]:
        message = json.loads(urllib.request.urlopen("http://127.0.0.1:%d/api/v1/message/%s" % (MAIL_HTTP, item["ID"])).read())
        out.append({"from": message["From"], "to": message["To"], "subject": message["Subject"], "text": message["Text"], "html": message["HTML"],
                    "inline": sorted((a["FileName"], a["ContentType"], a["Size"]) for a in message.get("Inline", [])),
                    "attachments": len(message.get("Attachments", []))})
    return sorted(out, key=lambda m: m["subject"] + m["to"][0]["Address"])


def clear_mailpit():
    req = urllib.request.Request("http://127.0.0.1:%d/api/v1/messages" % MAIL_HTTP, method="DELETE")
    urllib.request.urlopen(req).read()


def rows_of(database):
    connection = sqlite3.connect(database)
    def seconds(a, b):
        if a is None or b is None:
            return None
        def parse(text):
            whole, _, fraction = text.partition(".")
            return __import__("datetime").datetime.strptime(whole + "." + fraction[:6].ljust(6, "0"), "%Y-%m-%d %H:%M:%S.%f")
        return round((parse(a) - parse(b)).total_seconds())
    out = []
    for row in connection.execute('SELECT "Id","Channel","EventKind","DestinationKey","PushSubscriptionId","Attempts","Status","LastError","NextAttemptAt","LastAttemptAt","SentAt","LeaseId","LeaseUntil","CreatedAt","ExpiresAt" FROM "NotificationDeliveries" ORDER BY "Id"'):
        out.append({"id": row[0], "channel": row[1], "kind": row[2], "key": row[3], "subscription": row[4], "attempts": row[5], "status": row[6],
                    "error": row[7], "next_minus_last_s": seconds(row[8], row[9]) if row[6] != "sent" and row[6] != "cancelled" else None, "sent": row[10] is not None, "lease": row[11] is not None or row[12] is not None,
                    "expires_minus_created_s": seconds(row[14], row[13])})
    subs = sorted(r[0] for r in connection.execute('SELECT "Endpoint" FROM "PushSubscriptions"'))
    connection.close()
    return out, subs


def normalize_fake(log):
    out = []
    for entry in log:
        message = entry["message"]
        text = message.get("text")
        out.append({
            "path": entry["path"], "method": entry["method"],
            "headers": {k: v for k, v in entry["headers"].items() if k in ("ttl", "content-type", "user-agent", "urgency", "topic")},
            "vapid_ok": entry["vapid"].get("ok"), "aud": entry["vapid"].get("claims", {}).get("aud"), "sub": entry["vapid"].get("claims", {}).get("sub"),
            "error": message.get("error"),
            "json": json.loads(text) if text else None,
            "json_member_order": list(json.loads(text).keys()) if text else None,
        })
    return sorted(out, key=lambda e: (e["path"], json.dumps(e["json"], sort_keys=True)))


def forgive_leading_zero(first, this):
    """The first server's push library writes an ECDH secret that starts with a
    zero byte (one message in 256) one byte short, so the browser, and this
    fake, cannot open that push (measured: 27 of 6000 in a loop of the library
    alone). The push was sent and answered like any other; the entry is
    replaced by the one this server sent to the same path, so that the rest of
    the pass is still compared. Returns how many were replaced."""
    spare = [entry for entry in this]
    count = 0
    for index, entry in enumerate(first):
        if entry["error"] and "bad decrypt" in entry["error"]:
            held = [json.dumps(x["json"], sort_keys=True) for x in first]
            for other in spare:
                if other["path"] == entry["path"] and other["json"] is not None and json.dumps(other["json"], sort_keys=True) not in held:
                    first[index] = other
                    spare.remove(other)
                    count += 1
                    break
    first.sort(key=lambda e: (e["path"], json.dumps(e["json"], sort_keys=True)))
    return count


def header_report(log):
    """What differs by design between the two senders, shown not compared:
    the content coding (the first server's library writes the first draft's
    `aesgcm`, this one RFC 8291's `aes128gcm`) and the headers that go with it."""
    return [{"path": e["path"], "encoding": e["message"].get("encoding"), "record_size": e["message"].get("recordSize"), "header_names": e["rawHeaderNames"],
             "headers": {k: v for k, v in e["headers"].items() if k in ("content-encoding", "accept-encoding", "encryption", "content-length", "host", "connection")},
             "body_length": e["bodyLength"]} for e in log][:1]


SCENARIOS = {
    "ok-201": dict(behave=dict(mode="status", code=201), settle=14),
    "gone-404": dict(behave=dict(mode="status", code=404), settle=14),
    "gone-410": dict(behave=dict(mode="status", code=410), settle=14),
    "too-large-413": dict(behave=dict(mode="status", code=413), settle=14),
    "throttled-429": dict(behave=dict(mode="status", code=429), settle=14),
    "throttled-429-after": dict(behave=dict(mode="status", code=429, retryAfter=3600), settle=14),
    "error-500": dict(behave=dict(mode="status", code=500), settle=14),
    "big-answer": dict(behave=dict(mode="big", bytes=5000000), settle=16),
    "redirect-public": dict(behave=dict(mode="redirect", to="https://other.example.test:5241/elsewhere"), settle=14),
    "redirect-private": dict(behave=dict(mode="redirect", to="https://127.0.0.1/inside"), settle=14),
    "close-head": dict(behave=dict(mode="close-head"), settle=14),
    "close-body": dict(behave=dict(mode="close-body"), settle=14),
    "silent": dict(behave=dict(mode="silent"), settle=70),
    "polish": dict(behave=dict(mode="status", code=201), settle=14, language="pl"),
    "mail-not-set-up": dict(behave=dict(mode="status", code=201), settle=14, sql=['UPDATE "OperationsSettings" SET "MailingEnabled" = 0']),
    "mail-no-sender": dict(behave=dict(mode="status", code=201), settle=14, sql=['UPDATE "OperationsSettings" SET "MailFromAddress" = \'\'']),
    "mail-server-down": dict(behave=dict(mode="status", code=201), settle=14, sql=['UPDATE "OperationsSettings" SET "MailSmtpPort" = 59999']),
    "mail-refused": dict(behave=dict(mode="status", code=201), settle=14, sql=['UPDATE "OperationsSettings" SET "MailSmtpPort" = %d' % REFUSER_PORT]),
}


def prepare(baseline, name, host, scenario):
    folder = os.path.join(WORK, name)
    os.mkdir(folder)
    os.chmod(folder, 0o777)
    database = os.path.join(folder, "stellarr.db")
    shutil.copy(baseline, database)
    connection = sqlite3.connect(database)
    connection.execute('UPDATE "OperationsSettings" SET "MailSmtpHost" = ?', (host,))
    for statement in scenario.get("sql", []):
        connection.execute(statement)
    connection.commit()
    connection.close()
    return folder, database


def refuser():
    code = '''
import socketserver
class H(socketserver.StreamRequestHandler):
    def handle(self):
        self.wfile.write(b"220 refuser\\r\\n")
        for line in self.rfile:
            u = line.upper()
            if u.startswith(b"RCPT"): self.wfile.write(b"550 no such mailbox\\r\\n")
            elif u.startswith(b"QUIT"): self.wfile.write(b"221 bye\\r\\n"); return
            else: self.wfile.write(b"250 ok\\r\\n")
socketserver.TCPServer.allow_reuse_address = True
socketserver.ThreadingTCPServer(("0.0.0.0", %d), H).serve_forever()''' % REFUSER_PORT
    return subprocess.Popen(["python3", "-c", code])


UNSETTLED = ('SELECT COUNT(*) FROM "NotificationDeliveries" WHERE "LeaseId" IS NOT NULL '
             'OR ("Status" = \'pending\' AND "Attempts" = 0 AND "NextAttemptAt" <= ?)')


def first_pass(folder, scenario):
    extra = {}
    if scenario.get("language"):
        extra["Stellarr__Language"] = scenario["language"]
    first_container(folder, extra, worker=True)
    # Done when every row has had its try (or been put off); the settle time
    # is only the longest it may take.
    deadline = time.time() + scenario["settle"]
    time.sleep(3)
    while time.time() < deadline:
        look = os.path.join(WORK, "look")
        shutil.rmtree(look, ignore_errors=True)
        os.mkdir(look)
        for name in ("stellarr.db", "stellarr.db-wal", "stellarr.db-shm"):
            subprocess.run(["docker", "cp", "%s:/tmp/data/%s" % (FIRST, name), look], capture_output=True)
        try:
            connection = sqlite3.connect(os.path.join(look, "stellarr.db"))
            waiting = connection.execute(UNSETTLED, (time.strftime("%Y-%m-%d %H:%M:%S.000000", time.gmtime()),)).fetchone()[0]
            connection.close()
        except sqlite3.Error:
            waiting = 1
        if waiting == 0:
            break
        time.sleep(1)
    stop_first(folder)


def this_pass(database, scenario):
    env = dict(os.environ, DELIVERY_DB=database, FAKE_PORT=str(FAKE_PORT), FAKE_CA=os.path.join(FIXTURES, "ca.crt.txt"),
               RUN_SECONDS=str(scenario["settle"]), PUSH_PUBLIC_KEY=KEYS["vapid"]["pub"], PUSH_PRIVATE_KEY=KEYS["vapid"]["d"],
               PUBLIC_URL="http://127.0.0.1:%d" % FIRST_PORT, LANGUAGE=scenario.get("language", "en"))
    done = subprocess.run([API_TEST, "oracle_worker", "--ignored", "--nocapture"], env=env, capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit("this server's pass failed:\n" + (done.stdout + done.stderr)[-2500:])


def main():
    global KEYS
    KEYS = make_keys()
    run("docker", "rm", "-f", MAILPIT, check=False)
    run("docker", "run", "-d", "--name", MAILPIT, "-p", "127.0.0.1:%d:1025" % SMTP_PORT, "-p", "127.0.0.1:%d:8025" % MAIL_HTTP, "axllent/mailpit:v1.34.0")
    fake = Fake(KEYS["browsers"], KEYS["vapid"]["pub"])
    refusing = refuser()
    failures = 0
    try:
        baseline = arrange()
        chosen = (sys.argv[1:] or list(SCENARIOS)) * int(os.environ.get("LOOPS", "1"))
        for turn, name in enumerate(chosen):
            scenario = SCENARIOS[name]
            results = {}
            for label in ("first", "this"):
                fake.reset()
                clear_mailpit()
                fake.behave(pass_="%s/%s" % (name, label), **scenario["behave"])
                folder, database = prepare(baseline, "%s-%s-%d" % (name, label, turn), "host.docker.internal" if label == "first" else "127.0.0.1", scenario)
                if label == "first":
                    first_pass(folder, scenario)
                else:
                    this_pass(database, scenario)
                # Whatever arrives after this is not part of the pass: it is told
                # apart by the marker and reported, never compared.
                fake.behave(mode="status", code=599)
                log = fake.log()
                mine = [e for e in log if e["pass"] == "%s_%s" % (name, label)]
                strays = [e for e in log if e["pass"] != "%s_%s" % (name, label)]
                if strays:
                    print("  STRAY requests outside the pass of %s/%s: %s" % (name, label, [(e["path"], e["pass"], e["behaviour"]) for e in strays]))
                results[label] = {"fake": normalize_fake(mine), "headers": header_report(mine), "rows": rows_of(database), "mail": mailpit_messages(), "full": mine}
            lost = forgive_leading_zero(results["first"]["fake"], results["this"]["fake"])
            if lost:
                print("  note: %d push(es) of the first server could not be opened by the fake (its library drops a leading zero of the shared secret); matched by path" % lost)
            same = {part: results["first"][part] == results["this"][part] for part in ("fake", "rows", "mail")}
            print("== %s: requests %d/%d  fake %s  rows %s  mail %s (%d mails)" % (name, len(results["first"]["fake"]), len(results["this"]["fake"]),
                  "same" if same["fake"] else "DIFFERENT", "same" if same["rows"] else "DIFFERENT", "same" if same["mail"] else "DIFFERENT", len(results["first"]["mail"])))
            if not all(same.values()):
                failures += 1
                keep = os.environ.get("DELIVERY_KEEP")
                if keep:
                    os.makedirs(keep, exist_ok=True)
                    json.dump(results, open(os.path.join(keep, "%s-%d.json" % (name, int(time.time()))), "w"), indent=1)
                for part in ("fake", "rows", "mail"):
                    if not same[part]:
                        for one, other in zip(results["first"][part], results["this"][part]):
                            if one != other:
                                print("  first entry:", json.dumps(one, ensure_ascii=False)[:700])
                                print("  this  entry:", json.dumps(other, ensure_ascii=False)[:700])
                                break
                        print("  %s first: %s" % (part, json.dumps(results["first"][part], ensure_ascii=False)[:1500]))
                        print("  %s this : %s" % (part, json.dumps(results["this"][part], ensure_ascii=False)[:1500]))
            if name == "ok-201":
                print("  headers first:", json.dumps(results["first"]["headers"]))
                print("  headers this :", json.dumps(results["this"]["headers"]))
    finally:
        fake.stop()
        refusing.terminate()
        stop_first()
        run("docker", "rm", "-f", FIRST, MAILPIT, check=False)
        shutil.rmtree(WORK, ignore_errors=True)
    return 1 if failures else 0


sys.exit(main())
