"""The same inputs through the first server's code (the throwaway program in dn/) and through this server's."""
import base64, os, random, struct, subprocess, sys, uuid
HERE = os.path.dirname(os.path.abspath(__file__))
random.seed(int(sys.argv[1]) if len(sys.argv) > 1 else 3)
B = lambda s: base64.b64encode(s.encode("utf-8", "surrogatepass")).decode()
specials = list("\"*:()-+^{}[]\\'`~!@#$%&=<>?/|,.;_ \t\n  　​\u0085") + ["AND", "OR", "NOT", "NEAR", "NEAR(", "NEAR/2", "col:", "-word", "word*", "\"a b\"", "a OR", "(a", "a)", "^a"]
words = ["lampa", "Lampa", "LAMPA", "łódź", "Łódź", "lodz", "ŁÓDŹ", "zażółć", "gęślą", "jaźń", "kot", "KOT", "pies", "raport", "Müller", "straße", "STRASSE", "café", "Привет", "ПРИВЕТ", "привет", "日本語", "ǅ", "ǆ", "Ǆ", "ß", "SS", "ı", "I", "i", "İ", "ſ", "s", "ﬁ", "fi", "Σ", "σ", "ς", "ΣΑΣ", "ǈ", "12345", "٣٤", "Ⅻ", "½", "x²", "a1", "1a"]
def rand_char():
    r = random.random()
    if r < 0.5: return random.choice("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 ")
    if r < 0.7: return random.choice("ąćęłńóśźżĄĆĘŁŃÓŚŹŻäöüßÄÖÜéèêëçñ")
    if r < 0.8: return random.choice(specials) if len(random.choice(specials)) == 1 else " "
    if r < 0.9: return chr(random.choice([random.randint(0x80, 0x2FFF), random.randint(0x3000, 0xD7FF), random.randint(0xE000, 0xFFFD)]))
    return chr(random.randint(0x10000, 0x10FFFF))
def rand_text():
    parts = []
    for _ in range(random.randint(0, 12)):
        r = random.random()
        if r < 0.45: parts.append(random.choice(words))
        elif r < 0.7: parts.append(random.choice(specials))
        else: parts.append("".join(rand_char() for _ in range(random.randint(1, 8))))
    sep = random.choice([" ", " ", "", "  ", "\n", "+"])
    t = sep.join(parts)
    return t[: random.choice([10**6, 200, 40])]
cases = []
for _ in range(6000): cases.append(("Q", rand_text()))
for w in words: cases.append(("Q", w))
for _ in range(10): cases.append(("Q", " ".join(f"w{n}" for n in range(random.randint(14, 40)))))
cases.append(("Q", "a" * 5000))
for _ in range(1500): cases.append(("T", "".join(random.choice(["a", " ", "\t", " ", " ", "　", "\u001c", "\u001f", "​", "﻿", "\u0085", "x", "᠎"]) for _ in range(random.randint(0, 8)))))
for _ in range(1500): cases.append(("L", "".join(random.choice(["A", "Z", "İ", "I", "ı", "Σ", "ς", "ǅ", "ẞ", "ß", "Ω", "K", "Å", "Ł", "ŁÓDŹ", "MEDIA", "LiNkS", " ", "\t"]) for _ in range(random.randint(0, 6)))))
for t in ["5", " 5", "+5", "-5", "5 ", "\t5\n", "5\x00", "\x005", "", " ", "abc", "5,6", "5.0", "2147483647", "2147483648", "-2147483648", "-2147483649", "+", "-", "--5", "٣", "５", "0x10", "1e2", "5_0", " 5", "5 ", " 5", " +5 ", "+ 5", "- 5", "00000000000000000005", "+2147483647", "−5", "1 2"]: cases.append(("I", t))
for _ in range(800): cases.append(("I", "".join(random.choice("0123456789 +-\t\x00,.") for _ in range(random.randint(0, 12)))))
g = "24f08f99-6377-480c-afb8-c61fe5da5b36"
for t in [g, g.upper(), g.replace("-", ""), "{" + g + "}", "(" + g + ")", " " + g + " ", "{" + g, g + "}", g[:-1], g + "0", "{0x24f08f99,0x6377,0x480c,{0xaf,0xb8,0xc6,0x1f,0xe5,0xda,0x5b,0x36}}", "{0x1,0x2,0x3,{0x4,0x5,0x6,0x7,0x8,0x9,0xa,0xb}}", "0x24f08f99-6377-480c-afb8-c61fe5da5b36", g.replace("-", "_"), "", "xx", g[:8] + g[9:], "(" + g + "}", g.replace("a", "g"), "\t" + g, g + "\n", " " + g, "00000000-0000-0000-0000-000000000000"]: cases.append(("G", t))
for _ in range(500):
    h = list(g.replace("-", ""))
    for __ in range(random.randint(0, 3)): h[random.randrange(32)] = random.choice("0123456789abcdefABCDEFgG-{}() ")
    s = "".join(h)
    form = random.choice(["N", "D", "B", "P", "X"])
    if form == "D": s = f"{s[:8]}-{s[8:12]}-{s[12:16]}-{s[16:20]}-{s[20:]}"
    if form == "B": s = "{" + s + "}"
    if form == "P": s = "(" + s + ")"
    cases.append(("G", s))
def cursor(ch=g, ticks=639000000000000000, off=3, kind=1, h=12345):
    return uuid.UUID(ch).bytes_le + struct.pack(">q", ticks) + struct.pack(">i", off) + bytes([kind]) + struct.pack(">Q", h)
valid = base64.urlsafe_b64encode(cursor()).decode().rstrip("=")
cur_cases = [valid, valid + "==", valid + "=", valid + "===", valid + "A", valid[:-1], "", valid[:20] + "\t" + valid[20:], valid.replace("-", "+"), valid.replace("_", "/")]
for kind in [0, 1, 2, 3, 255]: cur_cases.append(base64.urlsafe_b64encode(cursor(kind=kind)).decode().rstrip("="))
for ticks in [0, -1, 1, 3155378975999999999, 3155378976000000000, 2**62, 2**63 - 1]: cur_cases.append(base64.urlsafe_b64encode(cursor(ticks=ticks)).decode().rstrip("="))
for off in [0, -1, 2**31 - 1, -2**31]: cur_cases.append(base64.urlsafe_b64encode(cursor(off=off)).decode().rstrip("="))
alpha = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
for _ in range(2500):
    s = list(valid)
    for __ in range(random.randint(0, 3)):
        r = random.random()
        i = random.randrange(len(s))
        if r < 0.5: s[i] = random.choice(alpha)
        elif r < 0.7: s[i] = random.choice("=+/\t.*")
        elif r < 0.85: del s[i]
        else: s.insert(i, random.choice(alpha + "="))
    t = "".join(s)
    if random.random() < 0.3: t += random.choice(["", "=", "==", "==="])
    cur_cases.append(t)
for t in cur_cases: cases.append(("D", t))
for _ in range(300): cases.append(("H", rand_text()))
for _ in range(1500):
    base = "".join(random.choice(["a", "b", "ż", " ", "\n", "\t", "😀", "é", " ", "​", "x" * 30]) for _ in range(random.randint(0, 30)))
    cases.append(("R", base * random.choice([1, 1, 3, 12])))
for pad in range(150, 165):
    for tail in ["😀", "é", "😀😀"]:
        cases.append(("R", "a" * pad + tail + " tail words here"))
cases.append(("CF", "")); cases.append(("LN", ""))
work = os.path.join(os.getcwd(), "work")
open(f"{work}/cases.txt", "w").write("\n".join(f"{k} {B(v) if k not in ('D',) else v}" for k, v in cases) + "\n")
dn = subprocess.run(["/Users/arkazz/.dotnet/dotnet", os.environ["DN_DLL"]], input="\n".join((f"{k} {B(v)}" if k not in ("D", "CF", "LN") else f"{k} {v}") for k, v in cases) + "\n", capture_output=True, text=True)
theirs = dn.stdout.split("\n")
# the first server's program prints several lines for CF; split by cases
open(f"{work}/dn-answers.txt", "w").write(dn.stdout)
print("cases", len(cases), "dn lines", len(theirs))
