"""Phase B: thousands of messages in several languages, written straight into a copy of the database
(the triggers fill the journal and the search index), with the awkward cases the routes meet:
equal moments, a pinned list over its limit, attachments out of id order, embeds, reactions."""
import sqlite3, sys, uuid, random, datetime, json
src, dst = sys.argv[1], sys.argv[2]
random.seed(11)
import shutil
shutil.copy(src, dst)
db = sqlite3.connect(dst)
db.execute("PRAGMA foreign_keys=ON")
state = json.load(open("state.json"))
users = [r[0] for r in db.execute('SELECT "Id" FROM "Users" ORDER BY "Username"')]
ch = {k: v.upper() for k, v in state["channels"].items()}
def gid(): return str(uuid.uuid4()).upper()
def ts(dt):
    micro = dt.microsecond
    base = dt.strftime("%Y-%m-%d %H:%M:%S")
    return base + ("." + f"{micro:06d}0".rstrip("0") if micro else "")
pl = "dzień dobry wszystkim kot pies lampa łódź żółw gęś jaźń zażółć gęślą raport faktura umowa spotkanie jutro wieczorem kawa herbata śniadanie obiad kolacja książka film muzyka wakacje morze góry las rzeka miasto wieś szkoła praca dom rodzina przyjaciel łatwo trudno szybko wolno często rzadko czasem zawsze nigdy Łukasz Ślązak Żaneta Ćma Ósemka".split()
en = "hello world quick brown fox jumps over lazy dog report invoice meeting tomorrow evening coffee tea breakfast lunch dinner book movie music holiday sea mountains forest river city village school work home family friend easy hard fast slow often rarely sometimes always never lamp lantern ball bell".split()
de = "Müller grüßt die Straße Größe Äpfel Öl über schön Käse Mädchen Fuß weiß heiß".split()
fr = "café crème brûlée français élève garçon où déjà naïve Noël œuvre".split()
ru = "привет мир как дела спасибо пожалуйста хорошо плохо дом работа".split()
ja = ["日本語", "検索", "テキスト", "こんにちは", "世界"]
cs = "příliš žluťoučký kůň úpěl ďábelské ódy".split()
langs = [pl, pl, pl, en, en, de, fr, ru, ja, cs]
urls = ["https://example.com/a/b?x=1", "http://example.org/zażółć", "https://www.youtube.com/watch?v=abc", "https://lampa.example/ł"]
extras = ["😀", "🇵🇱", "👍🏽", "a-b_c", "(nawias)", "foo*bar", "NOT", "AND", "OR", "NEAR", "\"cytat\"", "12345", "2024", "ǅ", "ß", "ﬁ", "İstanbul", "ıı", "ſ", "ǆ", "Ω", "Σίσυφος", "ΣΑΣ", "ǈ"]
def sentence():
    lang = random.choice(langs)
    words = [random.choice(lang) for _ in range(random.randint(2, 9))]
    if random.random() < 0.15: words.append(random.choice(extras))
    if random.random() < 0.06: words.append(random.choice(urls))
    if random.random() < 0.05: words.insert(random.randint(0, len(words)), random.choice(lang).upper())
    text = " ".join(words)
    if random.random() < 0.05: text += "\nnowa linia\t tab"
    return text
now = datetime.datetime(2026, 11, 20, 12, 0, 0)
def bulk(channel_key, count, start_days, ties=True):
    channel = ch[channel_key]
    made = []
    t = now - datetime.timedelta(days=start_days)
    last = t
    for n in range(count):
        if ties and made and random.random() < 0.1:
            t = last
        else:
            t = last + datetime.timedelta(seconds=random.randint(1, 3600), microseconds=random.choice([0, 0, 123456, 500000, 999999, 100]))
        last = t
        mid = gid()
        author = random.choice(users)
        reply = random.choice(made)[0] if made and random.random() < 0.08 else None
        if made and random.random() < 0.02: reply = made[0][0]  # a root with many replies
        system = 1 if random.random() < 0.01 else 0
        edited = ts(t + datetime.timedelta(minutes=3)) if random.random() < 0.05 else None
        content = sentence() if not system else "joined the server"
        db.execute('INSERT INTO "Messages" ("Id","ChannelId","AuthorId","IsSystem","Content","CreatedAt","EditedAt","PinnedAt","ReplyToMessageId") VALUES (?,?,?,?,?,?,?,NULL,?)',
                   (mid, channel, author, system, content, ts(t), edited, reply))
        made.append((mid, t, author))
    return made
general = bulk("general", 1500, 25)
lounge = bulk("lounge", 2500, 25)
noreact = bulk("noreact", 300, 20)
# a pinned list over its limit, with equal pin times
pins = random.sample(lounge, 57)
for i, (mid, t, _) in enumerate(pins):
    pt = now - datetime.timedelta(hours=i // 3)
    db.execute('UPDATE "Messages" SET "PinnedAt"=? WHERE "Id"=?', (ts(pt), mid))
# attachments, out of id order
def attach(mid, author, count):
    ids = sorted(gid() for _ in range(count))
    random.shuffle(ids)
    for i, aid in enumerate(ids):
        kind = random.choice(["image/png", "video/mp4", "audio/ogg", "application/pdf"])
        voice = 1 if kind == "audio/ogg" and random.random() < 0.5 else 0
        db.execute('INSERT INTO "ImageAttachments" ("Id","UploadedById","ObjectKey","FileName","ContentType","SizeBytes","Width","Height","DurationSeconds","IsVoiceMessage","PosterObjectKey","PosterContentType","PosterSizeBytes","PreviewObjectKey","PreviewContentType","PreviewSizeBytes","Placeholder","IsEncrypted","CreatedAt") VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,0,?)',
                   (aid, author, "k/" + aid, random.choice(["zdjęcie żółwia.png", "raport końcowy.pdf", "Foo & <bar>.mp4", "note.ogg", "ścieżka.png"]), kind,
                    random.randint(100, 10**7), random.choice([None, 640]), random.choice([None, 480]), random.choice([None, 3.0, 3.5, 12.25, 0.1]), voice,
                    random.choice([None, "p/" + aid]), None, None, random.choice([None, "v/" + aid]), None, None, random.choice([None, "LEHV6nWB2yk8pyo0adR*.7kCMdnj"]), ts(now)))
        db.execute('INSERT INTO "MessageAttachments" ("MessageId","AttachmentId") VALUES (?,?)', (mid, aid))
for mid, t, author in random.sample(general + lounge, 150):
    attach(mid, author, random.choice([1, 1, 2, 3, 4]))
# embeds, with equal positions
for mid, t, author in random.sample(general + lounge, 120):
    for pos in sorted(random.choice([0, 0, 1, 2]) for _ in range(random.randint(1, 3))):
        kind = random.choice([0, 1])
        db.execute('INSERT INTO "MessageEmbeds" ("Id","MessageId","Url","Kind","Provider","ProviderId","StartSeconds","Domain","Title","Description","ThumbnailAttachmentId","Position","CreatedAt") VALUES (?,?,?,?,?,?,?,?,?,?,NULL,?,?)',
                   (gid(), mid, random.choice(urls), kind, "youtube" if kind else "example.com", "abc" if kind else None, random.choice([None, 30]), "example.com",
                    random.choice([None, "Tytuł żółwia", "Lampa & <b>"]), random.choice([None, "Opis łódź"]), pos, ts(now)))
# reactions, some with equal moments across emoji
emojis = ["👍", "❤️", "😂", "🇵🇱", "👍🏽", "👨‍👩‍👧", "🎉", "ok", "<:blob:123>", "a", "ż"]
for mid, t, _ in random.sample(general + lounge + noreact, 400):
    base = t + datetime.timedelta(seconds=random.randint(1, 100))
    for u in random.sample(users, random.randint(1, 5)):
        e = random.choice(emojis)
        at = base if random.random() < 0.5 else base + datetime.timedelta(milliseconds=random.randint(1, 5000))
        db.execute('INSERT OR IGNORE INTO "MessageReactions" ("MessageId","UserId","Emoji","CreatedAt") VALUES (?,?,?,?)', (mid, u, e, ts(at)))
db.commit()
db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
db.close()
print("messages", sqlite3.connect(dst).execute('select count(*) from "Messages"').fetchone(), "index", sqlite3.connect(dst).execute('select count(*) from "MessageIndexEntries"').fetchone())
# The messages the event transcript acts on.
db2 = sqlite3.connect(dst)
def one(channel, where="1=1", off=0):
    return db2.execute(f'SELECT Id FROM Messages WHERE ChannelId=? AND IsSystem=0 AND {where} ORDER BY CreatedAt LIMIT 1 OFFSET ?', (state["channels"][channel].upper(), off)).fetchone()[0].lower()
json.dump({"g0": one("general"), "g1": one("general", off=1), "s0": one("secret"), "n0": one("noreact"), "l0": one("lounge", "PinnedAt IS NULL")}, open("work/ids.json", "w"))
