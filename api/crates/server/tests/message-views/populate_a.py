"""Phase A: people, channels, overrides, and some hundreds of messages made through the first server's own routes."""
import sys, json, random, subprocess, re
sys.path.insert(0, '.')
from lib import *
import os
B = "http://127.0.0.1:" + os.environ.get("ORACLE_PORT", "5215")
random.seed(7)
logs = subprocess.run(["docker", "logs", "stellarr-port-oracle-" + os.environ.get("ORACLE", "msgviews") + ""], capture_output=True, text=True)
setup = re.findall(r"Setup code: ([A-Z0-9-]+)\.", logs.stdout + logs.stderr)[-1]
s, body, _ = call(B, "POST", "/api/auth/register", {"username": "boss", "displayName": "Boss", "email": "boss@example.test", "password": PASSWORD, "setupCode": setup})
assert s == 201, (s, body)
people = {}
cookie, uid = login(B, "boss"); people["boss"] = (cookie, uid)
s, body, _ = call(B, "POST", "/api/admin/invites", {}, cookie)
invite = json.loads(body)["url"].rsplit("/", 1)[1]
for name, display in [("anna", "Anna Łączna"), ("bartek", "Bartek"), ("celina", "Celina Żółć"), ("dawid", "Dawid")]:
    s, body, _ = call(B, "POST", "/api/auth/register", {"username": name, "displayName": display, "email": f"{name}@example.test", "password": PASSWORD, "inviteCode": invite})
    assert s == 201, (s, body)
    people[name] = login(B, name)
boss = people["boss"][0]
s, body, _ = call(B, "GET", "/api/bootstrap", None, boss)
boot = json.loads(body)
server = boot["server"]["id"]
channels = {c["name"]: c["id"] for c in boot["channels"]}
for name in ["lounge", "secret", "noreact"]:
    s, body, _ = call(B, "POST", "/api/admin/channels", {"name": name, "kind": "text"}, boss)
    assert s in (200, 201), (s, body)
    channels[name] = json.loads(body)["id"]
s, body, _ = call(B, "PUT", f"/api/admin/channels/{channels['secret']}/permissions/members/{people['dawid'][1]}", {"allow": [], "deny": ["viewChannel"]}, boss)
print("secret override", s)
s, body, _ = call(B, "PUT", f"/api/admin/channels/{channels['noreact']}/permissions/members/{people['anna'][1]}", {"allow": [], "deny": ["addReactions"]}, boss)
print("noreact override", s)
texts = [
 "Dzień dobry wszystkim, jak się macie?", "Zażółć gęślą jaźń", "Łódź to piękne miasto", "lampa na biurku się zepsuła",
 "The quick brown fox jumps over the lazy dog", "Raport kwartalny: raport.pdf gotowy", "Hello world", "Müller grüßt die Straße",
 "Café crème brûlée à la française", "Привет мир, как дела?", "日本語のテキスト 検索", "https://example.com/page?q=1 sprawdź to",
 "Look at http://example.org/a-b_c.html please", "łazienka, łóżko, ślimak, świnia", "pies i kot", "KOT w butach", "Kot",
 "2024 plan: Q1 Q2 Q3 Q4", "numer 12345 oraz 67890", "NOT AND OR NEAR", "a OR b", "foo*bar", 'cytat "w cudzysłowie" tutaj', "co (to) jest: to",
 "emoji 😀 w tekście 🇵🇱", "wielokrotnie wielokrotnie wielokrotnie powtarzane słowo", "krótko", "x", "Strażnik i żółw",
 "ffmpeg is a tool", "<script>alert('x')</script> & friends", "line one\nline two\n\nline four", "   leading and trailing   ",
]
def fresh(who, n):
    ids = []
    for k in range(n):
        s_, body = upload(B, people[who][0], f"zdjęcie {who} {random.randint(0, 10**6)}.png", png(16 + k * 3, 12 + k, random.randint(0, 200)))
        assert s_ == 201, (s_, body)
        ids.append(json.loads(body)["id"])
    return ids
names = list(people)
msgs = {"general": [], "lounge": [], "secret": [], "noreact": []}
def post(who, channel, content, reply=None, attach=None):
    payload = {"content": content}
    if reply: payload["replyToMessageId"] = reply
    if attach: payload["attachmentIds"] = attach
    s, body, _ = call(B, "POST", f"/api/channels/{channels[channel]}/messages", payload, people[who][0])
    assert s == 201, (s, body[:200])
    m = json.loads(body)["id"]
    msgs[channel].append(m)
    return m
for n in range(160):
    ch = random.choice(["general"] * 6 + ["lounge", "noreact"])
    who = random.choice(names if ch != "secret" else names[:4])
    content = random.choice(texts) + (f" #{n}" if n % 3 == 0 else "")
    reply = random.choice(msgs[ch]) if msgs[ch] and n % 5 == 0 else None
    attach = None
    if n % 17 == 0:
        attach = fresh(who, random.choice([1, 2, 3, 4]))
    post(who, ch, content, reply, attach)
for n in range(12):
    post(random.choice(names[:4]), "secret", random.choice(texts))
# a root with many replies
root = post("boss", "general", "Wątek główny: co robimy dziś wieczorem?")
for n in range(7):
    post(random.choice(names), "general", f"odpowiedź {n} w wątku", reply=root)
emojis = ["👍", "❤️", "😂", "🇵🇱", "👍🏽", "👨‍👩‍👧", "🎉", "ok", "<:blob:123>"]
reacted = 0
for ch in ["general", "lounge", "noreact"]:
    for m in random.sample(msgs[ch], min(25, len(msgs[ch]))):
        for who in random.sample(names, random.randint(1, 4)):
            e = random.choice(emojis)
            s, _, _ = call(B, "PUT", f"/api/messages/{m}/reactions/{urllib.parse.quote(e)}", None, people[who][0])
            reacted += s == 200
print("reactions", reacted)
pinned = 0
for m in random.sample(msgs["general"], 9):
    s, _, _ = call(B, "PUT", f"/api/messages/{m}/pin", None, boss); pinned += s == 200
print("pins", pinned)
json.dump({"people": people, "server": server, "channels": channels, "msgs": msgs, "root": root}, open("state.json", "w"), indent=1)
print({k: len(v) for k, v in msgs.items()})
