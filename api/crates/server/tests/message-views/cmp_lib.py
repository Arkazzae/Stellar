import json, base64, struct, uuid, sys, re
sys.path.insert(0, '.')
from lib import *
from harness import ORACLE, RUST
HEADERS = ["Content-Type", "Cache-Control", "Allow", "Location", "WWW-Authenticate", "Pragma", "Expires", "Vary"]
TIME = re.compile(r"\d{4}-\d\d-\d\d[T ]\d\d:\d\d:\d\d(\.\d+)?Z?")
def decode_cursor(text):
    if not isinstance(text, str): return text
    try:
        raw = base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))
    except Exception: return text
    if len(raw) != 37: return text
    ch = uuid.UUID(bytes_le=raw[:16])
    ticks, off, kind, h = struct.unpack(">qiBQ", raw[16:])
    return {"channel": str(ch), "ticks": ticks, "offset": off, "kind": kind, "hash": h}
def pairs(body):
    try: return json.loads(body, object_pairs_hook=lambda p: ("obj", p))
    except Exception: return ("raw", body)
def norm(node, drop_times=False):
    if isinstance(node, tuple) and node[0] == "obj":
        out = []
        for k, v in node[1]:
            if k == "traceId": out.append((k, "<trace>")); continue
            if k == "nextCursor" and isinstance(v, str):
                d = decode_cursor(v)
                if isinstance(d, dict): d = dict(d); d["ticks"] = "<snapshot>"
                out.append((k, d))
            else: out.append((k, norm(v, drop_times)))
        return ("obj", out)
    if isinstance(node, list): return [norm(x, drop_times) for x in node]
    if drop_times and isinstance(node, str) and TIME.fullmatch(node): return "<time>"
    return node
def first_diff(a, b, path="$"):
    if type(a) != type(b): return f"{path}: {str(a)[:80]!r} vs {str(b)[:80]!r}"
    if isinstance(a, tuple):
        ka = [k for k, _ in a[1]] if a[0] == "obj" else None
        if a[0] == "obj":
            kb = [k for k, _ in b[1]]
            if ka != kb: return f"{path}: keys {ka} vs {kb}"
            for (k, x), (_, y) in zip(a[1], b[1]):
                d = first_diff(x, y, path + "." + k)
                if d: return d
            return None
        return None if a == b else f"{path}: raw differs"
    if isinstance(a, list):
        if len(a) != len(b): return f"{path}: length {len(a)} vs {len(b)}"
        for i, (x, y) in enumerate(zip(a, b)):
            d = first_diff(x, y, f"{path}[{i}]")
            if d: return d
        return None
    return None if a == b else f"{path}: {str(a)[:80]!r} vs {str(b)[:80]!r}"
class Side:
    def __init__(self, base): self.base = base; self.cookies = {}; self.ids = {}
    def sign_in(self, name):
        c, i = login(self.base, name); self.cookies[name] = c; self.ids[name] = i
    def go(self, method, path, who=None, body=None, headers=None):
        s, b, h = call(self.base, method, path, body, self.cookies.get(who) if who else None, headers)
        return s, b, {k: h.get(k) for k in HEADERS}
class Pair:
    def __init__(self, names):
        self.o, self.r = Side(ORACLE), Side(RUST)
        for n in names: self.o.sign_in(n); self.r.sign_in(n)
        self.diffs = 0; self.count = 0
    def go(self, method, path, who=None, body=None, headers=None, times=False, label=None, quiet=False):
        a = self.o.go(method, path, who, body, headers); b = self.r.go(method, path, who, body, headers)
        self.count += 1
        na, nb = norm(pairs(a[1]), times), norm(pairs(b[1]), times)
        problems = []
        if a[0] != b[0]: problems.append(f"status {a[0]} vs {b[0]}")
        for h in HEADERS:
            if a[2][h] != b[2][h]: problems.append(f"header {h}: {a[2][h]!r} vs {b[2][h]!r}")
        if na != nb: problems.append("body differs")
        if problems:
            self.diffs += 1
            if not quiet or self.diffs <= 40:
                print("DIFF", method, path, who, "|", "; ".join(problems))
                if "body differs" in problems:
                    print("   first:", first_diff(na, nb))
        return a, b
