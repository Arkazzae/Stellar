#!/usr/bin/env python3
"""Asks one server what it does with a stand-in (a placeholder of the Discord
import) at every route that names a person, signed in or not, and prints the
answers with ids and traces replaced, so that two servers on copies of one
database can be compared.

    probe.py BASE USERNAME PASSWORD PLACEHOLDER_ID PLACEHOLDER_NAME CHANNEL_ID VOICE_ID CONVERSATION_ID
"""
import json
import re
import sys
import urllib.error
import urllib.request

base, user, password, ghost, ghost_name, channel, voice, conversation = sys.argv[1:9]
HEADERS = {"X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0", "Content-Type": "application/json"}
ids = {}


def scrub(text):
    text = text.replace(ghost.lower(), "<ghost>").replace(ghost.upper(), "<ghost>")
    text = re.sub(r"00-[0-9a-f]{32}-[0-9a-f]{16}-00", "<trace>", text)

    def name(match):
        return "<id%d>" % ids.setdefault(match.group(0).lower(), len(ids) + 1)

    text = re.sub(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}", name, text)
    return re.sub(r"\d{4}-\d\d-\d\dT[\d:.]+Z?", "<time>", text)


def ask(cookie, method, path, body=None):
    request = urllib.request.Request(base + path, method=method, headers=dict(HEADERS, **({"Cookie": cookie} if cookie else {})),
                                     data=None if body is None else json.dumps(body).encode())
    try:
        with urllib.request.urlopen(request) as response:
            status, text = response.status, response.read().decode()
    except urllib.error.HTTPError as error:
        status, text = error.code, error.read().decode()
    print("%s %s -> %d %s" % (method, scrub(path), status, scrub(text)[:300]))
    return status


request = urllib.request.Request(base + "/api/auth/login", method="POST", headers=HEADERS,
                                 data=json.dumps({"username": user, "password": password}).encode())
with urllib.request.urlopen(request) as response:
    cookie = "; ".join(line.split(";")[0] for line in response.headers.get_all("Set-Cookie"))

# nobody signed in: the ways in
for name in (ghost_name, "discord:nobody-has-this"):
    ask(None, "POST", "/api/auth/login", {"username": name, "password": password})
    ask(None, "POST", "/api/auth/passkey/options", {"username": name})
    ask(None, "POST", "/api/auth/recovery/redeem", {"username": name, "code": "AAAA-BBBB-CCCC", "newPassword": "Another!2026-long"})
    ask(None, "POST", "/api/auth/password/forgot", {"email": name + "@example.com"})
    ask(None, "POST", "/api/auth/register", {"username": name, "displayName": "x", "email": "x@example.com", "password": password})
# an owner and the stand-in
for method, path, body in [
    ("DELETE", "/api/admin/channels/%s/members/%s" % (voice, ghost), None),
    ("PATCH", "/api/admin/channels/%s/members/%s" % (voice, ghost), {"targetChannelId": voice}),
    ("POST", "/api/social/conversations", {"userId": ghost}),
    ("POST", "/api/social/requests", {"username": ghost_name}),
    ("POST", "/api/social/blocks/" + ghost, None),
    ("DELETE", "/api/admin/members/" + ghost, None),
    ("POST", "/api/admin/members/%s/ban" % ghost, {"reason": "x"}),
    ("PATCH", "/api/admin/members/%s/role" % ghost, {"roleId": "00000000-0000-0000-0000-000000000000"}),
    ("GET", "/api/admin/members", None),
    ("GET", "/api/admin/bans", None),
    ("GET", "/api/me/encryption", None),
]:
    ask(cookie, method, path, body)
