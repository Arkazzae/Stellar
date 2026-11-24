"""Reads back from Mailpit what the crate sent, over Mailpit's HTTP API.

Usage: python3 -I verify_mailpit.py DIR
DIR holds one folder per security mode. Each has `http` (host:port of that
Mailpit), and NAME.eml / NAME.expect for every message sent.
"""
import email
import email.policy
import json
import os
import sys
import urllib.request

def get(base, path):
    with urllib.request.urlopen(f"http://{base}{path}", timeout=10) as response:
        return response.read()


def load_expect(path):
    result = {}
    for line in open(path, encoding="ascii"):
        key, *values = line.rstrip("\n").split("\t")
        result.setdefault(key, []).append([bytes.fromhex(v) for v in values])
    return result


def lines(text):
    return text.replace("\r\n", "\n").replace("\r", "\n")


def check(base, summary, expect, sent):
    problems = []

    def need(ok, what):
        if not ok:
            problems.append(what)

    mid = summary["ID"]
    message = json.loads(get(base, f"/api/v1/message/{mid}"))
    need(message["Subject"] == expect["subject"][0][0].decode(), f"subject {message['Subject']!r}")
    want_name, want_addr = (v.decode() for v in expect["from"][0])
    need(message["From"]["Address"] == want_addr and message["From"]["Name"] == want_name, f"from {message['From']!r}")
    want_to = [(n.decode(), a.decode()) for n, a in expect["to"]]
    got_to = [(t["Name"], t["Address"]) for t in message["To"]]
    need(got_to == want_to, f"to {got_to!r} != {want_to!r}")

    text = expect["text"][0][0].decode()
    html = expect["html"][0][0].decode()
    # Without a text part Mailpit makes a text from the HTML itself.
    if text:
        need(lines(message["Text"]).strip() == lines(text).strip(), f"text differs: {message['Text']!r}")
    need(lines(message["HTML"]).strip() == lines(html).strip(), "html differs")

    images = expect.get("image", [])
    inline = message.get("Inline") or []
    need(len(inline) == len(images), f"{len(inline)} inline images, wanted {len(images)}")
    by_cid = {i["ContentID"].strip("<>"): i for i in inline}
    for cid, fname, ctype, data in images:
        cid = cid.decode()
        part = by_cid.get(cid)
        if part is None:
            problems.append(f"image {cid} did not arrive")
            continue
        need(part["FileName"] == fname.decode(), f"filename {part['FileName']!r}")
        need(part["ContentType"] == ctype.decode(), f"content type {part['ContentType']!r}")
        body = get(base, f"/api/v1/message/{mid}/part/{part['PartID']}")
        need(body == data, f"image {cid} differs ({len(body)} vs {len(data)} bytes)")
    need(not message.get("Attachments"), "images arrived as attachments")

    # What Mailpit stored is what was sent: the body after the headers
    # Mailpit may add in front is the same bytes.
    raw = get(base, f"/api/v1/message/{mid}/raw")
    body = sent.split(b"\r\n\r\n", 1)[1].rstrip(b"\r\n")
    need(raw.rstrip(b"\r\n").endswith(body), "the stored body differs from what was sent")
    parsed = email.message_from_bytes(raw, policy=email.policy.default)
    need(all(not p.defects for p in parsed.walk()), "the stored message has defects")
    return problems


def main():
    root = sys.argv[1]
    failed = False
    for mode in sorted(os.listdir(root)):
        folder = os.path.join(root, mode)
        if not os.path.isdir(folder):
            continue
        base = open(os.path.join(folder, "http")).read().strip()
        listing = json.loads(get(base, "/api/v1/messages?limit=500"))
        by_id = {m["MessageID"]: m for m in listing["messages"]}
        names = sorted(f[:-4] for f in os.listdir(folder) if f.endswith(".eml"))
        print(f"{mode}: {listing['total']} messages in Mailpit, {len(names)} sent")
        if listing["total"] != len(names):
            failed = True
            print(f"FAIL {mode}: the counts differ")
        for name in names:
            expect = load_expect(os.path.join(folder, name + ".expect"))
            sent = open(os.path.join(folder, name + ".eml"), "rb").read()
            summary = by_id.get(expect["message-id"][0][0].decode())
            if summary is None:
                failed = True
                print(f"FAIL {mode}/{name}: not in Mailpit")
                continue
            problems = check(base, summary, expect, sent)
            if problems:
                failed = True
                print(f"FAIL {mode}/{name}: " + "; ".join(problems))
            else:
                print(f"ok {mode}/{name}")
    sys.exit(1 if failed else 0)


main()
