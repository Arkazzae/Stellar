import sys, base64
cases = open("work/cases.txt").read().split("\n")[:-1]
dn = open("work/dn-answers.txt").read().split("\n")[:-1]
rs = open("work/rust-answers.txt").read().split("\n")[:-1]
i = j = 0
bad = {}
total = {}
def dec(b): return base64.b64decode(b).decode("utf-16-le", "replace") if b not in ("-", "bad", "crash") else b
for case in cases:
    kind = case.split(" ", 1)[0]
    if kind == "CF":
        a = []; 
        while dn[i] != "END": a.append(dn[i]); i += 1
        i += 1
        b = []
        while rs[j] != "END": b.append(rs[j]); j += 1
        j += 1
        sa = {tuple(sorted(map(int, x.split(",")))) for x in a}; sb = {tuple(sorted(map(int, x.split(",")))) for x in b}
        total[kind] = 1
        if sa != sb:
            bad.setdefault(kind, []).append((sorted(sa - sb)[:10], sorted(sb - sa)[:10]))
        continue
    x, y = dn[i], rs[j]; i += 1; j += 1
    total[kind] = total.get(kind, 0) + 1
    if kind == "D" and x == "crash": x = "bad"
    if x != y:
        bad.setdefault(kind, []).append((case[:90], x[:120], y[:120]))
for k in total: print(k, "cases", total[k], "differences", len(bad.get(k, [])))
for k, v in bad.items():
    for item in v[:6]: print(k, item)
