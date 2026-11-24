"""Helpers of backups-archives.sh: the data set, the comparison of two
archives, the comparison of restored trees, and the hostile archives.

  python3 -I backups-archives.py fill SEED_DIR          fill every table, add files
  python3 -I backups-archives.py layout SEED_DIR ROOT   lay the seed out as the entrypoint does
  python3 -I backups-archives.py compare FIRST.zip OURS.zip
  python3 -I backups-archives.py tree SOURCE_ROOT RESTORED_ROOT
  python3 -I backups-archives.py hostile OUT_DIR        write the hostile archives
"""
import hashlib, json, os, shutil, sqlite3, struct, sys, tempfile, uuid, zipfile, zlib

def fail(message):
    print("DIFFERENCE:", message)
    global failures
    failures += 1

failures = 0
NOW = "2026-11-24 00:00:00"

# ---------------------------------------------------------------------------
# The data set.

def fill(seed):
    db = os.path.join(seed, "stellarr.db")
    c = sqlite3.connect(db)
    c.execute("PRAGMA foreign_keys=ON")
    tables = [r[0] for r in c.execute("select name from sqlite_master where type='table' and name not like 'sqlite_%' order by 1")]
    def count(t): return c.execute(f'select count(*) from "{t}"').fetchall()[0][0]
    def row_of(t, col):
        found = c.execute(f'select "{col}" from "{t}" limit 1').fetchall(); return found[0] if found else None
    stuck = {}
    for _ in range(6):
        for t in tables:
            if count(t): continue
            cols = c.execute(f'pragma table_info("{t}")').fetchall()
            fks = {r[3]: (r[2], r[4]) for r in c.execute(f'pragma foreign_key_list("{t}")')}
            values = {}
            ok = True
            for _, name, typ, notnull, default, pk in cols:
                if name in fks:
                    ref, refcol = fks[name]
                    got = row_of(ref, refcol or c.execute(f'pragma table_info("{ref}")').fetchall()[0][1])
                    if not got:
                        if notnull: ok = False
                        continue
                    values[name] = got[0]
                elif default is not None or (not notnull and not pk):
                    continue
                elif typ.upper().startswith("INT"):
                    values[name] = 0 if "Enabled" in name or name.startswith("Is") else 1
                elif typ.upper() == "BLOB":
                    values[name] = b"\x00\x01"
                elif name.endswith("At") or name.endswith("Date"):
                    values[name] = NOW
                elif name.endswith("Id") or name == "Id":
                    values[name] = str(uuid.uuid4()).upper()
                else:
                    values[name] = "x" + uuid.uuid4().hex[:6]
            if not ok: continue
            names = ", ".join(f'"{n}"' for n in values)
            try:
                c.execute(f'insert into "{t}" ({names}) values ({", ".join("?" * len(values))})', list(values.values()))
                c.commit()
            except sqlite3.Error as e:
                stuck[t] = str(e)
    if not count("MemberPermissionOverrides"):
        # Its foreign key is over two columns, which the generic filler does not follow.
        c.execute('insert into "MemberPermissionOverrides" ("Id","ServerId","TargetType","TargetId","UserId","AllowMask","DenyMask") '
                  'select ?, "ServerId", 0, "ServerId", "UserId", 1, 0 from "ServerMembers" limit 1', [str(uuid.uuid4()).upper()])
        c.commit()
    left = [t for t in tables if not count(t)]
    c.commit(); c.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchall()
    c.close()
    for suffix in ("-wal", "-shm"):
        try: os.remove(db + suffix)
        except FileNotFoundError: pass
    print("tables:", len(tables), "still empty:", {t: stuck.get(t, "no row to point at") for t in left})
    # Files.
    def put(path, data):
        full = os.path.join(seed, path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        open(full, "wb").write(data)
    png = b"\x89PNG\r\n\x1a\n" + b"p" * 40
    for folder in ["images", "audio", "video", "files", "encrypted", "emoji", "embeds", "sounds", "avatars", "banners", "servers/x/icon"]:
        put(f"files/{folder}/2026/11/{uuid.uuid4().hex}.bin", png + folder.encode())
    put("files/video/2026/11/big.mp4", os.urandom(3 * 1024 * 1024))
    for i in range(200):
        put(f"files/small/{i % 7}/{i:03}.txt", f"small {i}\n".encode() * (1 + i % 5))
    put("files/ünï/cødé-ü.txt", "non-ascii name".encode())
    put("files/zero/empty.bin", b"")
    put("keys/extra-key.xml", b"<key/>")
    put("secrets/turn-shared-secret", b"turn-secret-value")
    put("secrets/media/control", b"media-control")
    put("tls/privkey.pem", b"private key")
    put("tls/fullchain.pem", b"chain")
    put("releases/a-b/x", b"1")
    put("releases/a/x", b"2")
    put("releases/desktop/1.0/app.AppImage", os.urandom(1024))
    put("releases/ö/x", b"umlaut")

def layout(seed, root):
    os.makedirs(os.path.join(root, "db"))
    shutil.copy(os.path.join(seed, "stellarr.db"), os.path.join(root, "db", "stellarr.db"))
    for folder in ["files", "keys", "secrets", "tls", "releases"]:
        if os.path.isdir(os.path.join(seed, folder)):
            shutil.copytree(os.path.join(seed, folder), os.path.join(root, folder))
    # A staging folder of the object store is no object: neither server lists it.
    shutil.rmtree(os.path.join(root, "files", ".incoming"), ignore_errors=True)

# ---------------------------------------------------------------------------
# Comparing two archives.

def entries(path):
    with zipfile.ZipFile(path) as z:
        return [(i.filename, i.compress_type, i.file_size, i.compress_size, i.CRC) for i in z.infolist()]

def rows(db):
    c = sqlite3.connect(db)
    out = {}
    for (t,) in c.execute("select name from sqlite_master where type='table' and name not like 'sqlite_%' order by 1"):
        out[t] = c.execute(f'select count(*) from "{t}"').fetchone()[0]
    verdict = c.execute("pragma integrity_check").fetchone()[0]
    c.close()
    return out, verdict

def compare(first, ours, mode=""):
    # `login`: the first server was signed into after its data was copied, so its
    # database holds that session and its event, and is a little larger.
    allowed = {"SecurityEvents": 1, "UserSessions": 1} if mode == "login" else {}
    a, b = entries(first), entries(ours)
    za, zb = zipfile.ZipFile(first), zipfile.ZipFile(ours)
    print(f"entries: first server {len(a)}, ours {len(b)}")
    if [e[0] for e in a] != [e[0] for e in b]:
        fail("the entry names or their order differ")
        sa, sb = [e[0] for e in a], [e[0] for e in b]
        print("  only in first:", [n for n in sa if n not in sb][:10], "only in ours:", [n for n in sb if n not in sa][:10])
        for i, (x, y) in enumerate(zip(sa, sb)):
            if x != y:
                print("  first difference at", i, x, y); break
    by = {e[0]: e for e in b}
    compressed_a = compressed_b = 0
    for e in a:
        n = e[0]
        if n not in by: continue
        o = by[n]
        compressed_a += e[3]; compressed_b += o[3]
        if n in ("manifest.json", "database/stellarr.db"): continue
        if e[1] != o[1]: fail(f"{n}: method {e[1]} vs {o[1]}")
        if e[2] != o[2]: fail(f"{n}: size {e[2]} vs {o[2]}")
        if e[4] != o[4]: fail(f"{n}: crc differs")
    print(f"compressed bytes: first {compressed_a}, ours {compressed_b}")
    ma = json.loads(za.read("manifest.json")); mb = json.loads(zb.read("manifest.json"))
    if list(ma) != list(mb): fail(f"manifest members differ: {list(ma)} vs {list(mb)}")
    for k in ma:
        if k == "createdAt" or (mode == "login" and k == "databaseBytes"): continue
        if ma[k] != mb.get(k): fail(f"manifest {k}: {ma[k]!r} vs {mb.get(k)!r}")
    print("manifest:", {k: v for k, v in ma.items() if k != "missingObjects"}, "missing:", len(ma["missingObjects"]), len(mb["missingObjects"]))
    tmp = tempfile.mkdtemp()
    try:
        pa, pb = os.path.join(tmp, "a.db"), os.path.join(tmp, "b.db")
        open(pa, "wb").write(za.read("database/stellarr.db")); open(pb, "wb").write(zb.read("database/stellarr.db"))
        ra, va = rows(pa); rb, vb = rows(pb)
        print("database: integrity", va, vb, "tables", len(ra), len(rb), "empty tables", [t for t, n in ra.items() if not n])
        if va != "ok" or vb != "ok": fail("integrity_check")
        extra = [(t, ra[t], rb.get(t)) for t in ra if ra[t] != rb.get(t) + allowed.get(t, 0)]
        if extra: fail(f"row counts differ: {extra}")
        same = open(pa, "rb").read() == open(pb, "rb").read()
        print("database bytes equal:", same)
        if not same:
            ha = hashlib.sha256(open(pa, "rb").read()).hexdigest()[:12]; hb = hashlib.sha256(open(pb, "rb").read()).hexdigest()[:12]
            print("  sha256", ha, hb, "sizes", os.path.getsize(pa), os.path.getsize(pb))
    finally:
        shutil.rmtree(tmp)

# ---------------------------------------------------------------------------
# A restored tree against its source.

def tree_files(root, skip):
    out = {}
    for folder in ["files", "keys", "secrets", "tls", "releases"]:
        base = os.path.join(root, folder)
        for d, _, names in os.walk(base):
            for n in names:
                p = os.path.join(d, n)
                rel = os.path.relpath(p, root)
                if rel.startswith(skip): continue
                out[rel] = hashlib.sha256(open(p, "rb").read()).hexdigest()
    return out

def tree(source, restored, archive=None):
    a = tree_files(source, "files/.incoming")
    b = tree_files(restored, "files/.incoming")
    for k in sorted(set(a) | set(b)):
        if a.get(k) != b.get(k): fail(f"{k}: source {a.get(k, 'absent')[:8]} restored {b.get(k, 'absent')[:8]}")
    print("files compared:", len(a), "restored:", len(b))
    source_db = os.path.join(source, "db/stellarr.db")
    if archive:
        # The first server went on writing (a sign-in) between the data being
        # copied and its archive being made: its own database is the witness.
        source_db = os.path.join(tempfile.mkdtemp(), "a.db")
        open(source_db, "wb").write(zipfile.ZipFile(archive).read("database/stellarr.db"))
    ra, va = rows(source_db); rb, vb = rows(os.path.join(restored, "db/stellarr.db"))
    print("restored database integrity:", vb, "tables", len(rb))
    if vb != "ok": fail("restored integrity_check")
    if ra != rb: fail(f"restored row counts differ: {[(t, ra[t], rb.get(t)) for t in ra if ra[t] != rb.get(t)]}")
    for f in ["keys", "secrets", "tls"]:
        for d, _, names in os.walk(os.path.join(restored, f)):
            for n in names:
                mode = os.stat(os.path.join(d, n)).st_mode & 0o777
                if mode != 0o600: fail(f"{os.path.join(f, n)} has mode {oct(mode)}")
    previous = [d for d in os.listdir(os.path.join(restored, "restore")) if d.startswith("previous-")]
    if len(previous) != 1: fail(f"previous data: {previous}")
    else: print("previous data kept aside:", previous[0], sorted(os.listdir(os.path.join(restored, "restore", previous[0]))))

# ---------------------------------------------------------------------------
# Hostile archives, written by hand so that any name and any field can be said.

MANIFEST = {"format": "stellarr-backup", "version": 1, "createdAt": "2026-11-24T10:00:00.0000000Z", "instanceName": "x",
            "parts": ["database", "objects", "keys", "instance"], "databaseBytes": 0, "objectCount": 0, "objectBytes": 0,
            "missingObjects": [], "keyFiles": 0, "environmentSecrets": [], "instanceFiles": 0, "instanceFiles": 0}

def writer(entries):
    """entries: (name bytes, data, method, mode, declared_size|None, crc|None)"""
    out = b""; central = b""
    for name, data, method, mode, declared, crc in entries:
        raw = data if method == 0 else zlib.compressobj(6, zlib.DEFLATED, -15).compress(data)
        if method == 8:
            c = zlib.compressobj(6, zlib.DEFLATED, -15); raw = c.compress(data) + c.flush()
        crc = zlib.crc32(data) if crc is None else crc
        size = len(data) if declared is None else declared
        local = struct.pack("<IHHHHHIIIHH", 0x04034b50, 20, 0x0800, method, 0, 0x21, crc, len(raw), size, len(name), 0) + name
        central += struct.pack("<IHHHHHHIIIHHHHHII", 0x02014b50, (3 << 8) | 20, 20, 0x0800, method, 0, 0x21, crc, len(raw), size,
                               len(name), 0, 0, 0, 0, (mode or 0) << 16, len(out)) + name
        out += local + raw
    end = struct.pack("<IHHHHIIH", 0x06054b50, 0, 0, len(entries), len(entries), len(central), len(out), 0)
    return out + central + end

def small_db():
    t = tempfile.mkdtemp()
    p = os.path.join(t, "s.db")
    c = sqlite3.connect(p); c.execute("create table t (x)"); c.execute("insert into t values (1)"); c.commit(); c.close()
    data = open(p, "rb").read(); shutil.rmtree(t); return data

def hostile(out):
    os.makedirs(out, exist_ok=True)
    db = small_db()
    def man(**kw):
        m = dict(MANIFEST); m.update(kw); return json.dumps(m).encode()
    def E(name, data=b"d", method=0, mode=None, declared=None, crc=None):
        return (name if isinstance(name, bytes) else name.encode(), data, method, mode, declared, crc)
    base = lambda *extra, **m: [E("database/stellarr.db", db, 8)] + list(extra) + [E("manifest.json", man(**m), 8)]
    cases = {}
    for label, name in [("climb-out", "objects/../../evil"), ("climb-once", "objects/../evil"), ("absolute", "/etc/evil"),
                        ("drive-letter", "C:\\evil"), ("backslash-key", "objects/a\\b"), ("keys-subfolder", "keys/sub/x"),
                        ("instance-other-folder", "instance/other/x"), ("instance-climb", "instance/secrets/../../evil"),
                        ("unknown-top-level", "x"), ("dot-key", "objects/.hidden"), ("empty-segment", "objects//a"),
                        ("folder-entry", "objects/")]:
        cases[label] = writer(base(E(name), objectCount=1 if name.startswith("objects/") else 0))
    cases["nul-in-name"] = writer(base(E(b"objects/a\x00b"), objectCount=1))
    cases["symlink-entry"] = writer(base(E("objects/link", b"/etc/passwd", 0, 0o120777), objectCount=1))
    cases["device-entry"] = writer(base(E("objects/dev", b"", 0, 0o020666), objectCount=1))
    cases["duplicate-names"] = writer(base(E("objects/a", b"one"), E("objects/a", b"two"), objectCount=2))
    cases["size-lies-larger"] = writer(base(E("objects/a", b"hello", 0, None, 5_000_000), objectCount=1))
    cases["size-lies-smaller"] = writer(base(E("objects/a", b"hello", 0, None, 2), objectCount=1))
    cases["gigabytes-in-a-tiny-file"] = writer(base(E("objects/a", b"hello", 8, None, 0xfffffffe), objectCount=1))
    cases["wrong-crc"] = writer(base(E("objects/a", b"hello", 0, None, None, 0x12345678), objectCount=1))
    good = writer(base(E("objects/a", b"hello"), objectCount=1))
    cases["truncated-half"] = good[: len(good) // 2]
    cases["truncated-tail"] = good[:-30]
    cases["empty-file"] = b""
    cases["db-not-a-database"] = writer([E("database/stellarr.db", b"this is not a database, not even close", 8), E("manifest.json", man(), 8)])
    cases["db-damaged"] = writer([E("database/stellarr.db", db[: len(db) // 2] + b"\xa5" * (len(db) - len(db) // 2), 8), E("manifest.json", man(), 8)])
    cases["db-only-file"] = db
    cases["partial-parts"] = writer([E("database/stellarr.db", db, 8), E("manifest.json", man(parts=["database"]), 8)])
    cases["wrong-format"] = writer([E("database/stellarr.db", db, 8), E("manifest.json", man(format="other"), 8)])
    cases["newer-version"] = writer([E("database/stellarr.db", db, 8), E("manifest.json", man(version=2), 8)])
    cases["no-manifest"] = writer([E("database/stellarr.db", db, 8)])
    cases["counts-lie"] = writer(base(E("objects/a", b"hello"), objectCount=5))
    cases["valid-minimal"] = writer(base(E("objects/a", b"hello"), E("keys/k", b"k"), E("instance/secrets/s", b"s"),
                                        objectCount=1, keyFiles=1, instanceFiles=1))
    for name, data in cases.items():
        open(os.path.join(out, name + ".zip"), "wb").write(data)
    print(" ".join(sorted(cases)))

if __name__ == "__main__":
    command = sys.argv[1]
    {"fill": lambda: fill(sys.argv[2]), "layout": lambda: layout(sys.argv[2], sys.argv[3]),
     "compare": lambda: compare(*sys.argv[2:5]), "tree": lambda: tree(*sys.argv[2:5]),
     "hostile": lambda: hostile(sys.argv[2])}[command]()
    sys.exit(1 if failures else 0)
