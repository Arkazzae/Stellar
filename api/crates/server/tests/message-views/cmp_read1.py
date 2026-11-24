import sys, json, sqlite3
sys.path.insert(0, '.')
from cmp_lib import *
state = json.load(open("state.json"))
names = list(state["people"])
P = Pair(names)
ch = state["channels"]
db = sqlite3.connect("work/shared.db")
roots = [r[0].lower() for r in db.execute('SELECT ReplyToMessageId FROM Messages WHERE ReplyToMessageId IS NOT NULL GROUP BY 1 ORDER BY COUNT(*) DESC LIMIT 6')]
roots += [r[0].lower() for r in db.execute('SELECT Id FROM Messages WHERE ReplyToMessageId IS NOT NULL LIMIT 2')]  # a reply, not a root
roots += [r[0].lower() for r in db.execute('SELECT Id FROM Messages WHERE ChannelId=? LIMIT 2', (ch["noreact"].upper(),))]  # other channel's messages
roots += ["00000000-0000-4000-8000-000000000000"]
for who in names:
    for cname, cid in list(ch.items()) + [("unknown", "00000000-0000-4000-8000-000000000001"), ("voice", [c for c in [state["channels"].get("voice")] if c][0] if state["channels"].get("voice") else "00000000-0000-4000-8000-000000000002")]:
        P.go("GET", f"/api/channels/{cid}/pins", who, quiet=True)
        P.go("GET", f"/api/channels/{cid}/threads", who, quiet=True)
        for r in roots:
            P.go("GET", f"/api/channels/{cid}/threads/{r}", who, quiet=True)
print("requests", P.count, "differences", P.diffs)
