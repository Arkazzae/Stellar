#!/usr/bin/env python3
"""The routes a browser subscribes to push with, asked of both servers.

Both servers run on copies of one database file (an owner, a member and the
sign-ins of both), with the same Push settings, and get the same requests:
what a browser sees of each answer is compared, and so are the rows the
requests leave in PushSubscriptions.

  ORACLE_ENV_FILE=oracle.env oracle start delivery 5221 Production
  # oracle.env: RateLimiting__AuthWrite__PermitLimit=500, RateLimiting__Login__PermitLimit=500,
  #             Push__Enabled=true, Push__Subject, Push__PublicKey, Push__PrivateKey
  STELLARR_API=path/to/stellarr-api ORACLE=delivery ORACLE_PORT=5221 \
      PUSH_PUBLIC_KEY=... PUSH_PRIVATE_KEY=... python3 -I delivery-oracle.py

Needs docker and the first server's container `stellarr-port-oracle-$ORACLE`
started fresh (it founds the instance). Prints every difference and exits 1
if there is one.
"""
import base64
import http.client
import json
import os
import re
import sqlite3
import subprocess
import sys
import tempfile
import time

API = os.environ["STELLARR_API"]
ORACLE = os.environ.get("ORACLE", "delivery")
PORT = int(os.environ.get("ORACLE_PORT", "5221"))
CONTAINER = "stellarr-port-oracle-" + ORACLE
PASSWORD = "Corona!2026-long"
WORK = tempfile.mkdtemp(prefix="delivery-oracle-")
HEADERS = {"X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0"}
P256 = "BGsX0fLhLEJH-Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT-NC4v4af5uO5-tKfA-eFivOM1drMV7Oy7ZAaDe_UfU"
AUTH = "AAAAAAAAAAAAAAAAAAAAAA"


class Client:
    def __init__(self, port):
        self.port = port
        self.cookies = {}

    def call(self, method, path, body=None, content_type="application/json", raw=False):
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=30)
        headers = dict(HEADERS)
        if self.cookies:
            headers["Cookie"] = "; ".join("%s=%s" % pair for pair in self.cookies.items())
        data = None
        if body is not None:
            data = body if raw else json.dumps(body)
            if content_type:
                headers["Content-Type"] = content_type
        connection.request(method, path, data.encode() if data is not None else None, headers)
        response = connection.getresponse()
        text = response.read().decode("utf-8", "replace")
        for name, value in response.getheaders():
            if name.lower() == "set-cookie":
                pair = value.split(";")[0]
                key, _, val = pair.partition("=")
                if val:
                    self.cookies[key] = val
        connection.close()
        return response.status, dict((k.lower(), v) for k, v in response.getheaders()), text


def run(*command):
    return subprocess.run(command, check=True, capture_output=True, text=True).stdout


def wait(port):
    for _ in range(120):
        try:
            c = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
            c.request("GET", "/health/ready")
            if c.getresponse().status == 200:
                return
        except OSError:
            pass
        time.sleep(0.5)
    raise SystemExit("not ready: %d" % port)


def scrub(text):
    text = re.sub(r'"traceId":"[^"]*"', '"traceId":"T"', text)
    text = re.sub(r'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}', "ID", text)
    return text


def seen(result):
    status, headers, text = result
    keep = {k: v for k, v in headers.items() if k in ("content-type", "cache-control", "www-authenticate", "allow")}
    return (status, sorted(keep.items()), scrub(text))


def b64(raw):
    return base64.urlsafe_b64encode(raw).decode().rstrip("=")


def main():
    oracle = Client(PORT)
    setup = re.findall(r"Setup code: ([A-Z0-9-]+)\.", run("docker", "logs", CONTAINER))[-1]
    status, _, text = oracle.call("POST", "/api/auth/register", {"username": "owner", "displayName": "Owner", "email": "owner@example.test", "password": PASSWORD, "setupCode": setup})
    assert status == 201, text
    status, _, text = oracle.call("POST", "/api/admin/invites", {})
    invite = re.search(r'/([^/"]+)"', json.loads(text)["url"] + '"').group(1)
    status, _, text = oracle.call("POST", "/api/auth/register", {"username": "ada", "displayName": "Ada", "email": "ada@example.test", "password": PASSWORD, "inviteCode": invite})
    assert status == 201, text
    run("docker", "stop", "-t", "10", CONTAINER)
    database = os.path.join(WORK, "oracle.db")
    run("docker", "cp", CONTAINER + ":/tmp/data/stellarr.db", database)
    run("docker", "start", CONTAINER)
    wait(PORT)
    mine = os.path.join(WORK, "rust")
    os.mkdir(mine)
    for suffix in ("", "-wal", "-shm"):
        if os.path.exists(database + suffix):
            run("cp", database + suffix, os.path.join(mine, "stellarr.db" + suffix))
    env = dict(os.environ, ASPNETCORE_ENVIRONMENT="Production", Rtc__TurnSharedSecret="x", STELLARR_DATA_ROOT=mine,
               ASPNETCORE_URLS="http://127.0.0.1:5222", Operations__PublicUrl="http://127.0.0.1:5222",
               RateLimiting__AuthWrite__PermitLimit="500", RateLimiting__Login__PermitLimit="500",
               STELLARR_PUSH_ENABLED="true", STELLARR_PUSH_SUBJECT="mailto:push@example.test",
               STELLARR_PUSH_PUBLIC_KEY=os.environ["PUSH_PUBLIC_KEY"], STELLARR_PUSH_PRIVATE_KEY=os.environ["PUSH_PRIVATE_KEY"])
    env["ConnectionStrings__Database"] = "Data Source=%s/stellarr.db" % mine
    process = subprocess.Popen([API], env=env, stdout=open(os.path.join(mine, "log.txt"), "w"), stderr=subprocess.STDOUT)
    try:
        wait(5222)
        ours = Client(5222)
        servers = [("first", oracle), ("this", ours)]

        def people():
            out = {}
            for label, client in servers:
                who = {}
                for name in ("owner", "owner2", "ada"):
                    c = Client(client.port)
                    status, _, text = c.call("POST", "/api/auth/login", {"username": name.rstrip("2"), "password": PASSWORD})
                    assert status == 200, (label, name, text)
                    who[name] = c
                out[label] = who
            return out

        who = people()
        differences = []
        count = 0

        def both(person, method, path, body=None, **kwargs):
            nonlocal count
            results = [seen(who[label][person].call(method, path, body, **kwargs)) for label, _ in servers]
            count += 1
            if results[0] != results[1]:
                differences.append((person, method, path, body if kwargs.get("raw") is None else kwargs, results))
            return results[0]

        def sub(endpoint, p256dh=P256, auth=AUTH):
            return {"endpoint": endpoint, "keys": {"p256dh": p256dh, "auth": auth}}

        both("owner", "GET", "/api/me/notification-delivery")
        for body in ["", "null", "{}", '{"endpoint":null}', '{"keys":null}', '{"keys":{}}', "{bad", '{"endpoint":5}', "[]", '"x"',
                     '{"endpoint":"x","keys":5}', '{"Endpoint":"https://push.example.test/a","Keys":{"P256dh":"%s","Auth":"%s"}}' % (P256, AUTH)]:
            for method in ("PUT", "DELETE"):
                both("owner", method, "/api/me/push-subscriptions", body, raw=True)
                both("owner", method, "/api/me/push-subscriptions", body, raw=True, content_type="text/plain")
                both("owner", method, "/api/me/push-subscriptions", body, raw=True, content_type=None)
        both("owner", "PUT", "/api/me/push-subscriptions")
        both("owner", "DELETE", "/api/me/push-subscriptions")
        both("owner", "POST", "/api/me/push-subscriptions", {})
        both("owner", "GET", "/api/me/push-subscriptions")

        corpus = [l.rstrip("\n") for l in open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "delivery-endpoints.txt"))]
        for endpoint in corpus:
            endpoint = json.loads(endpoint)
            first = both("owner", "PUT", "/api/me/push-subscriptions", sub(endpoint))
            if first[0] == 200:
                both("owner", "DELETE", "/api/me/push-subscriptions", {"endpoint": endpoint})
            else:
                both("owner", "DELETE", "/api/me/push-subscriptions", {"endpoint": endpoint})

        good = "https://push.example.test/send/"
        key_variants = [(P256, AUTH), (P256 + "=", AUTH), (P256, AUTH + "=="), (P256, AUTH[:-1]), (P256[:-1], AUTH), ("", AUTH), (P256, ""),
                        (b64(b"\x04" + b"\x00" * 64), AUTH), (b64(b"\x00" * 65), AUTH), ("A" * 300, AUTH), (P256, "A" * 70), (P256, b64(b"a" * 15)),
                        (P256, b64(b"a" * 17)), (P256 + "+", AUTH), (" " + P256, AUTH), (P256, "AAAAAAAAAAAAAAAAAAAAAA "), (P256.replace("-", "+"), AUTH),
                        (None, AUTH), (P256, None)]
        for index, (p, a) in enumerate(key_variants):
            body = {"endpoint": good + "k%d" % index, "keys": {"p256dh": p, "auth": a}}
            both("owner", "PUT", "/api/me/push-subscriptions", body)
            both("owner", "DELETE", "/api/me/push-subscriptions", {"endpoint": good + "k%d" % index})
        both("owner", "PUT", "/api/me/push-subscriptions", {"endpoint": good + "nokeys"})
        both("owner", "PUT", "/api/me/push-subscriptions", {"endpoint": "x" * 2100, "keys": {"p256dh": P256, "auth": AUTH}})

        # Ownership, sign-ins and the limit.
        both("owner", "PUT", "/api/me/push-subscriptions", sub(good + "mine"))
        both("owner", "PUT", "/api/me/push-subscriptions", sub(good + "mine"))
        both("owner", "PUT", "/api/me/push-subscriptions", sub(good + "mine", p256dh=b64(b"\x04" + bytes.fromhex("6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296") + bytes.fromhex("4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5"))))
        both("ada", "PUT", "/api/me/push-subscriptions", sub(good + "mine"))
        both("owner2", "PUT", "/api/me/push-subscriptions", sub(good + "mine"))
        both("ada", "DELETE", "/api/me/push-subscriptions", {"endpoint": good + "mine"})
        both("owner2", "DELETE", "/api/me/push-subscriptions", {"endpoint": good + "mine"})
        both("owner", "PUT", "/api/me/push-subscriptions", sub(good.upper().replace("HTTPS", "https", 1) + "mine"))
        both("owner", "PUT", "/api/me/push-subscriptions", sub("HTTPS://PUSH.EXAMPLE.TEST:443/send/mine"))
        for index in range(12):
            both("owner", "PUT", "/api/me/push-subscriptions", sub(good + "n%d" % index))
        both("owner", "PUT", "/api/me/push-subscriptions", sub(good + "n3"))
        both("ada", "PUT", "/api/me/push-subscriptions", sub(good + "ada"))
        for index in range(12):
            both("owner", "DELETE", "/api/me/push-subscriptions", {"endpoint": good + "n%d" % index})
        both("owner", "DELETE", "/api/me/push-subscriptions", {"endpoint": good + "mine"})

        # Where this server refuses what the first accepted (a stricter
        # reading of an endpoint) is told apart from every other difference:
        # the first is by design, the rest is shown whole.
        stricter, other = [], []
        for item in differences:
            first, this = item[4]
            if item[1] in ("PUT", "DELETE") and first[0] in (200, 204) and this[0] == 400 and "subscription" in this[2]:
                stricter.append(item)
            else:
                other.append(item)
        print(json.dumps({"requests": count, "differences": len(differences), "this_stricter": len(stricter), "other": len(other)}))
        for item in stricter[:8]:
            print("stricter:", json.dumps(item[3], ensure_ascii=False)[:160])
        for item in other[:60]:
            print(json.dumps(item, ensure_ascii=False)[:700])
        differences = other
        # The rows each left.
        run("docker", "stop", "-t", "10", CONTAINER)
        first_db = os.path.join(WORK, "first-after.db")
        run("docker", "cp", CONTAINER + ":/tmp/data/stellarr.db", first_db)
        for suffix in ("-wal", "-shm"):
            subprocess.run(["docker", "cp", CONTAINER + ":/tmp/data/stellarr.db" + suffix, first_db + suffix], capture_output=True)
        process.terminate()
        process.wait(10)

        def rows(path):
            connection = sqlite3.connect(path)
            return sorted(connection.execute('SELECT "Endpoint", "P256dh", "Auth", (SELECT "Username" FROM "Users" WHERE "Id" = "UserId") FROM "PushSubscriptions"').fetchall())

        a, b = rows(first_db), rows(os.path.join(mine, "stellarr.db"))
        print("rows first:", len(a), "this:", len(b), "equal:", a == b)
        if a != b:
            print("only first:", [r for r in a if r not in b][:10])
            print("only this:", [r for r in b if r not in a][:10])
        return 1 if differences or a != b else 0
    finally:
        if process.poll() is None:
            process.terminate()


sys.exit(main())
