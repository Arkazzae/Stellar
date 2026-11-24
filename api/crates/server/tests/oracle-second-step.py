#!/usr/bin/env python3
"""The routes of the second step and of the password reset, asked of the first
server and of this one, request by request.

Each scenario is one conversation, run on each server by the same code: what
was sent, what came back (status, the headers a browser cares about, every
`Set-Cookie` with its attributes, the JSON as text so that the order of its
members counts), the mails that arrived, and what the conversation left in
the database (security events, challenges, sessions, remembered devices). What
differs by nature is replaced before comparing: ids, moments, secrets, the
values of cookies, trace identifiers, the digits of a code.

An authenticator app's secret is sealed by each server with keys of its own, so
each server enrols its own, through its own routes, and the two conversations
are compared, not one file. The codes come from totp-independent.py.

What it needs:
  docker, python3 (standard library only), a Mailpit of your own
  (`docker run -d --name stellarr-port-mail-secondstep -p 127.0.0.1:5252:1025
  -p 127.0.0.1:5253:8025 axllent/mailpit:v1.34.0`), the `oracle` script, a
  built `stellarr-api`, and for the recovery scenario the first server's own
  command line (`STELLARR_CLI`, a built Stellarr.Cli.dll, run with `dotnet`).

  STELLARR_API=path/to/stellarr-api ORACLE_SCRIPT=path/to/oracle \
      python3 -I oracle-second-step.py [scenario ...]

The first server's limits are raised through ORACLE_ENV_FILE (see below), and
both servers get the same raised limits, except in the `limits` scenario,
which starts both with the defaults and shows one refusal of each policy.

Every run starts a fresh first server and a fresh one of this crate, and stops
them at the end.

The scenarios whose names start with `sso` put an identity provider in the
conversation: a fake one (providers-oracle/fake-provider.mjs) over TLS that both
servers are pointed at. The first server runs in a container and this one on
the host, so the provider is reached by this machine's address on the network
(`ipconfig getifaddr en0`), which has to be given:

  PROVIDERS_ORACLE_HOST=<that address> STELLARR_API=... ORACLE_SCRIPT=... \
      python3 -I oracle-second-step.py sso_link sso_security sso_reauth sso_policy sso_named

The script makes a throwaway certificate authority for that address, starts the
fake provider on PROVIDER_PORT (control: PROVIDER_CONTROL), tells both servers
to trust the authority through SSL_CERT_FILE, and stops the provider with the
servers. `node` and `openssl` are needed for these. Every other scenario runs
without a provider, as before.

Ports come from the environment so that several people can run this at once:
ORACLE (container name), ORACLE_PORT, RUST_PORT, MAIL_SMTP_PORT, MAIL_HTTP_PORT,
PROVIDER_PORT and PROVIDER_CONTROL.
"""
import base64
import hashlib
import http.client
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request

API = os.environ["STELLARR_API"]
ORACLE_SCRIPT = os.environ.get("ORACLE_SCRIPT", "oracle")
ORACLE = os.environ.get("ORACLE", "secondstep")
ORACLE_PORT = int(os.environ.get("ORACLE_PORT", "5222"))
RUST_PORT = int(os.environ.get("RUST_PORT", "5251"))
SMTP_PORT = int(os.environ.get("MAIL_SMTP_PORT", "5252"))
MAIL_HTTP = int(os.environ.get("MAIL_HTTP_PORT", "5253"))
CONTAINER = "stellarr-port-oracle-" + ORACLE
HERE = os.path.dirname(os.path.abspath(__file__))
TOTP = os.path.join(HERE, "totp-independent.py")
PASSWORD = "Corona!2026-long"
AGENT = "Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Gecko/20100101 Firefox/146.0"
WORK = tempfile.mkdtemp(prefix="oracle-second-step-")
PROVIDERS_HOST = os.environ.get("PROVIDERS_ORACLE_HOST", "")
PROVIDER_PORT = int(os.environ.get("PROVIDER_PORT", "5301"))
PROVIDER_CONTROL = int(os.environ.get("PROVIDER_CONTROL", "5302"))
RAISED = {
    "RateLimiting__AuthWrite__PermitLimit": "5000",
    "RateLimiting__Login__PermitLimit": "5000",
    "RateLimiting__LoginPoll__PermitLimit": "5000",
    "RateLimiting__SecurityWrite__PermitLimit": "5000",
}


def run(*command, **kwargs):
    return subprocess.run(command, check=True, capture_output=True, text=True, **kwargs).stdout


def now_text(delta=0):
    moment = time.time() + delta
    return time.strftime("%Y-%m-%d %H:%M:%S", time.gmtime(moment)) + ".%06d" % int((moment % 1) * 1000000)


def totp_now(secret, steps=0):
    return run("python3", "-I", TOTP, "now", secret, str(steps)).strip()


def wait_ready(base):
    for _ in range(240):
        try:
            with urllib.request.urlopen(base + "/health/ready", timeout=2) as answer:
                if answer.status == 200:
                    return
        except Exception:
            pass
        time.sleep(0.5)
    raise SystemExit("%s did not become ready" % base)


def password_hash(password, iterations=210000):
    salt = os.urandom(16)
    digest = hashlib.pbkdf2_hmac("sha256", password.encode(), salt, iterations, 32)
    return "pbkdf2-sha256$%d$%s$%s" % (iterations, base64.b64encode(salt).decode(), base64.b64encode(digest).decode())


def token_hash(text):
    return hashlib.sha256(text.encode()).hexdigest().upper()


def new_id():
    import uuid
    return str(uuid.uuid4()).upper()


# ---- the identity provider ----

FAKE = {"process": None}


def provider_settings(kind):
    """The settings that point a server at the fake provider. `default` is the
    original provider (/p0, which does not vouch for a second step) beside two
    named ones: `trust` (/p1, whose second step counts) and `plain` (/p2).
    `named` is an instance with named providers only."""
    if not kind:
        return {}
    base = "https://%s:%d" % (PROVIDERS_HOST, PROVIDER_PORT)
    found = {}

    def named(name, number, title, trust=False):
        prefix = "Oidc__Providers__%s__" % name
        found.update({
            prefix + "Authority": "%s/p%d" % (base, number), prefix + "ClientId": "stellarr-tests",
            prefix + "ClientSecret": "test-client-secret", prefix + "DisplayName": title,
        })
        if trust:
            found[prefix + "TrustProviderMfa"] = "true"

    if kind == "default":
        found.update({
            "Oidc__Enabled": "true", "Oidc__Authority": base + "/p0", "Oidc__ClientId": "stellarr-tests",
            "Oidc__ClientSecret": "test-client-secret", "Oidc__DisplayName": "Test IdP",
        })
        named("trust", 1, "Trusted", trust=True)
        named("plain", 2, "Plain")
    else:
        named("work", 3, "Work")
    return found


def start_provider():
    """A certificate authority for this machine's address, and the fake
    provider behind a certificate it signed. Stopped with the servers."""
    if not PROVIDERS_HOST:
        raise SystemExit("set PROVIDERS_ORACLE_HOST to this machine's address on the network (ipconfig getifaddr en0)")
    here = os.path.join(HERE, "providers-oracle")
    quiet = {"check": True, "capture_output": True}
    subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
                    "-keyout", "ca.key", "-out", "ca.pem", "-days", "2", "-subj", "/CN=second step oracle CA",
                    "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign"], cwd=WORK, **quiet)
    subprocess.run(["openssl", "req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
                    "-keyout", "leaf.key", "-out", "leaf.csr", "-subj", "/CN=fake"], cwd=WORK, **quiet)
    with open(os.path.join(WORK, "ext.cnf"), "w") as handle:
        handle.write("subjectAltName=IP:%s,IP:127.0.0.1\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature\nextendedKeyUsage=serverAuth\n" % PROVIDERS_HOST)
    subprocess.run(["openssl", "x509", "-req", "-in", "leaf.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial",
                    "-out", "leaf.pem", "-days", "2", "-extfile", "ext.cnf"], cwd=WORK, **quiet)
    FAKE["process"] = subprocess.Popen(
        ["node", os.path.join(here, "fake-provider.mjs"), PROVIDERS_HOST, str(PROVIDER_PORT), str(PROVIDER_CONTROL), "leaf.pem", "leaf.key"],
        cwd=WORK, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def stop_provider():
    process = FAKE["process"]
    if process and process.poll() is None:
        process.terminate()
        process.wait(timeout=10)
    FAKE["process"] = None


def provider(prefix, **change):
    """Scripts what the provider at /<prefix> says next (see fake-provider.mjs)."""
    request = urllib.request.Request("http://127.0.0.1:%d/%s" % (PROVIDER_CONTROL, prefix), data=json.dumps(change).encode(), method="POST")
    urllib.request.urlopen(request, timeout=10).read()


def provider_reset(prefix, **change):
    """What a provider says by default, plus `change`: so that no scenario sees what an earlier one left."""
    fresh = {"subject": "subject", "amr": None, "authTime": None, "claims": {}, "userinfo": None, "signing": "published", "instead": {}}
    fresh.update(change)
    provider(prefix, **fresh)


# ---- the two servers ----

class Server:
    """What a scenario needs of either server."""

    def edit(self, *statements):
        """Runs SQL on the server's database file while the server is stopped."""
        raise NotImplementedError

    def database(self):
        """A read-only copy of the database file."""
        raise NotImplementedError


class Oracle(Server):
    name = "first server"
    smtp_host = "host.docker.internal"

    def __init__(self, limits_raised=True, providers=None):
        subprocess.run([ORACLE_SCRIPT, "stop", ORACLE], capture_output=True)
        env = os.path.join(WORK, "oracle.env")
        with open(env, "w") as handle:
            for key, value in (RAISED if limits_raised else {}).items():
                handle.write("%s=%s\n" % (key, value))
            handle.write("Mail__Enabled=true\nMail__FromAddress=noreply@chat.test\n")
            for key, value in provider_settings(providers).items():
                handle.write("%s=%s\n" % (key, value))
            if providers:
                handle.write("SSL_CERT_FILE=/tmp/ca.pem\n")
        out = subprocess.run(
            [ORACLE_SCRIPT, "start", ORACLE, str(ORACLE_PORT), "Production"],
            check=True, capture_output=True, text=True,
            env=dict(os.environ, ORACLE_ENV_FILE=env, ORACLE_PUBLIC_HOST="localhost"),
        ).stdout
        self.base = "http://127.0.0.1:%d" % ORACLE_PORT
        self.public = "http://localhost:%d" % ORACLE_PORT
        self.setup = re.search(r"Setup code: ([A-Z0-9-]+)\.", out).group(1)
        if providers:
            run("docker", "cp", os.path.join(WORK, "ca.pem"), CONTAINER + ":/tmp/ca.pem")

    def stop(self):
        run("docker", "stop", "-t", "10", CONTAINER)

    def start(self):
        run("docker", "start", CONTAINER)
        wait_ready(self.base)

    def edit(self, *statements):
        self.stop()
        copy = os.path.join(WORK, "oracle.db")
        run("docker", "cp", CONTAINER + ":/tmp/data/stellarr.db", copy)
        db = sqlite3.connect(copy)
        for statement in statements:
            db.executescript(statement) if isinstance(statement, str) else db.execute(*statement)
        db.commit()
        db.close()
        run("docker", "cp", copy, CONTAINER + ":/tmp/data/stellarr.db")
        self.start()

    def recover(self, username, reason="lost phone"):
        """What `stellarr security recover` does, by the first server's own
        command line, run on a copy of the database while the server is stopped."""
        cli = os.environ["STELLARR_CLI"]
        root = os.path.join(WORK, "oracle-cli")
        os.makedirs(os.path.join(root, "db"), exist_ok=True)
        self.stop()
        run("docker", "cp", CONTAINER + ":/tmp/data/stellarr.db", os.path.join(root, "db", "stellarr.db"))
        out = run(os.path.expanduser("~/.dotnet/dotnet"), cli, "--json", "security", "recover", username,
                  "--reason", reason, "--yes", env=dict(os.environ, STELLARR_DATA_ROOT=root))
        run("docker", "cp", os.path.join(root, "db", "stellarr.db"), CONTAINER + ":/tmp/data/stellarr.db")
        self.start()
        return json.loads(out)["data"]["recoveryCredential"]

    def database(self):
        self.stop()
        copy = os.path.join(WORK, "oracle-read.db")
        run("docker", "cp", CONTAINER + ":/tmp/data/stellarr.db", copy)
        self.start()
        return sqlite3.connect(copy)

    def close(self):
        subprocess.run([ORACLE_SCRIPT, "stop", ORACLE], capture_output=True)


class Rust(Server):
    name = "this server"
    smtp_host = "127.0.0.1"

    def __init__(self, limits_raised=True, providers=None):
        self.providers = providers
        self.folder = tempfile.mkdtemp(prefix="rust-", dir=WORK)
        self.limits = RAISED if limits_raised else {}
        self.process = None
        self.starts = 0
        self.base = "http://127.0.0.1:%d" % RUST_PORT
        self.public = "http://localhost:%d" % RUST_PORT
        self.start()
        self.setup = re.search(r"Setup code: ([A-Z0-9-]+)\.", open(os.path.join(self.folder, "log1.txt")).read()).group(1)

    def start(self):
        env = dict(
            os.environ,
            ASPNETCORE_ENVIRONMENT="Production",
            ASPNETCORE_URLS="http://127.0.0.1:%d" % RUST_PORT,
            Rtc__TurnSharedSecret="oracle-turn-secret-oracle-turn-secret-0123",
            STELLARR_DATA_ROOT=self.folder,
            Operations__PublicUrl=self.public,
            Mail__Enabled="true",
            Mail__FromAddress="noreply@chat.test",
            Testing__Workers="realtime-sessions",
            **self.limits,
            **provider_settings(self.providers),
        )
        if self.providers:
            env["SSL_CERT_FILE"] = os.path.join(WORK, "ca.pem")
        env["ConnectionStrings__Database"] = "Data Source=%s;Pooling=True" % os.path.join(self.folder, "stellarr.db")
        self.starts += 1
        path = os.path.join(self.folder, "log%d.txt" % self.starts)
        self.process = subprocess.Popen([API], env=env, stdout=open(path, "w"), stderr=subprocess.STDOUT)
        wait_ready(self.base)

    def stop(self):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            self.process.wait(timeout=40)

    def edit(self, *statements):
        self.stop()
        db = sqlite3.connect(os.path.join(self.folder, "stellarr.db"))
        for statement in statements:
            db.executescript(statement) if isinstance(statement, str) else db.execute(*statement)
        db.commit()
        db.close()
        self.start()

    def recover(self, username, reason="lost phone"):
        """What the same command does to the rows, written out: every method,
        remembered device, session, approved handoff, unused recovery code and
        earlier credential ends, the account moves to a new version and waits
        for recovery, and one credential is issued."""
        credential = base64.urlsafe_b64encode(os.urandom(32)).decode().rstrip("=")
        now = now_text()
        user = user_id(self, username)
        self.edit(
            ('UPDATE "MfaMethods" SET "RevokedAt" = ? WHERE "UserId" = ? AND "RevokedAt" IS NULL', (now, user)),
            ('UPDATE "TrustedDevices" SET "RevokedAt" = ? WHERE "UserId" = ? AND "RevokedAt" IS NULL', (now, user)),
            ('UPDATE "UserSessions" SET "RevokedAt" = ? WHERE "UserId" = ? AND "RevokedAt" IS NULL', (now, user)),
            ('UPDATE "DesktopBrowserLogins" SET "RedeemedAt" = ? WHERE "UserId" = ? AND "ApprovedAt" IS NOT NULL AND "RedeemedAt" IS NULL', (now, user)),
            ('UPDATE "RecoveryCodes" SET "UsedAt" = ? WHERE "UserId" = ? AND "UsedAt" IS NULL', (now, user)),
            ('UPDATE "CliRecoveryCredentials" SET "UsedAt" = ? WHERE "UserId" = ? AND "UsedAt" IS NULL', (now, user)),
            ('UPDATE "Users" SET "SessionVersion" = "SessionVersion" + 1, "SecurityRecoveryRequired" = 1 WHERE "Id" = ?', (user,)),
            ('INSERT INTO "SecurityEvents" ("Id","UserId","Kind","Detail","IpAddress","Device","CreatedAt") VALUES (?,?,?,?,?,?,?)',
             (new_id(), user, "cli-recovery", "operator recovery: " + reason, "", "", now)),
            ('INSERT INTO "CliRecoveryCredentials" ("Id","UserId","TokenHash","Reason","SecurityVersion","CreatedAt","ExpiresAt") '
             'SELECT ?, "Id", ?, ?, "SessionVersion", ?, ? FROM "Users" WHERE "Id" = ?',
             (new_id(), token_hash(credential), reason, now, now_text(600), user)),
        )
        return credential

    def database(self):
        copy = os.path.join(WORK, "rust-read.db")
        shutil.copy(os.path.join(self.folder, "stellarr.db"), copy)
        for suffix in ("-wal", "-shm"):
            source = os.path.join(self.folder, "stellarr.db" + suffix)
            if os.path.exists(source):
                shutil.copy(source, copy + suffix)
        return sqlite3.connect(copy)

    def close(self):
        self.stop()


# ---- a browser ----

class Answer:
    def __init__(self, status, headers, body):
        self.status, self.headers, self.body = status, headers, body

    def header(self, name):
        for key, value in self.headers:
            if key.lower() == name.lower():
                return value
        return None

    def json(self):
        return json.loads(self.body)


class Browser:
    """One browser: a jar of cookies, a name for itself, and an address."""

    def __init__(self, base, agent=AGENT, forwarded=None):
        self.base, self.agent, self.forwarded = base, agent, forwarded
        self.jar = {}
        self.host = base.split("//")[1]

    def send(self, method, path, body=None, headers=None, raw=None, content_type="application/json", cookies=True):
        connection = http.client.HTTPConnection(self.host, timeout=30)
        send = {"User-Agent": self.agent, "X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0"}
        if self.forwarded:
            send["X-Forwarded-For"] = self.forwarded
        if cookies and self.jar:
            send["Cookie"] = "; ".join("%s=%s" % pair for pair in self.jar.items())
        payload = raw if raw is not None else (None if body is None else json.dumps(body).encode())
        if payload is not None and content_type:
            send["Content-Type"] = content_type
        send.update(headers or {})
        connection.request(method, path, body=payload, headers=send)
        response = connection.getresponse()
        text = response.read().decode("utf-8", "replace")
        pairs = response.getheaders()
        connection.close()
        for key, value in pairs:
            if key.lower() == "set-cookie":
                name, _, rest = value.partition("=")
                cookie, _, attributes = rest.partition(";")
                if cookie == "" or "1970" in attributes:
                    self.jar.pop(name, None)
                else:
                    self.jar[name] = cookie
        return Answer(response.status, pairs, text)

    def get(self, path, **kwargs):
        return self.send("GET", path, **kwargs)

    def post(self, path, body=None, **kwargs):
        return self.send("POST", path, body, **kwargs)

    def cookie(self, name):
        return self.jar.get(name)


# ---- what is recorded, and made comparable ----

UUID = re.compile(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
COMPACT = re.compile(r"\b[0-9a-f]{32}\b")
MOMENT = re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?Z")
RECOVERY = re.compile(r"\b[A-Z0-9]{7}(?:-[A-Z0-9]{7}){3}\b")
SECRET = re.compile(r"\b[A-Z2-7]{32}\b")
TRACE = re.compile(r"00-[0-9a-f]{32}-[0-9a-f]{16}-00")
COLOUR = re.compile(r"oklch\([^)]*\)")
BASE64 = re.compile(r"[A-Za-z0-9_-]{40,}")
BINDING = re.compile(r"(\.AspNetCore\.(Correlation|OpenIdConnect\.Nonce)[^=]*|stellarr-oidc-[A-Za-z0-9_-]+)=")
SHOWN = ("cache-control", "pragma", "expires", "content-type", "retry-after", "location", "allow")


class Recording:
    def __init__(self, server):
        self.server = server
        self.lines = []
        self.ids = {}

    def id(self, match):
        key = match.group(0).lower().replace("-", "")
        self.ids.setdefault(key, len(self.ids) + 1)
        return "<id %d>" % self.ids[key]

    def scrub(self, text):
        text = TRACE.sub("<trace>", text)
        text = re.sub(r'"ipAddress":"[0-9a-f.:]+"', '"ipAddress":"<ip>"', text)
        text = UUID.sub(self.id, text)
        text = COMPACT.sub(self.id, text)
        text = MOMENT.sub("<time>", text)
        text = re.sub(r'"code":"\d{6}"', '"code":"<digits>"', text)
        text = re.sub(r'"token":"[A-Za-z0-9_-]{20,}"', '"token":"<token>"', text)
        text = re.sub(r'"credential":"\s*[A-Za-z0-9_-]{40,}\s*"', '"credential":"<credential>"', text)
        text = re.sub(r'"code":"[a-z0-9]{7}( [a-z0-9]{7}){3}"', '"code":"<typed recovery code>"', text)
        text = re.sub(r'"code":"[A-Z0-9]{7}(-[A-Z0-9]{7}){3}"', '"code":"<recovery code>"', text)
        text = RECOVERY.sub("<recovery code>", text)
        text = SECRET.sub("<secret>", text)
        text = COLOUR.sub("<colour>", text)
        text = text.replace(self.server.public, "<public>").replace(self.server.base, "<public>")
        text = re.sub(r"\b(state|nonce|ticket|code_challenge)=[^&\s\"]+", r"\1=<\1>", text)
        text = re.sub(r"\b(stellarr-dev-pending-identity|stellarr-oidc-[A-Za-z0-9_-]+)=[^;\s]+", r"\1=<value>", text)
        return text

    def location(self, value):
        """Where a redirect goes. The address of the provider's sign-in page
        keeps the parameters that mean something and only the names of the others."""
        if "state=" in value and "redirect_uri" in value:
            url = urllib.parse.urlparse(value)
            query = urllib.parse.parse_qs(url.query)
            keep = ["client_id", "response_type", "scope", "code_challenge_method", "response_mode", "redirect_uri", "max_age", "prompt"]
            shown = " ".join("%s=%s" % (name, query[name][0]) for name in keep if name in query)
            others = ",".join(sorted(name for name in query if name not in keep and not name.startswith("x-client")))
            value = "%s://%s%s %s +%s" % (url.scheme, url.netloc, url.path, shown, others)
        return self.scrub(re.sub(r"https://%s:%d" % (re.escape(PROVIDERS_HOST), PROVIDER_PORT), "<provider>", value)) if PROVIDERS_HOST else self.scrub(value)

    def cookie(self, line):
        parts = [part.strip() for part in line.split(";")]
        name, _, value = parts[0].partition("=")
        text = "%s=%s" % (name, "<value>" if value else "")
        for part in parts[1:]:
            if part.lower().startswith("expires="):
                if "1970" in part:
                    text += "; expires=1970"
                else:
                    seconds = time.mktime(time.strptime(part[8:], "%a, %d %b %Y %H:%M:%S GMT")) - time.timezone - time.time()
                    text += "; expires=in %d minutes" % (round(seconds / 60 / 5) * 5 if seconds > 600 else round(seconds / 60))
            else:
                text += "; " + part
        return text

    def note(self, label, text=""):
        self.lines.append("%s %s" % (label, self.scrub(text)) if text else label)

    def step(self, label, request, answer):
        self.lines.append("-- %s" % label)
        self.lines.append("   %s %s %s" % (request[0], self.scrub(request[1]), self.scrub(request[2]) if len(request) > 2 and request[2] else ""))
        self.lines.append("   => %d" % answer.status)
        for key, value in sorted(answer.headers):
            if key.lower() in SHOWN:
                shown = self.location(value) if key.lower() == "location" else value
                if key.lower() == "expires" and "1970" not in value:
                    shown = "<date>"
                self.lines.append("   %s: %s" % (key.lower(), shown))
        cookies = [self.cookie(value) for key, value in answer.headers if key.lower() == "set-cookie"]
        # The cookies that bind a round trip to its browser are named and counted
        # by each server in its own way (a known difference); they count as one.
        binding = [BINDING.sub("<binding>=", cookie) for cookie in cookies if BINDING.match(cookie)]
        cookies = [cookie for cookie in cookies if not BINDING.match(cookie)]
        for cookie in sorted(cookies + sorted(set(binding))):
            self.lines.append("   set-cookie: %s" % cookie)
        if len(cookies) > 1 and not binding:
            ORDERS.setdefault(label, {})[self.server.name] = [cookie.split("=")[0] for cookie in cookies]
        if answer.body:
            self.lines.append("   body: %s" % self.scrub(answer.body))
        return answer


def ask(rec, label, browser, method, path, body=None, **kwargs):
    answer = browser.send(method, path, body, **kwargs)
    shown = json.dumps(body, separators=(",", ":")) if body is not None else ""
    if kwargs.get("raw") is not None:
        shown = kwargs["raw"].decode("utf-8", "replace") if isinstance(kwargs["raw"], bytes) else kwargs["raw"]
    return rec.step(label, (method, path, shown), answer)


# ---- the mail server ----

def mailpit(method, path):
    request = urllib.request.Request("http://127.0.0.1:%d%s" % (MAIL_HTTP, path), method=method)
    with urllib.request.urlopen(request, timeout=20) as answer:
        body = answer.read()
    try:
        return json.loads(body)
    except ValueError:
        return body


def clear_mail():
    mailpit("DELETE", "/api/v1/messages")


def mails(count, wait=8.0):
    """The messages that arrived, oldest first, once `count` have."""
    deadline = time.time() + wait
    while True:
        found = mailpit("GET", "/api/v1/messages?limit=1000")["messages"]
        if len(found) >= count or time.time() > deadline:
            break
        time.sleep(0.2)
    found = sorted(found, key=lambda message: message["Created"])
    return [mailpit("GET", "/api/v1/message/" + message["ID"]) for message in found]


def code_of(message):
    found = re.search(r"\b(\d{6})\b", message["Text"])
    return found.group(1)


def link_token_of(message):
    return re.search(r"token=([A-Za-z0-9_-]+)", message["Text"]).group(1)


def describe_mail(rec, label, message):
    message = dict(message)
    for key in ("Text", "HTML"):
        message[key] = re.sub(r"(http:(//|&#x2F;&#x2F;))?(localhost|127\.0\.0\.1):\d+", "<public>", message[key])
    text = message["Text"]
    text = re.sub(r"token=[A-Za-z0-9_-]+", "token=<token>", text)
    text = re.sub(r"\b\d{6}\b", "<code>", text)
    text = re.sub(r"\d{1,2} [^\W\d_]+ \d{4}, \d{2}:\d{2}", "<moment>", text)
    html = re.sub(r"token=[A-Za-z0-9_-]+", "token=<token>", message["HTML"])
    html = re.sub(r"\b\d{6}\b", "<code>", html)
    html = re.sub(r"\d{1,2} [^\W\d_]+ \d{4}, \d{2}:\d{2}", "<moment>", html)
    html = re.sub(r"[0-9a-f]{32}", "<id>", html)
    rec.note("-- mail %s: subject %r from %r to %r" % (label, message["Subject"], message["From"], message["To"]))
    rec.note("   text: %s" % text.replace("\r", "").replace("\n", " | "))
    rec.note("   html: %d characters, %s" % (len(html), hashlib.sha256(re.sub(r"\s+", " ", html).encode()).hexdigest()[:12]))


# ---- the accounts the scenarios use ----

ACCOUNTS = ["totp", "email", "refuse", "budget", "recover", "reset", "keep", "disabled", "other", "empty", "shared", "limit",
            "ssopage", "ssokey", "ssoonly", "ssogone", "ssopolicy"]


def seed(server):
    """Founds the instance, and gives every scenario an account of its own and
    the instance the settings they all run under: a second step for everyone,
    and mail going to Mailpit."""
    owner = Browser(server.base)
    answer = owner.post("/api/auth/register", {
        "username": "owner", "displayName": "Owner", "email": "owner@example.test",
        "password": PASSWORD, "setupCode": server.setup,
    })
    assert answer.status == 201, (server.name, answer.status, answer.body)
    statements = []
    for index, name in enumerate(ACCOUNTS):
        statements.append((
            'INSERT INTO "Users" ("Id","Username","NormalizedUsername","DisplayName","PasswordHash","Email","NormalizedEmail",'
            '"IsDisabled","SessionVersion","SecurityRecoveryRequired","AvatarColor","CreatedAt") VALUES (?,?,?,?,?,?,?,0,1,0,?,?)',
            (new_id(), name, name.upper(), name.capitalize(), password_hash(PASSWORD), "%s@example.test" % name,
             "%s@EXAMPLE.TEST" % name.upper(), "oklch(0.58 0.14 %d)" % (index * 20), now_text()),
        ))
    statements.append((
        'INSERT OR REPLACE INTO "OperationsSettings" ("ServerId","PublicUrl","TimeZone","MailingEnabled","MailFromAddress",'
        '"MailSmtpHost","MailSmtpPort","MailSmtpUsername","MailSmtpSecurity","MailResendEnabled","InviteMailEnabled",'
        '"RecoveryMailEnabled","AlertMailEnabled","BackupRetentionDays","LogRetentionDays","OrphanObjectRetentionDays",'
        '"AutomaticCleanupEnabled","UpdatedAt","MfaRequirement") '
        'SELECT "Id","","UTC",1,"noreply@chat.test",?,?,"","none",0,1,1,0,14,30,1,1,?,"everyone" FROM "Servers" LIMIT 1',
        (server.smtp_host, SMTP_PORT, now_text()),
    ))
    server.edit(*statements)


def sql_one(server, query, *args):
    db = server.database()
    try:
        return db.execute(query, args).fetchall()
    finally:
        db.close()


def user_id(server, name):
    return sql_one(server, 'SELECT "Id" FROM "Users" WHERE "Username" = ?', name)[0][0]


# ---- a database as a conversation leaves it ----

def leave(rec, server, names):
    """What the conversations of the accounts left in the database."""
    db = server.database()
    try:
        for name in names:
            row = db.execute('SELECT "Id","SessionVersion","IsDisabled","SecurityRecoveryRequired" FROM "Users" WHERE "Username" = ?', (name,)).fetchone()
            if not row:
                continue
            user = row[0]
            rec.note("== left by %s: session version %s, disabled %s, recovery required %s" % (name, row[1], row[2], row[3]))
            for kind, detail, device, ip in db.execute(
                'SELECT "Kind","Detail","Device","IpAddress" FROM "SecurityEvents" WHERE "UserId" = ? ORDER BY "CreatedAt","Id"', (user,)
            ):
                rec.note("   event %s %r device=%r ip=%r" % (kind, detail, device[:20], "<ip>" if ip else ""))
            for row in db.execute(
                'SELECT "Purpose","Attempts","MaxAttempts","ConsumedAt" IS NOT NULL,"ShortLived","PrimarySource","WebAuthnOptions" IS NOT NULL,'
                '"PendingIdentityId" IS NOT NULL '
                'FROM "AuthChallenges" WHERE "UserId" = ? ORDER BY "CreatedAt","Id"', (user,)
            ):
                rec.note("   challenge %s attempts %s/%s consumed %s short %s primary %s options %s pending identity %s" % row)
            for row in db.execute(
                'SELECT "PrimarySource","MfaEvidence","MfaCompletedAt" IS NOT NULL,"ShortLived","RevokedAt" IS NOT NULL,"SecurityVersion",'
                'CASE WHEN "AuthenticatedAt" IS NULL THEN \'never\' WHEN ABS(julianday("AuthenticatedAt") - julianday("CreatedAt")) * 86400 < 5 THEN \'at sign-in\' '
                'WHEN "AuthenticatedAt" > "CreatedAt" THEN \'moved forward\' ELSE \'before\' END '
                'FROM "UserSessions" WHERE "UserId" = ? ORDER BY "CreatedAt","Id"', (user,)
            ):
                rec.note("   session primary %s evidence %s fresh %s short %s revoked %s version %s signed in %s" % row)
            for row in db.execute('SELECT "ProviderId","Issuer","Subject" FROM "ExternalIdentities" WHERE "UserId" = ? ORDER BY "Issuer","Subject"', (user,)):
                rec.note("   linked identity %s %s %s" % row)
            for row in db.execute(
                'SELECT "Type","Name","ConfirmedAt" IS NOT NULL,"RevokedAt" IS NOT NULL,"LastUsedAt" IS NOT NULL,"LastTotpStep" IS NOT NULL,"EmailAddress" '
                'FROM "MfaMethods" WHERE "UserId" = ? ORDER BY "CreatedAt","Id"', (user,)
            ):
                rec.note("   method %s %r confirmed %s revoked %s used %s step %s address %s" % row)
            for row in db.execute(
                'SELECT "Purpose","Attempts","MaxAttempts","ConsumedAt" IS NOT NULL,"DeliveredAt" IS NOT NULL '
                'FROM "EmailOtpCodes" WHERE "UserId" = ? ORDER BY "CreatedAt","Id"', (user,)
            ):
                rec.note("   mailed code %s attempts %s/%s consumed %s delivered %s" % row)
            used = db.execute('SELECT COUNT(*), SUM("UsedAt" IS NOT NULL) FROM "RecoveryCodes" WHERE "UserId" = ?', (user,)).fetchone()
            rec.note("   recovery codes %s, used %s" % used)
            for row in db.execute('SELECT "Name","RevokedAt" IS NOT NULL,"LastUsedAt" IS NOT NULL FROM "TrustedDevices" WHERE "UserId" = ? ORDER BY "CreatedAt","Id"', (user,)):
                rec.note("   remembered device %r revoked %s used %s" % row)
            for row in db.execute('SELECT "UsedAt" IS NOT NULL FROM "PasswordResetTokens" WHERE "UserId" = ? ORDER BY "CreatedAt","Id"', (user,)):
                rec.note("   reset link used %s" % row)
            for row in db.execute('SELECT "UsedAt" IS NOT NULL FROM "CliRecoveryCredentials" WHERE "UserId" = ? ORDER BY "CreatedAt","Id"', (user,)):
                rec.note("   recovery credential used %s" % row)
            for row in db.execute('SELECT e."Kind", n."EmailAddress" FROM "SecurityNotifications" n JOIN "SecurityEvents" e ON e."Id" = n."SecurityEventId" WHERE n."UserId" = ? ORDER BY e."CreatedAt", e."Id"', (user,)):
                rec.note("   alert queued for %s to %s" % row)
    finally:
        db.close()


def leave_identities(rec, server):
    """The identities that wait for a browser, as the conversations left them."""
    db = server.database()
    try:
        for row in db.execute(
            'SELECT "ProviderId","Subject","Email","EmailVerified","DisplayName","PreferredUsername","Amr","AuthTime" IS NOT NULL,"ShortLived",'
            'CAST(ROUND((julianday("ExpiresAt") - julianday("CreatedAt")) * 1440) AS INTEGER) '
            'FROM "PendingExternalIdentities" ORDER BY "Subject","Id"'
        ):
            rec.note("== waiting identity %s %s email %r verified %s name %r username %r amr %s auth time %s short %s lives %s minutes" % row)
        rec.note("== identities linked, all: %s" % [tuple(row) for row in db.execute(
            'SELECT u."Username", i."ProviderId", i."Subject" FROM "ExternalIdentities" i JOIN "Users" u ON u."Id" = i."UserId" ORDER BY 1, 2, 3')])
    finally:
        db.close()


# ---- signing in through the provider, as a browser does ----

PROVIDERS = {"default": ("p0", "/api/auth/oidc"), "trust": ("p1", "/api/auth/oidc/trust"),
             "plain": ("p2", "/api/auth/oidc/plain"), "work": ("p3", "/api/auth/oidc/work")}


def sso_start(rec, label, browser, path, prefix):
    if path is None:
        return None
    """Follows a route that sends the browser to the provider, and tells the
    provider the nonce it was given. Returns the state, or None if the answer
    was not a redirect to the provider."""
    answer = ask(rec, label, browser, "GET", path)
    where = answer.header("location") or ""
    if answer.status != 302 or not where.startswith("https://"):
        return None
    query = urllib.parse.parse_qs(urllib.parse.urlparse(where).query)
    provider(prefix, nonce=query["nonce"][0])
    return query["state"][0]


def sso_back(rec, label, browser, state, route):
    if state is None:
        return None
    return ask(rec, label, browser, "GET", "%s?code=the-code&state=%s" % (route, urllib.parse.quote(state, safe="")))


def sso_in(rec, label, browser, name="default", return_url="/", route=None, extra=""):
    """A whole sign-in at the provider `name`: out and back. `route` is the
    address that sends the browser out when it is not the sign-in's own."""
    prefix, base = PROVIDERS[name]
    start = route or "%s/login?returnUrl=%s%s" % (base, urllib.parse.quote(return_url, safe=""), extra)
    state = sso_start(rec, label + ", out", browser, start, prefix)
    return sso_back(rec, label + ", back", browser, state, base + "/callback") if state else None


def sso_ticket(rec, label, browser, name, what, body=None):
    """Asks the security page for a ticket (`link` or `reauthenticate`) and returns the address in it."""
    answer = ask(rec, label, browser, "POST", "/api/me/security/identities/%s/%s" % (name, what), {} if body is None else body)
    return answer.json().get("url") if answer.status == 200 else None


def sso_page(rec, label, browser, what, name="default", back=None, subject_ok=True):
    """The whole of a link or a re-authentication started from the security
    page. Returns the answer that came back from the provider."""
    url = sso_ticket(rec, label + ", ticket", browser, name, what)
    if url is None:
        return None
    if back:
        url += "&returnUrl=" + urllib.parse.quote(back, safe="")
    return sso_in(rec, label, browser, name, route=url)


def sso_login(rec, label, browser, name, **extra):
    return ask(rec, label, browser, "POST", "/api/auth/login", dict({"username": name, "password": PASSWORD}, **extra))


# ---- the scenarios ----

SCENARIOS = {}
ORDERS = {}


def scenario(function):
    SCENARIOS[function.__name__] = function
    return function


def login(rec, label, browser, name, **extra):
    return ask(rec, label, browser, "POST", "/api/auth/login", dict({"username": name, "password": PASSWORD}, **extra))


def enrol_totp(rec, server, name, label="", remember=False, extra=None):
    """Signs in as an account with no second step, sets an authenticator app up
    and is signed in by it. Returns the browser, the secret and the recovery
    codes."""
    browser = Browser(server.base)
    answer = login(rec, label + "sign in, the instance asks for a second step", browser, name)
    challenge = answer.json()["challengeId"]
    started = ask(rec, label + "start an authenticator app", browser, "POST", "/api/auth/mfa/enroll/totp/start",
                  {"challengeId": challenge, "name": "Phone"})
    secret, method = started.json()["secret"], started.json()["methodId"]
    body = {"challengeId": challenge, "methodId": method, "code": totp_now(secret), "name": "Phone", "rememberDevice": remember}
    done = ask(rec, label + "confirm it with its first code", browser, "POST", "/api/auth/mfa/enroll/totp/confirm", body)
    return browser, secret, method, done.json().get("recoveryCodes"), challenge


@scenario
def totp(server, rec):
    """An authenticator app: forced enrolment, a sign-in finished with its code,
    a remembered browser, a recovery code."""
    browser = Browser(server.base)
    ask(rec, "no challenge yet", browser, "GET", "/api/auth/mfa/pending")
    answer = login(rec, "sign in with the password", browser, "totp")
    challenge = answer.json()["challengeId"]
    ask(rec, "the challenge a reload would resume", browser, "GET", "/api/auth/mfa/pending")
    ask(rec, "the app named too long", browser, "POST", "/api/auth/mfa/enroll/totp/start", {"challengeId": challenge, "name": "x" * 65})
    ask(rec, "the app, named by nobody", browser, "POST", "/api/auth/mfa/enroll/totp/start", {"challengeId": challenge})
    started = ask(rec, "the app, named", browser, "POST", "/api/auth/mfa/enroll/totp/start", {"challengeId": challenge, "name": "Phone"})
    secret, method = started.json()["secret"], started.json()["methodId"]
    ask(rec, "a wrong first code", browser, "POST", "/api/auth/mfa/enroll/totp/confirm",
        {"challengeId": challenge, "methodId": method, "code": "000000", "name": "Phone"})
    ask(rec, "the code, with a name that is too long", browser, "POST", "/api/auth/mfa/enroll/totp/confirm",
        {"challengeId": challenge, "methodId": method, "code": totp_now(secret), "name": "x" * 65})
    ask(rec, "a method that is not there", browser, "POST", "/api/auth/mfa/enroll/totp/confirm",
        {"challengeId": challenge, "methodId": new_id(), "code": totp_now(secret)})
    ask(rec, "a method id that is not an id", browser, "POST", "/api/auth/mfa/enroll/totp/confirm",
        {"challengeId": challenge, "methodId": "nope", "code": totp_now(secret)})
    done = ask(rec, "the first code, and the browser is remembered", browser, "POST", "/api/auth/mfa/enroll/totp/confirm",
               {"challengeId": challenge, "methodId": method, "code": totp_now(secret), "name": "Phone", "rememberDevice": True})
    codes = done.json()["recoveryCodes"]
    ask(rec, "the challenge is spent", browser, "GET", "/api/auth/mfa/pending")
    ask(rec, "the same code again", browser, "POST", "/api/auth/mfa/enroll/totp/confirm",
        {"challengeId": challenge, "methodId": method, "code": totp_now(secret)})
    ask(rec, "who is signed in", browser, "GET", "/api/session")
    ask(rec, "sign out", browser, "POST", "/api/auth/logout")

    # The remembered browser keeps its cookie; a sign-in finds it.
    answer = login(rec, "sign in again from the remembered browser", browser, "totp")
    challenge = answer.json()["challengeId"]
    ask(rec, "use the remembered browser", browser, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "useTrustedDevice": True})
    ask(rec, "who is signed in, by a remembered browser", browser, "GET", "/api/session")
    ask(rec, "sign out again", browser, "POST", "/api/auth/logout")

    other = Browser(server.base)
    answer = login(rec, "a browser that is not remembered", other, "totp")
    challenge = answer.json()["challengeId"]
    ask(rec, "the remembered browser asked for by one that is not", other, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "useTrustedDevice": True})
    ask(rec, "no code", other, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "methodId": method})
    ask(rec, "a code of spaces", other, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "methodId": method, "code": "   "})
    ask(rec, "a method id that is not an id, so a recovery code", other, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "methodId": "nope", "code": "123456"})
    ask(rec, "a method that is not the account's", other, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "methodId": new_id(), "code": totp_now(secret)})
    ask(rec, "a wrong code", other, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "methodId": method, "code": "000000"})
    answered = ask(rec, "the right code, and asked to be remembered", other, "POST", "/api/auth/mfa/verify",
                   {"challengeId": challenge, "methodId": method, "code": totp_now(secret, 1), "rememberDevice": True})
    ask(rec, "the code again, on a spent challenge", other, "POST", "/api/auth/mfa/verify",
        {"challengeId": challenge, "methodId": method, "code": totp_now(secret, 1)})
    ask(rec, "sign out the new browser", other, "POST", "/api/auth/logout")

    # A remembered browser asked to be remembered again is not.
    answer = login(rec, "sign in from the browser remembered a moment ago", other, "totp")
    challenge = answer.json()["challengeId"]
    ask(rec, "remembered, and asked to be again", other, "POST", "/api/auth/mfa/verify",
        {"challengeId": challenge, "useTrustedDevice": True, "rememberDevice": True})
    ask(rec, "sign out the remembered one", other, "POST", "/api/auth/logout")

    # A recovery code stands in for the app, typed as people type them.
    third = Browser(server.base)
    answer = login(rec, "sign in without the app", third, "totp")
    challenge = answer.json()["challengeId"]
    typed = codes[0].lower().replace("-", " ")
    ask(rec, "a recovery code, typed in lower case with spaces", third, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "code": typed})
    ask(rec, "sign out", third, "POST", "/api/auth/logout")
    answer = login(rec, "sign in once more", third, "totp")
    challenge = answer.json()["challengeId"]
    ask(rec, "the same recovery code again", third, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "code": codes[0]})
    ask(rec, "another, and the challenge is cancelled first", third, "POST", "/api/auth/mfa/cancel")
    ask(rec, "the cancelled challenge answered", third, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "code": codes[1]})
    ask(rec, "cancelling again", third, "POST", "/api/auth/mfa/cancel")
    leave(rec, server, ["totp"])


@scenario
def email(server, rec):
    """The account's address as the second step: a forced enrolment, a mailed
    code to sign in with, the cooldown and the hourly limit."""
    clear_mail()
    browser = Browser(server.base)
    challenge = login(rec, "sign in with the password", browser, "email").json()["challengeId"]
    ask(rec, "a code mailed to prove the address", browser, "POST", "/api/auth/mfa/enroll/email/start", {"challengeId": challenge})
    ask(rec, "asked again at once", browser, "POST", "/api/auth/mfa/enroll/email/start", {"challengeId": challenge})
    sent = mails(1)
    describe_mail(rec, "enrolment", sent[0])
    ask(rec, "a wrong code", browser, "POST", "/api/auth/mfa/enroll/email/confirm", {"challengeId": challenge, "code": "000000"})
    ask(rec, "the right code with a name that is too long", browser, "POST", "/api/auth/mfa/enroll/email/confirm",
        {"challengeId": challenge, "code": code_of(sent[0]), "name": "x" * 65})
    ask(rec, "the right code", browser, "POST", "/api/auth/mfa/enroll/email/confirm",
        {"challengeId": challenge, "code": code_of(sent[0]), "name": "Mail", "rememberDevice": True})
    ask(rec, "sign out", browser, "POST", "/api/auth/logout")
    clear_mail()
    other = Browser(server.base)
    answer = login(rec, "sign in again", other, "email")
    challenge = answer.json()["challengeId"]
    method = answer.json()["methods"][0]["id"]
    ask(rec, "a mailed code without a method", other, "POST", "/api/auth/mfa/email/send", {"challengeId": challenge})
    ask(rec, "a mailed code for a method that is not an id", other, "POST", "/api/auth/mfa/email/send", {"challengeId": challenge, "methodId": "nope"})
    ask(rec, "a mailed code for a method that is not the account's", other, "POST", "/api/auth/mfa/email/send", {"challengeId": challenge, "methodId": new_id()})
    ask(rec, "a mailed code", other, "POST", "/api/auth/mfa/email/send", {"challengeId": challenge, "methodId": method})
    ask(rec, "asked for too soon", other, "POST", "/api/auth/mfa/email/send", {"challengeId": challenge, "methodId": method})
    sent = mails(1)
    describe_mail(rec, "sign in", sent[0])
    ask(rec, "the wrong code", other, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "methodId": method, "code": "000000"})
    ask(rec, "the right code", other, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "methodId": method, "code": code_of(sent[0])})
    ask(rec, "the same code, on a spent challenge", other, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "methodId": method, "code": code_of(sent[0])})
    leave(rec, server, ["email"])


@scenario
def limit(server, rec):
    """The hourly limit of mailed codes, with the cooldown taken off."""
    clear_mail()
    server.edit('UPDATE "OperationsSettings" SET "EmailCodeCooldownSeconds" = 0')
    browser = Browser(server.base)
    challenge = login(rec, "sign in", browser, "limit").json()["challengeId"]
    for number in range(1, 13):
        ask(rec, "mailed code %d" % number, browser, "POST", "/api/auth/mfa/enroll/email/start", {"challengeId": challenge})
    rec.note("mails that arrived: %d" % len(mails(10, wait=10)))
    server.edit('UPDATE "OperationsSettings" SET "EmailCodeCooldownSeconds" = 45')
    leave(rec, server, ["limit"])


@scenario
def refuse(server, rec):
    """Every way a challenge can be refused."""
    # One account that has an app, and the challenges it will be refused with.
    browser, secret, method, codes, _ = enrol_totp(rec, server, "refuse")
    ask(rec, "sign out", browser, "POST", "/api/auth/logout")
    mine = Browser(server.base)
    expired = login(rec, "a sign-in that will run out", mine, "refuse").json()["challengeId"]
    expired_jar = dict(mine.jar)
    mine2 = Browser(server.base)
    spent = login(rec, "a sign-in that will be spent", mine2, "refuse").json()["challengeId"]
    spent_jar = dict(mine2.jar)
    mine3 = Browser(server.base)
    changed = login(rec, "a sign-in the account changes under", mine3, "refuse").json()["challengeId"]
    changed_jar = dict(mine3.jar)
    server.edit(
        ('UPDATE "AuthChallenges" SET "ExpiresAt" = ? WHERE "Id" = ?', (now_text(-60), expired.upper())),
        ('UPDATE "AuthChallenges" SET "ConsumedAt" = ? WHERE "Id" = ?', (now_text(-60), spent.upper())),
    )
    mine.jar = expired_jar
    ask(rec, "the challenge that ran out, asked about", mine, "GET", "/api/auth/mfa/pending")
    ask(rec, "the challenge that ran out, answered", mine, "POST", "/api/auth/mfa/verify", {"challengeId": expired, "methodId": method, "code": totp_now(secret)})
    mine2.jar = spent_jar
    ask(rec, "the spent challenge, answered", mine2, "POST", "/api/auth/mfa/verify", {"challengeId": spent, "methodId": method, "code": totp_now(secret)})
    ask(rec, "a mailed code for a spent challenge", mine2, "POST", "/api/auth/mfa/email/send", {"challengeId": spent, "methodId": method})
    ask(rec, "passkey options for a spent challenge", mine2, "POST", "/api/auth/mfa/passkey/options", {"challengeId": spent})

    nobody = Browser(server.base)
    ask(rec, "no cookie at all", nobody, "POST", "/api/auth/mfa/verify", {"challengeId": changed, "methodId": method, "code": totp_now(secret)})
    ask(rec, "no cookie, asked about", nobody, "GET", "/api/auth/mfa/pending")
    stranger = Browser(server.base)
    mine3.jar = changed_jar
    stranger.jar = {name: value for name, value in changed_jar.items()}
    other_challenge = login(rec, "another browser's own sign-in", stranger, "refuse").json()["challengeId"]
    ask(rec, "another browser's cookie with this challenge's id", stranger, "POST", "/api/auth/mfa/verify", {"challengeId": changed, "methodId": method, "code": totp_now(secret)})
    wrong = Browser(server.base)
    for name, value in changed_jar.items():
        wrong.jar[name] = value[:-3] + "xyz"
    ask(rec, "a cookie with a wrong secret", wrong, "POST", "/api/auth/mfa/verify", {"challengeId": changed, "methodId": method, "code": totp_now(secret)})
    ask(rec, "a challenge id that is not an id", mine3, "POST", "/api/auth/mfa/verify", {"challengeId": "nope", "methodId": method, "code": totp_now(secret)})
    ask(rec, "a challenge id with no dashes", mine3, "POST", "/api/auth/mfa/verify", {"challengeId": changed.replace("-", ""), "methodId": method, "code": "000000"})
    ask(rec, "no challenge id", mine3, "POST", "/api/auth/mfa/verify", {"methodId": method, "code": totp_now(secret)})
    ask(rec, "an empty body", mine3, "POST", "/api/auth/mfa/verify", {})
    ask(rec, "a body that is not JSON", mine3, "POST", "/api/auth/mfa/verify", raw=b"{nope")
    ask(rec, "a body of another type", mine3, "POST", "/api/auth/mfa/verify", raw=b"x=1", content_type="application/x-www-form-urlencoded")
    ask(rec, "no body and no type", mine3, "POST", "/api/auth/mfa/verify", content_type=None)
    ask(rec, "a challenge of enrolment asked to start an app: the wrong purpose", mine3, "POST", "/api/auth/mfa/enroll/totp/start", {"challengeId": changed})
    ask(rec, "the challenge of a sign-in asked for an email enrolment", mine3, "POST", "/api/auth/mfa/enroll/email/start", {"challengeId": changed})
    ask(rec, "the options of a passkey for an account with none", mine3, "POST", "/api/auth/mfa/passkey/options", {"challengeId": changed})
    ask(rec, "a passkey answer with nothing in it", mine3, "POST", "/api/auth/mfa/passkey/verify", {"challengeId": changed})
    # Five wrong answers and the right one after.
    for index in range(5):
        ask(rec, "wrong code %d" % (index + 1), mine3, "POST", "/api/auth/mfa/verify", {"challengeId": changed, "methodId": method, "code": "000000"})
    ask(rec, "the right code after five wrong ones", mine3, "POST", "/api/auth/mfa/verify", {"challengeId": changed, "methodId": method, "code": totp_now(secret)})
    ask(rec, "the challenge, asked about", mine3, "GET", "/api/auth/mfa/pending")

    # The account changes between the two steps.
    one = Browser(server.base)
    disabled_challenge = login(rec, "a sign-in the account is disabled under", one, "refuse").json()["challengeId"]
    server.edit(('UPDATE "Users" SET "IsDisabled" = 1 WHERE "Username" = ?', ("refuse",)))
    ask(rec, "the code of a disabled account", one, "POST", "/api/auth/mfa/verify", {"challengeId": disabled_challenge, "methodId": method, "code": totp_now(secret, 1)})
    ask(rec, "the disabled account's challenge, asked about", one, "GET", "/api/auth/mfa/pending")
    server.edit(('UPDATE "Users" SET "IsDisabled" = 0 WHERE "Username" = ?', ("refuse",)))
    two = Browser(server.base)
    changed_challenge = login(rec, "a sign-in the password is changed under", two, "refuse").json()["challengeId"]
    server.edit(('UPDATE "Users" SET "SessionVersion" = "SessionVersion" + 1 WHERE "Username" = ?', ("refuse",)))
    ask(rec, "the code after the password changed", two, "POST", "/api/auth/mfa/verify", {"challengeId": changed_challenge, "methodId": method, "code": totp_now(secret, 1)})
    leave(rec, server, ["refuse"])


@scenario
def budget(server, rec):
    """The account-wide budget of failed proofs across challenges."""
    browser, secret, method, codes, _ = enrol_totp(rec, server, "budget")
    ask(rec, "sign out", browser, "POST", "/api/auth/logout")
    for round_ in range(2):
        mine = Browser(server.base)
        challenge = login(rec, "sign-in %d" % (round_ + 1), mine, "budget").json()["challengeId"]
        for index in range(5):
            ask(rec, "round %d wrong code %d" % (round_ + 1, index + 1), mine, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "methodId": method, "code": "000000"})
    mine = Browser(server.base)
    challenge = login(rec, "a third sign-in", mine, "budget").json()["challengeId"]
    ask(rec, "the right code, with the budget spent", mine, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "methodId": method, "code": totp_now(secret, 1)})
    ask(rec, "a recovery code, with the budget spent", mine, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "code": codes[0]})
    ask(rec, "the remembered browser, with the budget spent", mine, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "useTrustedDevice": True})
    ask(rec, "the challenge, asked about", mine, "GET", "/api/auth/mfa/pending")
    leave(rec, server, ["budget"])


@scenario
def keep(server, rec):
    """A sign-in that asks not to be kept: the second step finishes the short session it asked for."""
    browser = Browser(server.base)
    challenge = login(rec, "sign in, not kept", browser, "keep", keepSignedIn=False).json()["challengeId"]
    started = ask(rec, "start an app", browser, "POST", "/api/auth/mfa/enroll/totp/start", {"challengeId": challenge})
    secret, method = started.json()["secret"], started.json()["methodId"]
    ask(rec, "confirm it", browser, "POST", "/api/auth/mfa/enroll/totp/confirm",
        {"challengeId": challenge, "methodId": method, "code": totp_now(secret), "rememberDevice": True})
    ask(rec, "sign out", browser, "POST", "/api/auth/logout")
    other = Browser(server.base)
    challenge = login(rec, "sign in, kept", other, "keep", keepSignedIn=True).json()["challengeId"]
    ask(rec, "the code", other, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "methodId": method, "code": totp_now(secret, 1)})
    ask(rec, "sign out", other, "POST", "/api/auth/logout")
    third = Browser(server.base)
    challenge = login(rec, "sign in, not kept, by a remembered browser", third, "keep", keepSignedIn=False).json()["challengeId"]
    third.jar.update({name: value for name, value in browser.jar.items() if "trust" in name})
    ask(rec, "a remembered browser", third, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "useTrustedDevice": True})
    leave(rec, server, ["keep"])


@scenario
def reset(server, rec):
    """Forgot and reset: the same answer for every address, one link, every session ended."""
    clear_mail()
    # A signed-in account with a remembered browser, whose sessions the reset ends.
    browser, secret, method, codes, _ = enrol_totp(rec, server, "reset", remember=True)
    ask(rec, "signed in before the reset", browser, "GET", "/api/session")
    for label, address in [("an address that is not an account's", "nobody@example.test"), ("no address", None), ("an address with spaces and capitals", "  RESET@example.test ")]:
        ask(rec, "forgot: " + label, Browser(server.base), "POST", "/api/auth/password/forgot", {"email": address} if address is not None else {})
    ask(rec, "forgot: a body that is not JSON", Browser(server.base), "POST", "/api/auth/password/forgot", raw=b"{nope")
    ask(rec, "forgot: a body of another type", Browser(server.base), "POST", "/api/auth/password/forgot", raw=b"x=1", content_type="text/plain")
    first = mails(1)
    describe_mail(rec, "first link", first[0])
    rec.note("mails after the first three requests: %d" % len(mails(1)))
    ask(rec, "forgot again: a second link", Browser(server.base), "POST", "/api/auth/password/forgot", {"email": "reset@example.test"})
    both = mails(2)
    describe_mail(rec, "second link", both[1])
    old, new = link_token_of(both[0]), link_token_of(both[1])
    anon = Browser(server.base)
    ask(rec, "reset: no token, a short password", anon, "POST", "/api/auth/password/reset", {"newPassword": "short"})
    ask(rec, "reset: a password of 257 characters", anon, "POST", "/api/auth/password/reset", {"token": new, "newPassword": "p" * 257})
    ask(rec, "reset: nothing", anon, "POST", "/api/auth/password/reset", {})
    ask(rec, "reset: a token that is nothing", anon, "POST", "/api/auth/password/reset", {"token": "nope", "newPassword": PASSWORD + "2"})
    ask(rec, "reset: the older link, retired by the newer", anon, "POST", "/api/auth/password/reset", {"token": old, "newPassword": PASSWORD + "2"})
    ask(rec, "reset: the newer link, a password the policy refuses", anon, "POST", "/api/auth/password/reset", {"token": new, "newPassword": "short"})
    ask(rec, "reset: the newer link", anon, "POST", "/api/auth/password/reset", {"token": new, "newPassword": PASSWORD + "2"})
    ask(rec, "reset: the same link again", anon, "POST", "/api/auth/password/reset", {"token": new, "newPassword": PASSWORD + "3"})
    ask(rec, "the session from before the reset", browser, "GET", "/api/session")
    ask(rec, "the old password", Browser(server.base), "POST", "/api/auth/login", {"username": "reset", "password": PASSWORD})
    answer = ask(rec, "the new password", Browser(server.base), "POST", "/api/auth/login", {"username": "reset", "password": PASSWORD + "2"})
    ask(rec, "the remembered browser, after the reset", browser, "POST", "/api/auth/login", {"username": "reset", "password": PASSWORD + "2"})
    leave(rec, server, ["reset"])


@scenario
def reset_refused(server, rec):
    """A link that ran out, one of a disabled account, and mail that is off."""
    clear_mail()
    anon = Browser(server.base)
    ask(rec, "forgot", anon, "POST", "/api/auth/password/forgot", {"email": "disabled@example.test"})
    expired = link_token_of(mails(1)[0])
    ask(rec, "forgot, for another account", anon, "POST", "/api/auth/password/forgot", {"email": "other@example.test"})
    second = link_token_of(mails(2)[1])
    server.edit(
        ('UPDATE "PasswordResetTokens" SET "ExpiresAt" = ? WHERE "TokenHash" = ?', (now_text(-5), token_hash(expired))),
    )
    ask(rec, "reset: a link that ran out", anon, "POST", "/api/auth/password/reset", {"token": expired, "newPassword": PASSWORD + "2"})
    server.edit(('UPDATE "Users" SET "IsDisabled" = 1 WHERE "Username" = ?', ("other",)))
    ask(rec, "reset: a link of a disabled account", anon, "POST", "/api/auth/password/reset", {"token": second, "newPassword": PASSWORD + "2"})
    clear_mail()
    ask(rec, "forgot: a disabled account", anon, "POST", "/api/auth/password/forgot", {"email": "other@example.test"})
    rec.note("mails for a disabled account: %d" % len(mails(0, wait=1.5)))
    server.edit('UPDATE "OperationsSettings" SET "RecoveryMailEnabled" = 0')
    ask(rec, "forgot: recovery mail switched off", anon, "POST", "/api/auth/password/forgot", {"email": "empty@example.test"})
    rec.note("mails with recovery mail off: %d" % len(mails(0, wait=1.5)))
    server.edit('UPDATE "OperationsSettings" SET "RecoveryMailEnabled" = 1, "MailingEnabled" = 0')
    ask(rec, "forgot: mail switched off", anon, "POST", "/api/auth/password/forgot", {"email": "empty@example.test"})
    rec.note("mails with mail off: %d" % len(mails(0, wait=1.5)))
    ask(rec, "a mailed code with mail off", anon, "POST", "/api/auth/mfa/email/send", {"challengeId": new_id(), "methodId": new_id()})
    server.edit('UPDATE "OperationsSettings" SET "MailingEnabled" = 1')
    leave(rec, server, ["disabled", "other", "empty"])




@scenario
def recover(server, rec):
    """An operator's recovery: the credential the command line issued is
    exchanged for the challenge that sets a new second step up."""
    browser, secret, method, codes, _ = enrol_totp(rec, server, "recover")
    ask(rec, "signed in before the recovery", browser, "GET", "/api/session")
    credential = server.recover("recover")
    ask(rec, "the session, after the recovery", browser, "GET", "/api/session")
    anon = Browser(server.base)
    ask(rec, "the password of an account waiting for recovery", anon, "POST", "/api/auth/login", {"username": "recover", "password": PASSWORD})
    ask(rec, "redeem: nothing", anon, "POST", "/api/auth/recovery/redeem", {})
    ask(rec, "redeem: too short", anon, "POST", "/api/auth/recovery/redeem", {"credential": "a" * 15})
    ask(rec, "redeem: too long", anon, "POST", "/api/auth/recovery/redeem", {"credential": "a" * 513})
    ask(rec, "redeem: a credential that is nothing", anon, "POST", "/api/auth/recovery/redeem", {"credential": "a" * 43})
    ask(rec, "redeem: the credential, with spaces round it, not to be kept", anon, "POST", "/api/auth/recovery/redeem",
        {"credential": "  %s  " % credential, "keepSignedIn": False})
    ask(rec, "the challenge a reload would resume", anon, "GET", "/api/auth/mfa/pending")
    ask(rec, "redeem: the credential again", Browser(server.base), "POST", "/api/auth/recovery/redeem", {"credential": credential})
    challenge = anon.cookie("stellarr-dev-preauth").split(".")[0]
    challenge = "%s-%s-%s-%s-%s" % (challenge[:8], challenge[8:12], challenge[12:16], challenge[16:20], challenge[20:])
    ask(rec, "a code, for an enrolment", anon, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "code": "123456"})
    started = ask(rec, "start a new app", anon, "POST", "/api/auth/mfa/enroll/totp/start", {"challengeId": challenge, "name": "New phone"})
    secret, method = started.json()["secret"], started.json()["methodId"]
    ask(rec, "confirm it", anon, "POST", "/api/auth/mfa/enroll/totp/confirm",
        {"challengeId": challenge, "methodId": method, "code": totp_now(secret), "name": "New phone", "rememberDevice": True})
    ask(rec, "the old password still signs in", Browser(server.base), "POST", "/api/auth/login", {"username": "recover", "password": PASSWORD})
    leave(rec, server, ["recover"])


@scenario
def link(server, rec):
    """A password sign-in that asked to link the identity a provider confirmed: no provider is
    configured on either server, so the link comes out as failed and the identity is used up."""
    browser, secret, method, codes, _ = enrol_totp(rec, server, "shared", remember=True)
    ask(rec, "sign out", browser, "POST", "/api/auth/logout")
    user = user_id(server, "shared")
    cases = {}
    for name in ("taken", "no cookie", "replaced", "device"):
        mine = Browser(server.base)
        if name == "device":
            mine.jar.update({key: value for key, value in browser.jar.items() if "trust" in key})
        cases[name] = (mine, login(rec, "a sign-in that asks to link (%s)" % name, mine, "shared").json()["challengeId"])
    pending = {name: (new_id(), "S" + name.replace(" ", "x") * 12) for name in cases}
    statements = []
    for name, (identity, secret_text) in pending.items():
        statements.append((
            'INSERT INTO "PendingExternalIdentities" ("Id","SecretHash","ProviderId","Issuer","Subject","Email","EmailVerified","DisplayName",'
            '"PreferredUsername","Amr","AuthTime","ShortLived","CreatedAt","ExpiresAt") VALUES (?,?,?,?,?,NULL,0,NULL,NULL,?,NULL,0,?,?)',
            (identity, token_hash(secret_text), "default", "https://issuer.test", "subject-" + name, "[]", now_text(), now_text(600)),
        ))
        statements.append(('UPDATE "AuthChallenges" SET "PendingIdentityId" = ? WHERE "Id" = ?',
                           (identity if name != "replaced" else new_id(), cases[name][1].upper())))
    server.edit(*statements)
    for name, (mine, challenge) in cases.items():
        if name != "no cookie":
            mine.jar["stellarr-dev-pending-identity"] = pending[name][0].lower().replace("-", "") + "." + pending[name][1]
    first = cases["taken"]
    ask(rec, "the challenge of a sign-in that asks to link, asked about", first[0], "GET", "/api/auth/mfa/pending")
    ask(rec, "the code, linking", first[0], "POST", "/api/auth/mfa/verify", {"challengeId": first[1], "code": codes[1]})
    nocookie = cases["no cookie"]
    ask(rec, "the code, with no cookie of the waiting identity", nocookie[0], "POST", "/api/auth/mfa/verify", {"challengeId": nocookie[1], "code": codes[2]})
    replaced = cases["replaced"]
    ask(rec, "the code, the identity replaced since", replaced[0], "POST", "/api/auth/mfa/verify", {"challengeId": replaced[1], "code": codes[3]})
    device = cases["device"]
    ask(rec, "a remembered browser does not stand in for a link", device[0], "POST", "/api/auth/mfa/verify", {"challengeId": device[1], "useTrustedDevice": True})
    rows = sql_one(server, 'SELECT "SecretHash" IS NOT NULL, "Subject" FROM "PendingExternalIdentities" ORDER BY "Subject"')
    rec.note("identities still waiting: %s" % [row[1] for row in rows])
    rec.note("identities linked: %s" % sql_one(server, 'SELECT COUNT(*) FROM "ExternalIdentities"')[0][0])
    if PROVIDERS_HOST:
        sso_link(server, rec, user_id(server, "other"), codes)
    leave(rec, server, ["shared"])
    leave_identities(rec, server)


def sso_link(server, rec, other, codes):
    """The same sign-in, with a provider that is set up: the identity that
    waits in the browser is linked once the second step is passed, or the link
    fails and the sign-in stands. `codes` are the recovery codes of `shared`, of
    which the first four are spent by the cases above."""
    issuer = "https://%s:%d/p2" % (PROVIDERS_HOST, PROVIDER_PORT)
    claims = {"email": "ola@example.test", "email_verified": True, "name": "Ola Nowak"}
    spare = iter(codes[4:])

    def waiting(label, subject, browser, name="default", **more):
        provider_reset(PROVIDERS[name][0], subject=subject, claims=claims, **more)
        return sso_in(rec, label, browser, name)

    def asks(label, browser):
        return sso_login(rec, label, browser, "shared", linkPending=True).json()["challengeId"]

    def pass_it(label, browser, challenge, code=None):
        return ask(rec, label, browser, "POST", "/api/auth/mfa/verify", {"challengeId": challenge, "code": code or next(spare)})

    rec.note("== the identity of a provider waits in the browser, and the sign-in that asked to link passes its second step")
    mine = Browser(server.base)
    waiting("an identity nobody has arrives", "s-linked", mine)
    ask(rec, "what waits", mine, "GET", "/api/auth/oidc/pending")
    challenge = asks("the password asks to link", mine)
    pass_it("a wrong code", mine, challenge, "000000")
    pass_it("the right one links", mine, challenge)
    ask(rec, "who is signed in", mine, "GET", "/api/session")
    ask(rec, "what waits now", mine, "GET", "/api/auth/oidc/pending")
    ask(rec, "sign out", mine, "POST", "/api/auth/logout")
    provider_reset("p0", subject="s-linked", claims=claims)
    again = Browser(server.base)
    sso_in(rec, "the linked identity signs in", again)
    ask(rec, "who", again, "GET", "/api/session")
    ask(rec, "sign out", again, "POST", "/api/auth/logout")

    rec.note("== the identity was linked to somebody else in the meantime")
    stolen = Browser(server.base)
    waiting("an identity arrives", "s-taken", stolen, "plain")
    challenge = asks("the password asks to link", stolen)
    server.edit((
        'INSERT INTO "ExternalIdentities" ("Id","UserId","Issuer","Subject","ProviderId","CreatedAt") VALUES (?,?,?,?,?,?)',
        (new_id(), other, issuer, "s-taken", "plain", now_text()),
    ))
    pass_it("the right code, the identity is another's", stolen, challenge)
    ask(rec, "what waits now", stolen, "GET", "/api/auth/oidc/pending")

    rec.note("== the account already has another identity at that provider")
    twice = Browser(server.base)
    waiting("a second identity arrives", "s-second", twice)
    challenge = asks("the password asks to link", twice)
    pass_it("the right code, the account has one already", twice, challenge)

    rec.note("== the browser meets another identity after the sign-in asked")
    swapped = Browser(server.base)
    waiting("an identity arrives", "s-first", swapped, "plain")
    challenge = asks("the password asks to link", swapped)
    waiting("another identity arrives in the same browser", "s-other", swapped, "plain")
    pass_it("the right code, the identity is not the one the sign-in saw", swapped, challenge)
    ask(rec, "what waits now", swapped, "GET", "/api/auth/oidc/pending")

    rec.note("== the identity is dropped before the second step is passed")
    dropped = Browser(server.base)
    waiting("an identity arrives", "s-dropped", dropped, "plain")
    challenge = asks("the password asks to link", dropped)
    ask(rec, "drop it", dropped, "DELETE", "/api/auth/oidc/pending")
    pass_it("the right code, nothing waits", dropped, challenge)

    rec.note("== the provider's own second step does not stand in for the account's when linking")
    proud = Browser(server.base)
    waiting("an identity arrives with a second step of its own", "s-trusted", proud, "trust", amr=["pwd", "mfa"], authTime=int(time.time()) - 30)
    challenge = asks("the password asks to link", proud)
    pass_it("the right code", proud, challenge)


def require(server, level):
    """What the instance asks of everybody's second step: optional, admins, everyone."""
    server.edit(('UPDATE "OperationsSettings" SET "MfaRequirement" = ?', (level,)))


def old_sign_in(server, name):
    """Makes every session of the account look like a sign-in from long ago."""
    return ('UPDATE "UserSessions" SET "AuthenticatedAt" = ? WHERE "UserId" = ?', ("2026-01-01 00:00:00.000000", user_id(server, name)))


def first_identity(page):
    return [identity["id"] for identity in page.json()["identities"]]


@scenario
def sso_security(server, rec):
    """The security page with providers set up: what it lists, a link started
    from it and completed, an identity removed, a session that signed in long
    ago refused before the identity is looked at, and the last way in."""
    require(server, "optional")
    page = Browser(server.base)
    sso_login(rec, "sign in", page, "ssopage")
    ask(rec, "the page, nothing linked", page, "GET", "/api/me/security")
    provider_reset("p0", subject="page-a")
    provider_reset("p1", subject="page-b")
    ask(rec, "a link for a provider nobody set up", page, "POST", "/api/me/security/identities/nope/link", {})
    sso_page(rec, "link the original provider", page, "link", "default", back="/settings?tab=security")
    sso_page(rec, "link it again", page, "link", "default")
    provider_reset("p0", subject="page-other")
    sso_page(rec, "link another identity of the same provider", page, "link", "default")
    sso_page(rec, "link the trusted provider", page, "link", "trust")
    shown = ask(rec, "the page, two linked", page, "GET", "/api/me/security")
    ids = first_identity(shown)
    ask(rec, "remove an identity nobody has", page, "POST", "/api/me/security/identities/%s/remove" % new_id(), {})
    ask(rec, "remove, not an id", page, "POST", "/api/me/security/identities/xyz/remove", {})
    ask(rec, "remove the trusted provider's identity", page, "POST", "/api/me/security/identities/%s/remove" % ids[1], {})
    ask(rec, "remove it again", page, "POST", "/api/me/security/identities/%s/remove" % ids[1], {})
    provider_reset("p1", subject="page-b")
    sso_page(rec, "link the trusted provider again", page, "link", "trust")
    # Three accounts whose password goes, and one whose sign-in grows old.
    key, only, gone = Browser(server.base), Browser(server.base), Browser(server.base)
    for browser, name, subject in ((key, "ssokey", "key-a"), (only, "ssoonly", "only-a"), (gone, "ssogone", "gone-a")):
        sso_login(rec, "sign in", browser, name)
        provider_reset("p0", subject=subject)
        sso_page(rec, "link the original provider", browser, "link", "default")
    server.edit(
        old_sign_in(server, "ssopage"),
        ('UPDATE "Users" SET "PasswordHash" = NULL WHERE "Username" IN (?,?,?)', ("ssokey", "ssoonly", "ssogone")),
        # The passkey is a second step, so changing the account takes one passed lately.
        ('UPDATE "UserSessions" SET "MfaCompletedAt" = ?, "MfaEvidence" = ? WHERE "UserId" = ?',
         (now_text(), "passkey", user_id(server, "ssokey"))),
        ('INSERT INTO "MfaMethods" ("Id","UserId","Type","Name","CreatedAt","ConfirmedAt") VALUES (?,?,?,?,?,?)',
         (new_id(), user_id(server, "ssokey"), "passkey", "Key", now_text(-3600), now_text(-3600))),
        ('INSERT INTO "ExternalIdentities" ("Id","UserId","Issuer","Subject","ProviderId","CreatedAt") VALUES (?,?,?,?,?,?)',
         (new_id(), user_id(server, "ssogone"), "https://Gone.Example.test/realms/x", "gone-b", "gone", now_text(-1800))),
    )
    rec.note("== the session of ssopage signed in long ago")
    shown = ask(rec, "the page, the session is old", page, "GET", "/api/me/security")
    ids = first_identity(shown)
    ask(rec, "remove an identity nobody has, the session is old", page, "POST", "/api/me/security/identities/%s/remove" % new_id(), {})
    ask(rec, "remove one, the session is old", page, "POST", "/api/me/security/identities/%s/remove" % ids[0], {})
    ask(rec, "link, the session is old", page, "POST", "/api/me/security/identities/plain/link", {})
    ask(rec, "link a provider nobody set up, the session is old", page, "POST", "/api/me/security/identities/nope/link", {})
    ask(rec, "sign in again at the provider, an identity this account has", page, "POST", "/api/me/security/identities/trust/reauthenticate", {})
    ask(rec, "sign in again at the provider, one it has not", page, "POST", "/api/me/security/identities/plain/reauthenticate", {})
    ask(rec, "remove with a wrong password, the session is old", page, "POST", "/api/me/security/identities/%s/remove" % ids[0], {"password": "wrong"})
    ask(rec, "remove with the password, the session is old", page, "POST", "/api/me/security/identities/%s/remove" % ids[0], {"password": PASSWORD})
    ask(rec, "the page afterwards", page, "GET", "/api/me/security")
    rec.note("== the last way in")
    ask(rec, "a passkey and an identity: the page", key, "GET", "/api/me/security")
    ids = first_identity(ask(rec, "only an identity: the page", only, "GET", "/api/me/security"))
    ask(rec, "only an identity: remove it", only, "POST", "/api/me/security/identities/%s/remove" % ids[0], {})
    ask(rec, "only an identity: sign in again at the provider", only, "POST", "/api/me/security/identities/default/reauthenticate", {})
    ids = first_identity(ask(rec, "a passkey and an identity: the page", key, "GET", "/api/me/security"))
    ask(rec, "a passkey and an identity: remove the identity", key, "POST", "/api/me/security/identities/%s/remove" % ids[0], {})
    shown = ask(rec, "an identity of a provider that is gone and one that is not: the page", gone, "GET", "/api/me/security")
    ids = first_identity(shown)
    ask(rec, "the identity that counts is the only one that does: remove it", gone, "POST", "/api/me/security/identities/%s/remove" % ids[1], {})
    ask(rec, "the gone identity may go", gone, "POST", "/api/me/security/identities/%s/remove" % ids[0], {})
    leave(rec, server, ["ssopage", "ssokey", "ssoonly", "ssogone"])
    leave_identities(rec, server)
    require(server, "everyone")


def register_external(rec, label, browser, name, invite, email=None):
    return ask(rec, label, browser, "POST", "/api/auth/register", {
        "username": name, "displayName": name.capitalize(), "email": email or "%s@example.test" % name,
        "inviteCode": invite, "external": True})


@scenario
def sso_reauth(server, rec):
    """Accounts made through a provider, which have no password: the first
    password inside the window of a fresh sign-in, after it, and a fresh
    sign-in at the provider as the proof."""
    invites = ["ssoreauth%d" % number for number in range(4)]
    server.edit(
        ('INSERT OR REPLACE INTO "IdentityProviderSettings" VALUES (?,1,?)', ("default", now_text())),
        *[('INSERT INTO "ServerInvites" ("Id","ServerId","CreatedById","ChannelId","Code","Uses","MaxUses","ExpiresAt","CreatedAt") '
           'SELECT ?, "ServerId", "UserId", NULL, ?, 0, NULL, NULL, ? FROM "ServerMembers" WHERE "OwnerSlot" = 1 LIMIT 1',
           (new_id(), code, now_text())) for code in invites],
    )
    require(server, "optional")
    now = int(time.time())

    def arrive(name, subject, browser, **more):
        provider_reset("p0", subject=subject, claims={"email": "%s@example.test" % name, "email_verified": True, "name": name.capitalize()}, **more)
        sso_in(rec, "%s arrives from the provider" % name, browser)

    rec.note("== a fresh sign-in stands in for the password")
    a = Browser(server.base)
    arrive("ssonewa", "new-a", a)
    register_external(rec, "an account is made", a, "ssonewa", invites[0])
    ask(rec, "the first password", a, "POST", "/api/me/security/password", {"newPassword": "Another-Passw0rd!2026"})
    ask(rec, "who", a, "GET", "/api/session")

    rec.note("== the sign-in grows old, and the provider's answer makes it fresh again")
    b = Browser(server.base)
    arrive("ssonewb", "new-b", b)
    register_external(rec, "an account is made", b, "ssonewb", invites[1])
    c = Browser(server.base)
    arrive("ssonewc", "new-c", c)
    register_external(rec, "an account is made", c, "ssonewc", invites[2])
    # b's sign-in grows old. c gets two mailed-code methods and a second step passed lately, so
    # that it can take one away while keeping its session.
    mine = user_id(server, "ssonewc")
    server.edit(
        old_sign_in(server, "ssonewb"),
        *[('INSERT INTO "MfaMethods" ("Id","UserId","Type","Name","CreatedAt","ConfirmedAt","EmailAddress") VALUES (?,?,?,?,?,?,?)',
           (new_id(), mine, "email", "Mail %d" % number, now_text(-86400), now_text(-86400), "ssonewc%d@example.test" % number)) for number in (1, 2)],
        ('UPDATE "UserSessions" SET "MfaCompletedAt" = ?, "MfaEvidence" = ? WHERE "UserId" = ?', (now_text(), "totp", mine)),
    )
    prefix, base = PROVIDERS["default"]
    provider_reset("p0", subject="new-c", authTime=int(time.time()) - 20)
    url = sso_ticket(rec, "c: a ticket for a fresh sign-in", c, "default", "reauthenticate")
    state = sso_start(rec, "c: out to the provider", c, url, prefix)
    method = [m["id"] for m in ask(rec, "c: the page", c, "GET", "/api/me/security").json()["methods"]][0]
    ask(rec, "c: a second step is taken away, the session stays", c, "POST", "/api/me/security/mfa/%s/remove" % method, {})
    sso_back(rec, "c: back from the provider, the account's security moved", c, state, base + "/callback")
    ask(rec, "c: who", c, "GET", "/api/session")
    ask(rec, "b: the first password, the sign-in is old", b, "POST", "/api/me/security/password", {"newPassword": "Another-Passw0rd!2026"})
    for what, change in (
        ("an hour old", {"subject": "new-b", "authTime": now - 3600}),
        ("no time at all", {"subject": "new-b", "authTime": None}),
        ("somebody else's identity", {"subject": "new-a", "authTime": now}),
        ("nobody's identity", {"subject": "new-z", "authTime": now}),
    ):
        provider_reset("p0", **change)
        sso_page(rec, "b: the provider's answer is %s" % what, b, "reauthenticate", "default", back="/settings?tab=security")
        ask(rec, "b: the first password, it was not confirmed", b, "POST", "/api/me/security/password", {"newPassword": "Another-Passw0rd!2026"})
    provider_reset("p0", subject="new-b", authTime=now - 20)
    url = sso_ticket(rec, "b: a ticket", b, "default", "reauthenticate")
    state = sso_start(rec, "b: out", b, url + "&returnUrl=%2F", prefix)
    sso_back(rec, "b: back, fresh", b, state, base + "/callback")
    sso_back(rec, "b: the same answer again", b, state, base + "/callback")
    ask(rec, "b: the first password, it was confirmed", b, "POST", "/api/me/security/password", {"newPassword": "Another-Passw0rd!2026"})
    ask(rec, "b: a link ticket asked of the reauthentication route", b, "GET", url.replace("/reauthenticate?", "/link?"))
    leave(rec, server, ["ssonewa", "ssonewb", "ssonewc"])
    leave_identities(rec, server)
    require(server, "everyone")


@scenario
def sso_policy(server, rec):
    """An account with a second step of its own signing in through a provider
    whose second step counts, or does not, and signing up through one while the
    instance asks everybody for a second step."""
    app, secret, method, codes, _ = enrol_totp(rec, server, "ssopolicy")
    user = user_id(server, "ssopolicy")
    codes_ = iter(codes)
    invites = ["ssopolicy%d" % number for number in range(4)]
    statements = []
    for name, prefix in (("trust", "p1"), ("plain", "p2"), ("default", "p0")):
        issuer = "https://%s:%d/%s" % (PROVIDERS_HOST, PROVIDER_PORT, prefix)
        statements.append(('INSERT INTO "ExternalIdentities" ("Id","UserId","Issuer","Subject","ProviderId","CreatedAt") VALUES (?,?,?,?,?,?)',
                           (new_id(), user, issuer, "policy", name, now_text(-3600))))
        statements.append(('INSERT OR REPLACE INTO "IdentityProviderSettings" VALUES (?,1,?)', (name, now_text())))
    for code in invites:
        statements.append(('INSERT INTO "ServerInvites" ("Id","ServerId","CreatedById","ChannelId","Code","Uses","MaxUses","ExpiresAt","CreatedAt") '
                           'SELECT ?, "ServerId", "UserId", NULL, ?, 0, NULL, NULL, ? FROM "ServerMembers" WHERE "OwnerSlot" = 1 LIMIT 1',
                           (new_id(), code, now_text())))
    server.edit(*statements)
    now = int(time.time())
    rec.note("== the provider's second step, for an account that has its own")
    for name in ("trust", "plain"):
        for what, change in (
            ("a second step a moment ago", {"amr": ["pwd", "mfa"], "authTime": now - 30}),
            ("a second step an hour ago", {"amr": ["pwd", "mfa"], "authTime": now - 3600}),
            ("a password only", {"amr": ["pwd"], "authTime": now - 30}),
            ("a second step and no time", {"amr": ["pwd", "mfa"], "authTime": None}),
        ):
            provider_reset(PROVIDERS[name][0], subject="policy", **change)
            browser = Browser(server.base)
            label = "%s, %s" % (name, what)
            answer = sso_in(rec, label, browser, name)
            ask(rec, label + ": who", browser, "GET", "/api/session")
            if answer is not None and "mfa_required" in (answer.header("location") or ""):
                ask(rec, label + ": the challenge waiting", browser, "GET", "/api/auth/mfa/pending")
    rec.note("== signing up while the instance asks everybody for a second step")
    for index, (name, change) in enumerate((
        ("trust", {"amr": ["mfa"], "authTime": now - 10}),
        ("trust", {"amr": ["pwd"], "authTime": now - 10}),
        ("plain", {"amr": ["mfa"], "authTime": now - 10}),
    )):
        username = "ssonew%s%d" % (name, index)
        provider_reset(PROVIDERS[name][0], subject="up-%d" % index, claims={"email": "%s@example.test" % username, "email_verified": True}, **change)
        browser = Browser(server.base)
        sso_in(rec, "%s arrives with %s" % (name, change["amr"]), browser, name)
        register_external(rec, "an account is made", browser, username, invites[index])
        ask(rec, "who", browser, "GET", "/api/session")
    leave(rec, server, ["ssopolicy", "ssonewtrust0", "ssonewtrust1", "ssonewplain2"])
    leave_identities(rec, server)


@scenario
def sso_named(server, rec):
    """An instance with named providers only: the original provider's routes do not exist."""
    require(server, "optional")
    page = Browser(server.base)
    sso_login(rec, "sign in", page, "ssopage")
    ask(rec, "the providers on offer", Browser(server.base), "GET", "/api/auth/oidc")
    ask(rec, "the page", page, "GET", "/api/me/security")
    for path in ("/api/auth/oidc/login", "/api/auth/oidc/callback", "/api/auth/oidc/link", "/api/auth/oidc/link?ticket=x",
                 "/api/auth/oidc/reauthenticate", "/api/auth/oidc/reauthenticate?ticket=x", "/api/auth/oidc/pending",
                 "/api/auth/oidc/work/link", "/api/auth/oidc/work/reauthenticate"):
        ask(rec, "the bare route %s" % path, page, "GET", path)
    ask(rec, "link the original provider", page, "POST", "/api/me/security/identities/default/link", {})
    ask(rec, "sign in again at the original provider", page, "POST", "/api/me/security/identities/default/reauthenticate", {})
    provider_reset("p3", subject="work-a")
    sso_page(rec, "link the named provider", page, "link", "work")
    ask(rec, "the page", page, "GET", "/api/me/security")
    provider_reset("p3", subject="work-a", authTime=int(time.time()) - 20)
    sso_page(rec, "sign in again at the named provider", page, "reauthenticate", "work")
    sso_in(rec, "sign in through the named provider", Browser(server.base), "work")
    leave(rec, server, ["ssopage"])
    leave_identities(rec, server)


def limits(server, rec):
    """Each rate limit, on the defaults: the request that is refused, and what it is refused with."""
    routes = [
        ("LoginPoll", 120, "GET", "/api/auth/mfa/pending", None),
        ("SecurityWrite", 30, "POST", "/api/auth/mfa/cancel", None),
        ("Login", 10, "POST", "/api/auth/mfa/verify", {"challengeId": new_id(), "code": "000000"}),
        ("AuthWrite", 5, "POST", "/api/auth/password/forgot", {"email": "nobody@example.test"}),
    ]
    for policy, permits, method, path, body in routes:
        browser = Browser(server.base)
        refused = None
        for number in range(1, permits + 3):
            answer = browser.send(method, path, body)
            if answer.status == 429:
                refused = number
                rec.step("%s: the request that is refused" % policy, (method, path, json.dumps(body) if body else ""), answer)
                break
        rec.note("%s: refused from request %s of %d permitted" % (policy, refused, permits))
    # The two second-step routes that share a policy share its count.
    browser = Browser(server.base)
    for number in range(1, 8):
        answer = browser.post("/api/auth/mfa/email/send", {"challengeId": new_id()})
        if answer.status == 429:
            rec.note("AuthWrite, by the mailed-code route: refused from request %d" % number)
            break


def slow_mail_server(port, delay):
    """A mail server that takes `delay` seconds to accept each message, for the
    timing run: `SLOW_MAIL=1` points both servers at it."""
    import socketserver
    import threading

    class Handler(socketserver.StreamRequestHandler):
        def handle(self):
            self.wfile.write(b"220 slow.test ESMTP\r\n")
            data = False
            for raw in self.rfile:
                line = raw.decode("latin-1").rstrip("\r\n")
                if data:
                    if line == ".":
                        data = False
                        time.sleep(delay)
                        self.wfile.write(b"250 2.0.0 Queued\r\n")
                    continue
                verb = line.split(" ")[0].upper()
                if verb == "EHLO":
                    self.wfile.write(b"250-slow.test\r\n250 8BITMIME\r\n")
                elif verb == "DATA":
                    data = True
                    self.wfile.write(b"354 go ahead\r\n")
                elif verb == "QUIT":
                    self.wfile.write(b"221 bye\r\n")
                    return
                else:
                    self.wfile.write(b"250 OK\r\n")

    class Server(socketserver.ThreadingTCPServer):
        allow_reuse_address = True
        daemon_threads = True

    server = Server(("0.0.0.0", port), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def timing(servers):
    """How long forgot takes for an address that is an account's and for one that is not."""
    import statistics
    slow = None
    if os.environ.get("SLOW_MAIL"):
        slow = slow_mail_server(SMTP_PORT + 2, 1.0)
        for server in servers:
            server.edit(('UPDATE "OperationsSettings" SET "MailSmtpPort" = ?', (SMTP_PORT + 2,)))
    for server in servers:
        clear_mail()
        samples = {"known": [], "unknown": []}
        for index in range(200):
            for kind, email in (("known", "shared@example.test"), ("unknown", "nobody%d@example.test" % index)):
                browser = Browser(server.base)
                start = time.perf_counter()
                answer = browser.post("/api/auth/password/forgot", {"email": email})
                samples[kind].append((time.perf_counter() - start) * 1000)
                assert answer.status == 202, answer.status
        for kind, values in samples.items():
            values.sort()
            cut = lambda p: values[min(len(values) - 1, int(len(values) * p))]
            print("   %-12s %-8s n=%d  min %.0f  p10 %.0f  median %.0f  p90 %.0f  p99 %.0f  max %.0f ms  (mean %.1f, deviation %.1f)" % (
                server.name, kind, len(values), values[0], cut(0.1), cut(0.5), cut(0.9), cut(0.99), values[-1],
                statistics.mean(values), statistics.pstdev(values)))
        if not slow:
            print("   mails that arrived for the known address on %s: %d" % (server.name, len(mails(200, wait=20))))
    if slow:
        slow.shutdown()


def prepare():
    """Starts the first server as a scenario would find it and leaves it
    running, for the comparison that is written in Rust (the passkey one,
    which needs an authenticator): `cargo test -p stellarr-api --test api
    passkey_oracle -- --ignored --nocapture`. Stop it with `oracle stop`."""
    server = Oracle()
    seed(server)
    print("the first server is ready on %s with accounts %s" % (server.base, ", ".join(ACCOUNTS)))


NAMED_ONLY = ("sso_named",)


def main():
    import signal
    # A signal ends the run through the `finally` below, which stops everything started.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    if sys.argv[1:] == ["prepare"]:
        prepare()
        shutil.rmtree(WORK, ignore_errors=True)
        return
    names = sys.argv[1:] or [name for name in SCENARIOS if PROVIDERS_HOST or not name.startswith("sso")]
    first = second = None
    different = 0
    try:
        if names == ["limits"]:
            first, second = Oracle(limits_raised=False), Rust(limits_raised=False)
            transcripts = []
            for server in (first, second):
                rec = Recording(server)
                limits(server, rec)
                transcripts.append(rec.lines)
            print("== limits")
            different += diff(transcripts[0], transcripts[1])
            print("\n%d difference(s)" % different)
            sys.exit(1 if different else 0)
        if PROVIDERS_HOST or any(name.startswith("sso") for name in names):
            start_provider()
        # One pair of servers for the scenarios that run on instances with the
        # original provider and named ones, and one for named providers only.
        groups = [[n for n in names if n not in NAMED_ONLY], [n for n in names if n in NAMED_ONLY]]
        for group, kind in zip(groups, ("default", "named")):
            if not group:
                continue
            first, second = Oracle(providers=kind if PROVIDERS_HOST else None), Rust(providers=kind if PROVIDERS_HOST else None)
            for server in (first, second):
                seed(server)
            if group == ["timing"]:
                timing((first, second))
                sys.exit(0)
            for name in group:
                transcripts = []
                for server in (first, second):
                    rec = Recording(server)
                    clear_mail()
                    try:
                        SCENARIOS[name](server, rec)
                    except BaseException:
                        if os.environ.get("SHOW"):
                            print("\n".join(rec.lines))
                        raise
                    transcripts.append(rec.lines)
                print("== %s" % name)
                if os.environ.get("SHOW"):
                    print("\n".join(transcripts[1]))
                different += diff(transcripts[0], transcripts[1])
            for server in (first, second):
                server.close()
            first = second = None
    finally:
        for server in (first, second):
            if server:
                server.close()
        stop_provider()
        shutil.rmtree(WORK, ignore_errors=True)
    for label, sides in ORDERS.items():
        if len(set(map(tuple, sides.values()))) > 1:
            print("cookie order differs at %r: %s" % (label, sides))
    print("\n%d difference(s)" % different)
    sys.exit(1 if different else 0)


# What differs on purpose, and is listed in api/README.md ("Single sign-on: where it differs from the
# first server"): the step it shows in, the line of each server, and why. A pair that is not listed
# here is a difference, and fails the run.
KNOWN = [
    ("b: the same answer again", "   location: /link-account?result=unconfirmed", "   location: /login?oidc_error=failed",
     "an answer whose binding cookie is gone cannot open its sealed state, so it cannot say it was a fresh sign-in"),
]


def diff(first, second):
    import difflib
    known = []
    if first != second:
        shown_first, shown_second = list(first), list(second)
        for tag, a1, a2, b1, b2 in difflib.SequenceMatcher(None, first, second, autojunk=False).get_opcodes():
            if tag != "replace" or a2 - a1 != b2 - b1:
                continue
            for offset in range(a2 - a1):
                at = a1 + offset
                step = next((line[3:] for line in reversed(first[:at + 1]) if line.startswith("-- ")), "")
                for label, line_first, line_second, reason in KNOWN:
                    if step == label and first[at] == line_first and second[b1 + offset] == line_second:
                        shown_first[at] = shown_second[b1 + offset] = "   <known difference: %s>" % reason
                        known.append(reason)
        first, second = shown_first, shown_second
    for reason in known:
        print("   known difference: %s" % reason)
    if first == second:
        print("   same, %d lines" % len(first))
        return 0
    count = 0
    for line in difflib.unified_diff(first, second, "first server", "this server", lineterm="", n=2):
        print("   " + line)
        count += 1
    return 1


if __name__ == "__main__":
    main()
