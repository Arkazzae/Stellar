"""After cmp_write.py: what each server left in the rows."""
import sqlite3, subprocess, sys, os
sys.path.insert(0, '.')
import harness
from collections import Counter
subprocess.run(["lsof", "-tiTCP:" + harness.RUST_PORT, "-sTCP:LISTEN"], capture_output=True)
for pid in subprocess.run(["lsof", "-tiTCP:" + harness.RUST_PORT, "-sTCP:LISTEN"], capture_output=True, text=True).stdout.split(): os.kill(int(pid), 15)
import time; time.sleep(2)
subprocess.run(["docker", "stop", "-t", "15", harness.CONTAINER], check=True, capture_output=True)
subprocess.run(["docker", "cp", harness.CONTAINER + ":/tmp/data/stellarr.db", "work/oracle-after.db"], check=True)
def rows(path):
    db = sqlite3.connect(path)
    return {
        "reactions": Counter(db.execute('SELECT MessageId, UserId, Emoji FROM MessageReactions').fetchall()),
        "pinned": Counter(db.execute('SELECT Id FROM Messages WHERE PinnedAt IS NOT NULL').fetchall()),
        "changes": Counter(db.execute('SELECT ChannelId, MessageId, Kind FROM MessageChanges').fetchall()),
        "messages": Counter(db.execute('SELECT Id, ChannelId, Content, EditedAt FROM Messages').fetchall()),
        "index": Counter(db.execute('SELECT MessageId, SearchText, HasAttachments, HasLinks FROM MessageIndexEntries').fetchall()),
        "audit": Counter(db.execute('SELECT COUNT(*) FROM ServerAuditEvents').fetchall()) if db.execute("select 1 from sqlite_master where name='ServerAuditEvents'").fetchone() else Counter(),
    }
a, b = rows("work/oracle-after.db"), rows("work/rust/stellarr.db")
for k in a:
    print(k, "same" if a[k] == b[k] else f"DIFFERENT: only oracle {sum((a[k]-b[k]).values())}, only rust {sum((b[k]-a[k]).values())}", "rows", sum(a[k].values()))
