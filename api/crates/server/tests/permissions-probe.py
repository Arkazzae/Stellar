#!/usr/bin/env python3
"""Talks to the first server for tests/permissions-shared-database.sh.

  permissions-probe.py register <base-url> <setup-code> <workdir> <count>
      founds the instance with `boss` when the setup code is given, registers
      `count - 1` more people, signs each in once and writes workdir/users.txt
      (`name id` lines, the owner of the arranged servers first) and
      workdir/cookies.json.
  permissions-probe.py observe <base-url> <workdir> <expected.jsonl>
      asks the bootstrap of every person for every server in the expected
      answers, and sends a message to every text channel, and writes
      workdir/observed.jsonl in the same shape.
  permissions-probe.py compare <expected.jsonl> <observed.jsonl>
"""
import json
import sys
import time
import urllib.error
import urllib.request
import uuid

PASSWORD = "Corona!2026-long"
CLIENT = {"X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0", "Content-Type": "application/json"}


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def call(base, method, path, body=None, cookie=None, headers=None):
    # Production counts attempts per window: wait out the limit instead of failing.
    while True:
        status, reply, answer_headers = send(base, method, path, body, cookie, headers)
        if status != 429:
            return status, reply, answer_headers
        time.sleep(float(answer_headers.get("Retry-After") or 5) + 1)


def send(base, method, path, body=None, cookie=None, headers=None):
    request = urllib.request.Request(base + path, method=method, data=json.dumps(body).encode() if body is not None else None)
    for name, value in {**CLIENT, **(headers or {})}.items():
        request.add_header(name, value)
    if cookie:
        request.add_header("Cookie", cookie)
    opener = urllib.request.build_opener(NoRedirect)
    try:
        with opener.open(request) as response:
            return response.status, response.read(), response.headers
    except urllib.error.HTTPError as error:
        return error.code, error.read(), error.headers


def session_cookie(headers):
    for line in headers.get_all("Set-Cookie") or []:
        first = line.split(";")[0]
        if "session" in first.split("=")[0]:
            return first
    raise SystemExit("no session cookie")


def register(base, setup, work, count):
    people = []
    cookies = {}
    status, body, _ = call(base, "POST", "/api/auth/register", {
        "username": "boss", "displayName": "Boss", "email": "boss@example.test", "password": PASSWORD, "setupCode": setup})
    if status not in (200, 201):
        raise SystemExit(f"founding failed: {status} {body[:200]}")
    status, body, headers = call(base, "POST", "/api/auth/login", {"username": "boss", "password": PASSWORD})
    cookies["boss"] = session_cookie(headers)
    people.append(("boss", json.loads(body)["id"]))
    status, body, _ = call(base, "POST", "/api/admin/invites", {}, cookies["boss"])
    invite = json.loads(body)["url"].rsplit("/", 1)[1]
    for number in range(1, count):
        name = f"person{number}"
        status, body, _ = call(base, "POST", "/api/auth/register", {
            "username": name, "displayName": name, "email": f"{name}@example.test", "password": PASSWORD, "inviteCode": invite})
        if status != 201:
            raise SystemExit(f"registering {name}: {status} {body[:200]}")
        status, body, headers = call(base, "POST", "/api/auth/login", {"username": name, "password": PASSWORD})
        cookies[name] = session_cookie(headers)
        people.append((name, json.loads(body)["id"]))
    with open(f"{work}/users.txt", "w") as users:
        users.write("".join(f"{name} {ident}\n" for name, ident in people))
    with open(f"{work}/cookies.json", "w") as out:
        json.dump(cookies, out)


def observe(base, work, expected_path):
    with open(f"{work}/cookies.json") as cookies_file:
        cookies = json.load(cookies_file)
    lines = []
    with open(expected_path) as expected:
        for raw in expected:
            want = json.loads(raw)
            cookie = cookies[want["user"]]
            server = {"X-Stellarr-Server": want["server"]}
            status, body, _ = call(base, "GET", "/api/bootstrap", None, cookie, server)
            if status != 200:
                lines.append({"user": want["user"], "server": want["server"], "status": status})
                continue
            data = json.loads(body)
            sent = {}
            for channel in want["send"]:
                status, reply, _ = call(base, "POST", f"/api/channels/{channel}/messages",
                                        {"content": "probe", "clientMessageId": str(uuid.uuid4())}, cookie, server)
                permission = None
                if status == 403:
                    permission = json.loads(reply).get("permission")
                sent[channel] = [status, permission]
            lines.append({
                "user": want["user"], "server": want["server"], "permissions": data["permissions"],
                "channels": sorted(c["id"] for c in data["channels"]),
                "sections": sorted(s["id"] for s in data["sections"]), "send": sent})
    with open(f"{work}/observed.jsonl", "w") as out:
        out.write("".join(json.dumps(line) + "\n" for line in lines))


def compare(expected_path, observed_path):
    with open(expected_path) as expected, open(observed_path) as observed:
        wanted = [json.loads(line) for line in expected]
        seen = [json.loads(line) for line in observed]
    differences = 0
    checks = 0
    for want, got in zip(wanted, seen):
        where = f"{want['user']} on {want['server'][:8]}"
        if "status" in got:
            print(f"DIFFERENT {where}: the first server answered {got['status']} to the bootstrap")
            differences += 1
            continue
        for key in ("permissions", "channels", "sections"):
            checks += 1
            if want[key] != got[key]:
                differences += 1
                print(f"DIFFERENT {where}: {key}\n   ours   {want[key]}\n   theirs {got[key]}")
        for channel, ours in want["send"].items():
            checks += 1
            status, permission = got["send"][channel]
            if status != ours or (status == 403 and permission != "sendMessages"):
                differences += 1
                print(f"DIFFERENT {where}: sending to {channel[:8]}: ours {ours}, theirs {status} {permission}")
    if len(wanted) != len(seen):
        differences += 1
        print("DIFFERENT: the number of answers")
    print(f"{len(wanted)} person-and-server pairs, {checks} comparisons, {differences} differences")
    return 1 if differences else 0


if __name__ == "__main__":
    command = sys.argv[1]
    if command == "register":
        register(sys.argv[2], sys.argv[3], sys.argv[4], int(sys.argv[5]))
    elif command == "observe":
        observe(sys.argv[2], sys.argv[3], sys.argv[4])
    elif command == "compare":
        sys.exit(compare(sys.argv[2], sys.argv[3]))
