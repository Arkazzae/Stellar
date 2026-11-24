#!/usr/bin/env python3
"""Talks to both servers for tests/roles-shared-database.sh.

  roles-probe.py run <this-url> <first-url> <workdir> <plan.jsonl>
      signs every person in on both servers, sends each world's plan to both,
      step by step, and compares the answers (status, the headers a browser
      cares about, the JSON with ids that the servers made replaced), then the
      lists of roles and members after every change.
  roles-probe.py dump <database> <plan.jsonl> <out.json>
      what each world holds in the file: roles by rank, who holds what,
      overrides, bans and the audit lines, with the ids the servers made replaced.
  roles-probe.py diff <ours.json> <theirs.json>
"""
import importlib.util
import json
import os
import re
import sqlite3
import sys
import urllib.error
import urllib.request
import datetime

here = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("probe", os.path.join(here, "permissions-probe.py"))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)

GUID = re.compile(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
MOMENT = re.compile(r'("(?:bannedAt)":")(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,7})?Z)(")')


def fresh(match):
    """A moment a server just made differs by nature; one that was arranged is compared as it is."""
    moment = match.group(2)
    stamp = datetime.datetime.strptime(moment[:19], "%Y-%m-%dT%H:%M:%S").replace(tzinfo=datetime.timezone.utc)
    if abs((datetime.datetime.now(datetime.timezone.utc) - stamp).total_seconds()) < 3600:
        return match.group(1) + "<moment>" + match.group(3)
    return match.group(0)


class World:
    """What differs by nature, replaced: ids a server made, in the order they appear."""

    def __init__(self, known):
        self.known = {k.lower() for k in known}
        self.made = {}

    def scrub(self, text):
        def replace(match):
            found = match.group(0).lower()
            if found in self.known:
                return found
            return "<made %d>" % self.made.setdefault(found, len(self.made) + 1)

        text = MOMENT.sub(fresh, text)
        return GUID.sub(replace, text)


def send(base, ask, cookie, header):
    """The request: without a content type when the step says so, with the text it gives as its body."""
    header = {**header, **({"X-Stellarr-Server": ask["server"]} if ask.get("server") else {})}
    if not ask.get("noContentType") and "raw" not in ask:
        return probe.call(base, ask["method"], ask["path"], ask["body"], cookie, header)
    if "raw" in ask:
        data = ask["raw"].encode()
    else:
        data = json.dumps(ask["body"]).encode() if ask["body"] is not None else None
    request = urllib.request.Request(base + ask["path"], method=ask["method"], data=data)
    for name, value in {**probe.CLIENT, **header}.items():
        if name != "Content-Type":
            request.add_header(name, value)
    request.add_header("Cookie", cookie)
    try:
        with urllib.request.build_opener(probe.NoRedirect).open(request) as response:
            return response.status, response.read(), response.headers
    except urllib.error.HTTPError as error:
        return error.code, error.read(), error.headers


def describe(world, status, body, headers):
    lines = [str(status)]
    for name in ("Content-Type", "Location", "Allow"):
        value = headers.get(name)
        if value:
            lines.append(f"{name.lower()}: {world.scrub(value)}")
    text = body.decode("utf-8", "replace")
    # A trace identifier differs by nature.
    text = re.sub(r'"traceId":"[^"]*"', '"traceId":"<trace>"', text)
    try:
        # The first server wrote every character outside ASCII as \uXXXX; the
        # JSON is the same, so it is compared as JSON, keys in their order.
        text = json.dumps(json.loads(text), ensure_ascii=False, separators=(",", ":"))
    except ValueError:
        pass
    lines.append("body: " + world.scrub(text))
    return "\n".join(lines)


def sign_in(base, users):
    cookies = {}
    for name in users:
        status, _, headers = probe.call(base, "POST", "/api/auth/login", {"username": name, "password": probe.PASSWORD})
        if status != 200:
            raise SystemExit(f"{name} could not sign in on {base}: {status}")
        cookies[name] = probe.session_cookie(headers)
    return cookies


def login(base, work, plan_path, out_path):
    """Signs the people of a plan in and keeps their cookies, for a run after the database is edited."""
    people = plan_people(work, plan_path)
    with open(out_path, "w") as out:
        json.dump(sign_in(base, people), out)


def plan_people(work, plan_path):
    actors = set()
    with open(plan_path) as plan_file:
        for line in plan_file:
            actors.update(step["actor"] for step in json.loads(line)["steps"])
    users = [line.split()[0] for line in open(f"{work}/users.txt")]
    return [name for name in users if name in actors or name == users[0]]


def edit_second_step(database, users_path):
    """person1, person4 and person5 get a second step their sessions never passed, person2 one passed a
    minute ago, person3 none: the sessions were made before, so the accounts only now have a step."""
    db = sqlite3.connect(database)
    ids = {line.split()[0]: line.split()[1].upper() for line in open(users_path)}
    now = datetime.datetime.utcnow()
    for name in ("person1", "person2", "person4", "person5"):
        db.execute('INSERT INTO "MfaMethods" ("Id","UserId","Type","Name","CreatedAt","ConfirmedAt") VALUES (?,?,?,?,?,?)',
                   (os.urandom(16).hex().upper(), ids[name], "totp", "Phone", now.strftime("%Y-%m-%d %H:%M:%S.%f"), now.strftime("%Y-%m-%d %H:%M:%S.%f")))
    minute = (now - datetime.timedelta(seconds=60)).strftime("%Y-%m-%d %H:%M:%S.%f")
    db.execute('UPDATE "UserSessions" SET "MfaCompletedAt" = ? WHERE "UserId" = ?', (minute, ids["person2"]))
    for name in ("person1", "person4", "person5"):
        db.execute('UPDATE "UserSessions" SET "MfaCompletedAt" = NULL WHERE "UserId" = ?', (ids[name],))
    db.commit()


def run(ours, theirs, work, plan_path, cookies_ours=None, cookies_theirs=None):
    actors = set()
    with open(plan_path) as plan_file:
        for line in plan_file:
            actors.update(step["actor"] for step in json.loads(line)["steps"])
    users = [line.split()[0] for line in open(f"{work}/users.txt")]
    users = [name for name in users if name in actors or name == users[0]]
    mine = json.load(open(cookies_ours)) if cookies_ours else sign_in(ours, users)
    first = json.load(open(cookies_theirs)) if cookies_theirs else sign_in(theirs, users)
    boss = users[0]
    differences = 0
    steps = 0
    seen_statuses = {}
    with open(plan_path) as plan_file:
        for number, line in enumerate(plan_file):
            plan = json.loads(line)
            worlds = (World(plan["known"]), World(plan["known"]))
            header = {"X-Stellarr-Server": plan["server"]}
            for index, step in enumerate(plan["steps"] + [None]):
                asks = [step] if step else []
                # After the last step, and after every change, both sides are asked for the lists.
                if step is None or step["method"] != "GET":
                    asks += [
                        {"actor": boss, "method": "GET", "path": "/api/admin/roles", "body": None},
                        {"actor": boss, "method": "GET", "path": "/api/admin/members", "body": None},
                    ]
                for ask in asks:
                    answers = []
                    for (base, cookies), seen in ((( ours, mine), worlds[0]), ((theirs, first), worlds[1])):
                        status, body, headers = send(base, ask, cookies[ask["actor"]], header)
                        answers.append(describe(seen, status, body, headers))
                    steps += 1
                    kind = GUID.sub("{id}", ask["path"])
                    key = (ask.get("class", "owner"), ask["method"], kind, answers[0].split("\n")[0])
                    seen_statuses[key] = seen_statuses.get(key, 0) + 1
                    if answers[0] != answers[1]:
                        differences += 1
                        print(f"DIFFERENT world {number} step {index}: {ask['actor']} {ask['method']} {ask['path']} {json.dumps(ask['body'])}")
                        print("   ours  ", answers[0].replace("\n", "\n          "))
                        print("   theirs", answers[1].replace("\n", "\n          "))
    by_class = {}
    for (actor_class, method, kind, status), count in seen_statuses.items():
        if method != "GET":
            by_class.setdefault(actor_class, {}).setdefault(status, 0)
            by_class[actor_class][status] += count
    for actor_class, counts in sorted(by_class.items()):
        print(f"   {actor_class:9s} {dict(sorted(counts.items()))}")
    print(f"{steps} requests, {differences} differences")
    return 1 if differences else 0


def dump(database, plan_path, out_path):
    db = sqlite3.connect(database)
    result = {}
    with open(plan_path) as plan_file:
        for line in plan_file:
            plan = json.loads(line)
            server = plan["server"].upper()
            known = {k.lower() for k in plan["known"]}
            made = {}

            def shape(value):
                if value is None:
                    return None
                text = str(value)

                def replace(match):
                    found = match.group(0).lower()
                    return found if found in known else "<made %d>" % made.setdefault(found, len(made) + 1)

                return GUID.sub(replace, text)

            names = {row[0]: row[1] for row in db.execute('SELECT "Id", "Name" FROM "ServerRoles" WHERE "ServerId" = ?', (server,))}
            roles = [list(row) for row in db.execute(
                'SELECT "Name", "Color", "Permissions", "Position", "IsSystem", "IsOwner" FROM "ServerRoles" WHERE "ServerId" = ? ORDER BY "Position", "Name"', (server,))]
            members = []
            for user, top, owner_slot, joined in db.execute('SELECT "UserId", "RoleId", "OwnerSlot", "JoinedAt" FROM "ServerMembers" WHERE "ServerId" = ? ORDER BY "UserId"', (server,)):
                held = sorted(names.get(row[0], "?") for row in db.execute(
                    'SELECT "RoleId" FROM "ServerMemberRoles" WHERE "ServerId" = ? AND "UserId" = ?', (server, user)))
                members.append([user, names.get(top, "?"), owner_slot, joined, held])
            overrides = sorted(
                [row[0], shape(row[1]), names.get(row[2], "?"), row[3], row[4]]
                for row in db.execute('SELECT "TargetType", "TargetId", "RoleId", "AllowMask", "DenyMask" FROM "PermissionOverrides" WHERE "ServerId" = ?', (server,)))
            member_overrides = sorted(
                [row[0], shape(row[1]), row[2], row[3], row[4]]
                for row in db.execute('SELECT "TargetType", "TargetId", "UserId", "AllowMask", "DenyMask" FROM "MemberPermissionOverrides" WHERE "ServerId" = ?', (server,)))
            bans = sorted(list(row) for row in db.execute('SELECT "UserId", "BannedById", "Reason" FROM "ServerBans" WHERE "ServerId" = ?', (server,)))
            audit = sorted(
                [row[0], row[1], row[2], shape(row[3]), row[4], row[5], row[6]]
                for row in db.execute('SELECT "ActorId", "Action", "TargetType", "TargetId", "TargetName", "Summary", "ChangesJson" FROM "ServerAuditEntries" WHERE "ServerId" = ?', (server,)))
            result[plan["server"]] = {"roles": roles, "members": members, "overrides": overrides,
                                      "memberOverrides": member_overrides, "bans": bans, "audit": audit}
    with open(out_path, "w") as out:
        json.dump(result, out, indent=1, sort_keys=True)


def diff(ours_path, theirs_path):
    ours = json.load(open(ours_path))
    theirs = json.load(open(theirs_path))
    differences = 0
    for server in ours:
        for part in ours[server]:
            if ours[server][part] != theirs.get(server, {}).get(part):
                differences += 1
                print(f"DIFFERENT {part} of {server}")
                mine, first = ours[server][part], theirs.get(server, {}).get(part) or []
                print("   only ours  ", json.dumps([row for row in mine if row not in first])[:1500])
                print("   only theirs", json.dumps([row for row in first if row not in mine])[:1500])
    totals = {part: sum(len(ours[server][part]) for server in ours) for part in next(iter(ours.values()))}
    print(f"{len(ours)} worlds compared in the files ({totals}), {differences} differences")
    return 1 if differences else 0


if __name__ == "__main__":
    command = sys.argv[1]
    if command == "run":
        sys.exit(run(*sys.argv[2:8]))
    elif command == "login":
        login(*sys.argv[2:6])
    elif command == "edit-second-step":
        edit_second_step(*sys.argv[2:4])
    elif command == "dump":
        dump(*sys.argv[2:5])
    elif command == "diff":
        sys.exit(diff(*sys.argv[2:4]))
