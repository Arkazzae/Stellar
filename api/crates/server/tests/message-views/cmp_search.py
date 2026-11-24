import sys, json, random, urllib.parse
sys.path.insert(0, '.')
from cmp_lib import *
random.seed(21)
state = json.load(open("state.json"))
P = Pair(["boss", "dawid"])
ch = state["channels"]
Q = urllib.parse.quote
qs = ["lampa", "Lampa", "LAMPA", "lamp", "ł", "lodz", "łódź", "Lodz", "zazolc", "zażółć", "gesla", "gęślą", "jazn", "kot", "KOT", "kot pies", "pies kot", "raport", "rap", "ra", "r", "rapor*", "\"raport\"", "\"kot pies\"", "kot*", "*", "**", "\"", "\"\"", "'", "(", ")", "()", "(kot)", "-kot", "kot -pies", "kot AND pies", "kot OR pies", "kot NOT pies", "NOT kot", "AND", "OR", "NOT", "NEAR", "NEAR(kot pies)", "NEAR/3", "kot:", "SearchText:kot", "MessageId:a", "col:kot", "^kot", "{kot}", "[kot]", "kot\"", "\"kot", "a OR", "OR a", "kot*pies", "foo*bar", "foo bar", "http", "https://example.com", "example.com/page?q=1", "example", "www", "youtube", "😀", "🇵🇱", "a😀b", "müller", "muller", "straße", "strasse", "STRASSE", "café", "cafe", "crème", "creme", "привет", "ПРИВЕТ", "日本語", "検索", "ǅ", "ß", "ﬁ", "İstanbul", "istanbul", "ıı", "ſ", "Σίσυφος", "ΣΑΣ", "12345", "2024", "Q1", "x²", "٣", "  kot  ", "k", "ko", "\t\tkot", "%", "%25", "kot%", "+", "kot+pies", "kot,pies", "kot;pies", "kot/pies", "kot\\pies", "kot pies", "kot pies", "kot​pies", "a" * 199, "a" * 200, "a" * 201, "kot " * 60, " ".join(f"w{n}" for n in range(30)), "\u0000kot", "kot\u0000", "kot\nnowa", "\"; DROP TABLE Messages; --", "' OR 1=1 --", "kot\") OR (\"", "kot\"* OR \"a", "\"*\"", "a\"b", "\"a\"\"b\"", "NEAR(\"a\" \"b\", 1)", "kot AND (pies OR", "�", "퟿", "￿", "ab́", "é", "ÆØÅ", "æøå", "ǰ", "ŉ", "ΐ", "ᾳ", "ǲ", "Ǳ", "ǳ", "ᲀ", "ꙁ", "Ａ", "ａｂ", "①", "Ⅷ", "²", "½", "௧", "ａ１"]
for _ in range(300):
    words = ["lampa", "kot", "raport", "łódź", "Zażółć", "ł", "Ł", "lo", "d", "dz", "pr", "wo", "ma", "he", "ex"]
    sp = ["\"", "*", ":", "(", ")", "-", "+", "^", " AND ", " OR ", " NOT ", " NEAR ", ",", ".", "'", "\\", "/", "{", "}"]
    parts = [random.choice(words) if random.random() < 0.6 else random.choice(sp) for _ in range(random.randint(1, 8))]
    qs.append("".join(parts) if random.random() < 0.5 else " ".join(parts))
for _ in range(120):
    qs.append("".join(chr(random.choice([random.randint(0x20, 0x17F), random.randint(0x370, 0x4FF), random.randint(0x3000, 0x30FF), random.randint(0x10000, 0x1F9FF)])) for _ in range(random.randint(2, 9))))
for q in qs:
    for cname, who in [("lounge", "boss"), ("general", "dawid")]:
        c = ch[cname]
        P.go("GET", f"/api/channels/{c}/messages/search?query={Q(q)}", who, quiet=True)
    P.go("GET", f"/api/channels/{ch['lounge']}/messages/index?limit=20&query={Q(q)}", "boss", quiet=True)
    P.go("GET", f"/api/channels/{ch['general']}/messages/index?kind=media&limit=30&query={Q(q)}", "boss", quiet=True)
# bad shapes
c = ch["general"]
for path in ["search", "search?query=", "search?query=a", "search?query=%20%20", "search?Query=lampa", "search?query=lampa&query=kot", "search?QUERY=kot",
             "index?limit=0", "index?limit=101", "index?limit=-1", "index?limit=abc", "index?limit=", "index?limit=%205", "index?limit=%2B5", "index?limit=5&limit=6",
             "index?kind=x", "index?kind=MEDIA", "index?kind=%20media%20", "index?kind=", "index?kind=ALL", "index?kind=İ", "index?query=a", "index?query=%20", "index?query=%20ab%20", "index?cursor=", "index?cursor=abc", "index?cursor=%00",
             "index?kind=media&cursor=" , "index?query=lampa&kind=links&limit=1000", "index?limit=abc&kind=x"]:
    P.go("GET", f"/api/channels/{c}/messages/{path}", "boss", quiet=True)
# cursors against another channel / kind / query
a = json.loads(P.o.go("GET", f"/api/channels/{c}/messages/index?limit=5", "boss")[1])["nextCursor"]
b = json.loads(P.r.go("GET", f"/api/channels/{c}/messages/index?limit=5", "boss")[1])["nextCursor"]
for (who, path) in [("boss", f"/api/channels/{ch['lounge']}/messages/index?limit=5&cursor={{}}"), ("boss", f"/api/channels/{c}/messages/index?limit=5&kind=media&cursor={{}}"), ("boss", f"/api/channels/{c}/messages/index?limit=5&query=lampa&cursor={{}}"), ("boss", f"/api/channels/{c}/messages/index?limit=5&cursor={{}}=")]:
    ra = P.o.go("GET", path.format(a), who); rb = P.r.go("GET", path.format(b), who)
    print("cursor misuse", ra[0], rb[0], ra[1][:100] == rb[1][:100] or (ra[1][:60], rb[1][:60]))
for who in ["boss", "dawid"]:
    for cname in ["secret", "general"]:
        for path in ["search?query=lampa", "index", "pins", "threads"]:
            P.go("GET", f"/api/channels/{ch[cname]}/messages/{path}" if path in ("index",) or path.startswith("search") else f"/api/channels/{ch[cname]}/{path}", who, quiet=True)
# unauthenticated and unknown
P.go("GET", f"/api/channels/{c}/messages/index", None, quiet=True)
P.go("GET", f"/api/channels/xx/messages/index", "boss", quiet=True)
P.go("GET", f"/api/channels/{c}/messages/index", "boss", headers={"X-Stellarr-Server": "xx"}, quiet=True)
P.go("GET", f"/api/channels/{c}/messages/index", "boss", headers={"X-Stellarr-Server": "00000000-0000-4000-8000-000000000000"}, quiet=True)
print("requests", P.count, "differences", P.diffs)
