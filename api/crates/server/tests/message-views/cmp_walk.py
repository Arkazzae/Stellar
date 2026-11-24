import sys, json, urllib.parse
sys.path.insert(0, '.')
from cmp_lib import *
state = json.load(open("state.json"))
names = list(state["people"])
P = Pair(names)
ch = state["channels"]
def walk(who, cid, qs, limit):
    """Walks the index to its end on both servers, comparing every page and cursor, and then feeds each server's
    cursor to the other."""
    path = f"/api/channels/{cid}/messages/index?limit={limit}" + qs
    cur_o = cur_r = None
    pages = 0
    cross = []
    while True:
        po = path + (f"&cursor={cur_o}" if cur_o else ""); pr = path + (f"&cursor={cur_r}" if cur_r else "")
        a = P.o.go("GET", po, who); b = P.r.go("GET", pr, who); P.count += 1
        na, nb = norm(pairs(a[1])), norm(pairs(b[1]))
        if a[0] != b[0] or na != nb or a[2] != b[2]:
            P.diffs += 1; print("DIFF walk", qs, limit, who, a[0], b[0], "page", pages); print("  ", first_diff(na, nb)); return pages
        pages += 1
        if a[0] != 200: return pages
        ja, jb = json.loads(a[1]), json.loads(b[1])
        cur_o, cur_r = ja["nextCursor"], jb["nextCursor"]
        if cur_o:
            cross.append((cur_o, cur_r))
        if not cur_o: break
    # each server's cursor on the other server
    for co, cr in cross[:3]:
        a = P.o.go("GET", path + f"&cursor={cr}", who); b = P.r.go("GET", path + f"&cursor={co}", who); P.count += 1
        if norm(pairs(a[1])) != norm(pairs(b[1])) or a[0] != b[0]:
            P.diffs += 1; print("DIFF cross", qs, a[0], b[0])
    return pages
total = 0
for who in ["boss", "dawid"]:
    for cname in ["general", "lounge", "noreact", "secret"]:
        for qs, limit in [("", 100), ("", 37), ("&kind=media", 20), ("&kind=Links", 25), ("&kind=all&query=lampa", 10), ("&query=raport", 15), ("&kind=media&query=%C5%82%C3%B3d%C5%BA", 7), ("&kind=links&query=example", 30), ("&query=kot+pies", 50)]:
            total += walk(who, ch[cname], qs, limit)
print("pages walked", total, "requests", P.count, "differences", P.diffs)
