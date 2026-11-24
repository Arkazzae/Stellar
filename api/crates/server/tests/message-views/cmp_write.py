"""Reactions and pins on both servers, request by request, then the rows each left. Both servers start from
the same database file (harness.py); run restart first."""
import sys, json, sqlite3, threading, subprocess, urllib.parse, shutil, os
sys.path.insert(0, '.')
import harness
from cmp_lib import *
state = json.load(open("state.json"))
db0 = sqlite3.connect("work/shared.db")
ch = state["channels"]
def msgs(channel, n, where="1=1"):
    return [r[0].lower() for r in db0.execute(f'SELECT Id FROM Messages WHERE ChannelId=? AND IsSystem=0 AND {where} ORDER BY CreatedAt LIMIT ?', (ch[channel].upper(), n))]
g, l, nr, sec = msgs("general", 6), msgs("lounge", 6), msgs("noreact", 3), msgs("secret", 3)
unpinned_l = msgs("lounge", 3, "PinnedAt IS NULL")
pinned_l = msgs("lounge", 3, "PinnedAt IS NOT NULL")
P = Pair(list(state["people"]))
E = lambda e: urllib.parse.quote(e, safe="")
def go(method, path, who, **kw): return P.go(method, path, who, times=True, **kw)
awkward = ["👍", "👍🏽", "🇵🇱", "👨‍👩‍👧", "❤️", "✌🏿", "🏳️‍🌈", "<:blob:0123456789>", ":smile:", "ok", "a", "ż", "é", "%", "%25", "%2F", "%2f", "%FF", "%C3%28", "%5C", "+", "a+b", "?", "#", "%3F", "%23", "x" * 64, "x" * 65, "😀" * 32, "😀" * 33, "a b", "%20x%20", "%09", "%0A", "%C2%A0", "%E3%80%80", "%E2%80%8B", "%EF%BB%BF", "%1F", "%7F", "%00", "%u00e9", "%F0%9F", "%ED%A0%80", "é", "%C3%A9", "%c3%a9"]
raw_paths = {"%25", "%2F", "%2f", "%FF", "%C3%28", "%5C", "%3F", "%23", "%09", "%0A", "%C2%A0", "%E3%80%80", "%E2%80%8B", "%EF%BB%BF", "%1F", "%7F", "%00", "%u00e9", "%F0%9F", "%ED%A0%80", "%C3%A9", "%c3%a9", "%20x%20"}
for e in awkward:
    seg = e if e in raw_paths else E(e)
    for who in ["boss", "anna"]:
        go("PUT", f"/api/messages/{g[0]}/reactions/{seg}", who)
    go("DELETE", f"/api/messages/{g[0]}/reactions/{seg}", "anna")
# the permission and visibility cases
go("PUT", f"/api/messages/{nr[0]}/reactions/{E('👍')}", "anna")       # addReactions denied
go("PUT", f"/api/messages/{nr[0]}/reactions/{E('👍')}", "boss")
go("DELETE", f"/api/messages/{nr[0]}/reactions/{E('👍')}", "anna")     # taking back needs no permission
go("PUT", f"/api/messages/{sec[0]}/reactions/{E('👍')}", "dawid")      # cannot see it
go("DELETE", f"/api/messages/{sec[0]}/reactions/{E('👍')}", "dawid")
go("PUT", f"/api/messages/{sec[0]}/reactions/{E('👍')}", "bartek")
go("PUT", "/api/messages/00000000-0000-4000-8000-000000000000/reactions/%F0%9F%91%8D", "boss")
go("PUT", "/api/messages/not-a-guid/reactions/%F0%9F%91%8D", "boss")
go("PUT", f"/api/messages/{g[0].upper()}/reactions/{E('🎉')}", "boss")
go("PUT", f"/api/messages/{g[0].replace('-', '')}/reactions/{E('🎉')}", "celina")
go("PUT", f"/api/messages/{g[0]}/reactions/", "boss")
go("PUT", f"/api/messages/{g[0]}/reactions", "boss")
go("PUT", f"/api/messages/{g[0]}/reactions/{E('🎉')}/x", "boss")
go("PUT", f"/api/messages/{g[0]}/reactions/{E('🎉')}", None)
go("PUT", f"/api/messages/{g[0]}/reactions/{E('🎉')}", "boss", headers={"X-Stellarr-Server": "xx"})
go("PUT", f"/api/messages/{g[0]}/reactions/{E('🎉')}", "boss", headers={"X-Stellarr-Server": "00000000-0000-4000-8000-000000000000"})
go("PUT", f"/api/messages/{g[0]}/reactions/{E('🎉')}?serverId=" + state["server"], "boss")
go("GET", f"/api/messages/{g[0]}/reactions/{E('🎉')}", "boss")
go("PATCH", f"/api/messages/{g[0]}/reactions/{E('🎉')}", "boss")
go("PUT", f"/api/messages/{g[1]}/reactions/{E('🎉')}", "boss", body={"x": 1})
# concurrency: the same reaction eight times at once
results = {"o": [], "r": []}
def put(side, who, path):
    results[side].append(side_obj[side].go("PUT", path, who)[:2])
side_obj = {"o": P.o, "r": P.r}
for side in "or":
    ts = [threading.Thread(target=put, args=(side, "celina", f"/api/messages/{g[2]}/reactions/{E('🧵')}")) for _ in range(8)]
    [t.start() for t in ts]; [t.join() for t in ts]
print("concurrent adds, statuses/counts:", {s: sorted({(r[0], json.loads(r[1])["count"]) for r in v}) for s, v in results.items()})
# many distinct reactions on one message: no limit
for n in range(80):
    P.go("PUT", f"/api/messages/{g[3]}/reactions/{E('r%d' % n)}", "boss", times=True, quiet=True)
# pins
for who in ["anna", "dawid", "boss"]:
    go("PUT", f"/api/messages/{g[4]}/pin", who)
go("PUT", f"/api/messages/{g[4]}/pin", "boss")        # again
go("GET", f"/api/channels/{ch['general']}/pins", "anna")
go("DELETE", f"/api/messages/{g[4]}/pin", "anna")
go("DELETE", f"/api/messages/{g[4]}/pin", "boss")
go("DELETE", f"/api/messages/{g[4]}/pin", "boss")     # again
go("PUT", f"/api/messages/{sec[1]}/pin", "dawid")
go("PUT", f"/api/messages/{sec[1]}/pin", "boss")
go("PUT", f"/api/messages/{sec[1]}/pin", "anna")
go("PUT", "/api/messages/00000000-0000-4000-8000-000000000000/pin", "boss")
go("PUT", "/api/messages/zz/pin", "boss")
go("PUT", f"/api/messages/{unpinned_l[0]}/pin", "boss")       # over the limit
go("DELETE", f"/api/messages/{pinned_l[0]}/pin", "boss")
go("PUT", f"/api/messages/{unpinned_l[0]}/pin", "boss")       # 57 -> 56 pinned, still over
go("PUT", f"/api/messages/{pinned_l[1]}/pin", "boss")         # already pinned, over the limit: fine
go("PUT", f"/api/messages/{g[4]}/pin", "boss", headers={"X-Stellarr-Server": "xx"})
go("GET", f"/api/channels/{ch['lounge']}/pins", "boss")
go("GET", f"/api/channels/{ch['general']}/pins", "boss")
# a pin of a message that has reactions and replies: the answer says which reactions are the caller's
go("PUT", f"/api/messages/{g[2]}/pin", "celina") if False else None
go("PUT", f"/api/messages/{g[2]}/pin", "boss")
go("GET", f"/api/channels/{ch['general']}/pins", "celina")
print("requests", P.count, "differences", P.diffs)
