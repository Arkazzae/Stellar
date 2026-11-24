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

here = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("probe", os.path.join(here, "permissions-probe.py"))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)

GUID = re.compile(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
MOMENT = re.compile(r'("(?:bannedAt)":")(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,7})?Z)(")')


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

        text = MOMENT.sub(r"\1<moment>\3", text)
        return GUID.sub(replace, text)


def describe(world, status, body, headers):
    lines = [str(status)]
    for name in ("Content-Type", "Location", "Allow"):
        value = headers.get(name)
        if value:
            lines.append(f"{name.lower()}: {world.scrub(value)}")
    text = body.decode("utf-8", "replace")
    # A trace identifier differs by nature.
    text = re.sub(r'"traceId":"[^"]*"', '"traceId":"<trace>"', text)
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


def run(ours, theirs, work, plan_path):
    users = [line.split()[0] for line in open(f"{work}/users.txt")]
    mine = sign_in(ours, users)
    first = sign_in(theirs, users)
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
                        status, body, headers = probe.call(base, ask["method"], ask["path"], ask["body"], cookies[ask["actor"]], header)
                        answers.append(describe(seen, status, body, headers))
                    steps += 1
                    kind = GUID.sub("{id}", ask["path"])
                    key = (ask["method"], kind, answers[0].split("\n")[0])
                    seen_statuses[key] = seen_statuses.get(key, 0) + 1
                    if answers[0] != answers[1]:
                        differences += 1
                        print(f"DIFFERENT world {number} step {index}: {ask['actor']} {ask['method']} {ask['path']} {json.dumps(ask['body'])}")
                        print("   ours  ", answers[0].replace("\n", "\n          "))
                        print("   theirs", answers[1].replace("\n", "\n          "))
    for (method, kind, status), count in sorted(seen_statuses.items()):
        print(f"   {count:5d}  {status}  {method} {kind}")
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
                print("   ours  ", json.dumps(ours[server][part])[:1500])
                print("   theirs", json.dumps(theirs.get(server, {}).get(part))[:1500])
    totals = {part: sum(len(ours[server][part]) for server in ours) for part in next(iter(ours.values()))}
    print(f"{len(ours)} worlds compared in the files ({totals}), {differences} differences")
    return 1 if differences else 0


if __name__ == "__main__":
    command = sys.argv[1]
    if command == "run":
        sys.exit(run(*sys.argv[2:6]))
    elif command == "dump":
        dump(*sys.argv[2:5])
    elif command == "diff":
        sys.exit(diff(*sys.argv[2:4]))
