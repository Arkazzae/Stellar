#!/usr/bin/env python3
"""How pl.json and en.json were made. Not run by the tests.

A server of the previous implementation ran on 127.0.0.1:5196 on an empty data
folder, in the language given as the argument, with these settings in the
environment: Stellarr__Language, Mail__Enabled=true, Mail__FromAddress=
noreply@chat.test, Operations__TimeZone=Europe/Warsaw and the rate limits
raised. A Mailpit listened on 51025 (SMTP) and 58025 (HTTP). This script
founds the instance, points its mail at Mailpit, makes it send one of each
kind it can (a delivery test, a password reset, a code, alerts, an address
change, a mention and a direct message) and saves what Mailpit read back to
captures/<lang>.json, which was then stripped of the fields the tests do not
use. Setup code: from the server's log."""
import hashlib, http.cookiejar, json, re, subprocess, sys, time, urllib.error, urllib.request

LANG = sys.argv[1]
BASE = "http://127.0.0.1:5196"
MAILPIT = "http://127.0.0.1:58025"
FIREFOX = "Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Gecko/20100101 Firefox/146.0"
EDGE = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36 Edg/142.0.0.0"
IPHONE = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1"


class Client:
    def __init__(self):
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar))

    def call(self, method, path, body=None, ua=None, kind="web"):
        h = {"X-Stellarr-Client": kind, "X-Stellarr-Client-Version": "1.0.0", "Content-Type": "application/json"}
        if ua is not None:
            h["User-Agent"] = ua
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(BASE + path, data=data, method=method, headers=h)
        try:
            with self.opener.open(req) as r:
                raw = r.read().decode()
                return r.status, (json.loads(raw) if raw else None)
        except urllib.error.HTTPError as e:
            raw = e.read().decode()
            try:
                return e.code, json.loads(raw)
            except Exception:
                return e.code, raw


def need(result, what, ok=(200, 201, 202, 204)):
    if result[0] not in ok:
        sys.exit(f"{what} failed: {result}")
    return result[1]


def mailpit(path, method="GET", raw=False):
    req = urllib.request.Request(MAILPIT + path, method=method)
    with urllib.request.urlopen(req) as r:
        data = r.read()
        if raw or not data.startswith((b'{', b'[')):
            return data
        return json.loads(data)


def message_count():
    return mailpit("/api/v1/messages")["total"]


def wait_for(count, seconds=40):
    end = time.time() + seconds
    while time.time() < end:
        if message_count() >= count:
            return
        time.sleep(1)
    sys.exit(f"only {message_count()} messages, wanted {count}")


def latest_text(subject_part):
    for m in mailpit("/api/v1/messages")["messages"]:
        if subject_part in m["Subject"]:
            return mailpit("/api/v1/message/" + m["ID"])["Text"]
    sys.exit("no mail " + subject_part)


mailpit("/api/v1/messages", method="DELETE")
setup_code = re.search(r"Setup code: (\S+?)\.", subprocess.run(
    ["docker", "logs", "stellarr-port-oracle-mail"], capture_output=True, text=True).stdout + subprocess.run(
    ["docker", "logs", "stellarr-port-oracle-mail"], capture_output=True, text=True).stderr).group(1)

owner = Client()
PW1, PW2 = "correct horse battery 1A!", "another horse battery 2B!"
need(owner.call("POST", "/api/auth/register", {
    "username": "owner", "displayName": "Jo <b>Owner</b>", "email": "owner@example.test", "password": PW1,
    "setupCode": setup_code, "initialServer": {"name": "Orbit & Co", "description": None}}, ua=FIREFOX), "register")
need(owner.call("POST", "/api/auth/login", {"username": "owner", "password": PW1}, ua=FIREFOX), "login")
settings = need(owner.call("GET", "/api/cockpit/settings", kind="cockpit"), "settings")
settings.update(smtpHost="host.docker.internal", smtpPort=51025, smtpSecurity="none", smtpUsername="",
                alertMailEnabled=True, resendEnabled=False, mailingEnabled=True, emailCodeCooldownSeconds=0)
need(owner.call("PUT", "/api/cockpit/settings", settings, kind="cockpit"), "save settings")

steps = []
need(owner.call("POST", "/api/cockpit/mail/test", {"to": "test@example.test"}, kind="cockpit"), "mail test")
need(owner.call("POST", "/api/auth/password/forgot", {"email": "owner@example.test"}, ua=FIREFOX), "forgot")
wait_for(2)

enroll = need(owner.call("POST", "/api/me/security/mfa/enroll/start", {"password": PW1}, ua=FIREFOX), "enroll start")
cid = enroll["challengeId"]
need(owner.call("POST", "/api/auth/mfa/enroll/email/start", {"challengeId": cid, "name": None}, ua=FIREFOX), "email start")
wait_for(3)
code = re.search(r"\n(\d{6})\r?\n", latest_text("")).group(1)
done = need(owner.call("POST", "/api/auth/mfa/enroll/email/confirm", {"challengeId": cid, "code": code, "name": None}, ua=FIREFOX), "confirm")
wait_for(4)  # the alert for the new method

need(owner.call("PATCH", "/api/me/preferences", {"emailNotifications": True}), "preferences")
invite = need(owner.call("POST", "/api/admin/invites", {"channelId": None}), "invite")
sam = Client()
need(sam.call("POST", "/api/auth/register", {"username": "sam", "displayName": "Sam", "email": "sam@example.test",
    "password": "sams horse battery 4D!", "inviteCode": invite["url"].rsplit("/", 1)[1]}), "register sam")
owner_id = need(owner.call("GET", "/api/me"), "me")["id"]
conversation = need(sam.call("POST", "/api/social/conversations", {"userId": owner_id}), "open conversation")
need(sam.call("POST", f"/api/social/conversations/{conversation['id']}/messages", {"content": "private hello"}), "dm")
boot = need(sam.call("GET", "/api/bootstrap"), "bootstrap")
channel = next(c for c in boot["channels"] if c["kind"].lower() == "text")
need(sam.call("POST", f"/api/channels/{channel['id']}/messages", {"content": "hey @owner look", "attachmentIds": []}), "mention")
time.sleep(6)

# Events that alert: a password change from Edge on Windows, a revoke from an iPhone.
need(owner.call("POST", "/api/me/password", {"currentPassword": PW1, "newPassword": PW2}, ua=EDGE), "password change")
need(owner.call("POST", "/api/me/security/sessions/revoke-others", {}, ua=IPHONE), "revoke others")
time.sleep(8)
need(owner.call("POST", "/api/me/email", {"email": "new.address@example.test", "password": PW2}, ua=FIREFOX), "email change")
link = re.search(r"(http://127\.0\.0\.1:5196/\S*token=\S+)", latest_text("")).group(1) if False else None
time.sleep(75)

events = {e["id"].replace("-", ""): e for e in need(owner.call("GET", "/api/me/security/events"), "events")} \
    if True else {}
# The events list may be nested.
captured = []
for m in mailpit("/api/v1/messages?limit=200")["messages"]:
    d = mailpit("/api/v1/message/" + m["ID"])
    inline = []
    for part in d.get("Inline", []) + d.get("Attachments", []):
        blob = mailpit(f"/api/v1/message/{m['ID']}/part/{part['PartID']}", raw=True)
        inline.append({"content_id": part["ContentID"], "file_name": part["FileName"], "content_type": part["ContentType"],
                       "sha256": hashlib.sha256(blob).hexdigest(), "size": len(blob)})
    raw = mailpit(f"/api/v1/message/{m['ID']}/raw", raw=True).decode("utf-8", "replace")
    captured.append({
        "id": m["ID"], "subject": d["Subject"], "text": d["Text"], "html": d["HTML"],
        "from": d["From"], "to": d["To"], "message_id": d["MessageID"], "date": d["Date"],
        "images": inline, "raw_head": raw.split("\r\n\r\n", 1)[0],
    })
out = {"language": LANG, "owner_id": owner_id, "events": events if isinstance(events, dict) else {}, "mails": captured,
       "code": code}
json.dump(out, open(f"captures/{LANG}.json", "w"), ensure_ascii=False, indent=1)
print(LANG, len(captured), "mails:", [c["subject"] for c in captured])
