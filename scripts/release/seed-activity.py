"""Load an SDK-packed activity into the empty library of the disposable release runner."""
import hashlib
import json
from pathlib import Path
import sqlite3
import sys
import uuid
import zipfile

release, database = map(Path, sys.argv[1:])
entry, = json.loads((release / "stellarr-release.json").read_bytes())["activities"]
archive = release / entry["asset"]
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
if digest != entry["sha256"] or archive.stat().st_size != entry["bytes"]:
    raise ValueError("SDK release index differs from ZIP")
with zipfile.ZipFile(archive) as packed:
    files = {name: packed.read(name) for name in packed.namelist() if not name.endswith("/")}
manifest = json.loads(files["stellarr.json"])
with sqlite3.connect(database.resolve(strict=True).as_uri() + "?mode=rw", uri=True) as db:
    db.execute("PRAGMA foreign_keys = ON")
    db.execute("BEGIN IMMEDIATE")
    if db.execute("SELECT 1 FROM ActivityInstalls LIMIT 1").fetchone():
        raise ValueError("Smoke test requires an empty library")
    install = str(uuid.uuid4()).upper()
    source = "https://github.com/Arkazzae/Stellarr"
    db.execute("INSERT INTO ActivityInstalls(Id,Source,Activity,Digest,Enabled) VALUES(?,?,?,?,1)",
               (install, source, manifest["id"], digest))
    db.execute("INSERT INTO ActivityVersions(Install,Digest,Revision,Release,Manifest,Bytes) VALUES(?,?,?,?,?,?)",
               (install, digest, manifest["version"], source, json.dumps(manifest), entry["bytes"]))
    db.executemany("INSERT INTO ActivityFiles(Install,Digest,Path,Content) VALUES(?,?,?,?)",
                   [(install, digest, path, body) for path, body in files.items()])
