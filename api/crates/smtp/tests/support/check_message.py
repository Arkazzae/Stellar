"""Reads messages the crate wrote, with Python's own parser, and checks them
against what was meant to be in them.

Usage: python3 -I check_message.py DIR
For every NAME.eml in DIR there is a NAME.expect, lines of `key<TAB>hex...`.
"""
import email
import email.policy
import email.utils
import datetime
import os
import sys


def load_expect(path):
    result = {}
    for line in open(path, encoding="ascii"):
        key, *values = line.rstrip("\n").split("\t")
        result.setdefault(key, []).append([bytes.fromhex(v) for v in values])
    return result


def lines(text):
    return text.replace("\r\n", "\n").replace("\r", "\n")


def same(got, want, single):
    # A single-part message must end with a line break, so a text that has
    # none comes back with one; inside a multipart nothing is added.
    got, want = lines(got), lines(want)
    return got == want or (single and got == want + "\n")


def check(name, raw, expect):
    problems = []

    def need(ok, what):
        if not ok:
            problems.append(what)

    # The wire form itself.
    need(raw.endswith(b"\r\n"), "does not end with CRLF")
    need(b"\n" not in raw.replace(b"\r\n", b""), "a bare LF")
    need(b"\r" not in raw.replace(b"\r\n", b""), "a bare CR")
    need(all(b < 128 for b in raw), "not 7-bit")
    need(b"\x00" not in raw, "a NUL")
    for n, l in enumerate(raw.split(b"\r\n")):
        need(len(l) <= 998, f"line {n} is {len(l)} bytes")
    head = raw.split(b"\r\n\r\n", 1)[0]
    need(all(len(l) <= 78 for l in head.split(b"\r\n")), "a header line is over 78 bytes")

    msg = email.message_from_bytes(raw, policy=email.policy.default)
    for part in msg.walk():
        need(not part.defects, f"defects in {part.get_content_type()}: {part.defects}")

    need(msg["Subject"] == expect["subject"][0][0].decode(), f"subject {msg['Subject']!r}")
    sender = msg["From"].addresses[0]
    want_name, want_addr = expect["from"][0]
    need(sender.display_name == want_name.decode() and sender.addr_spec == want_addr.decode(), f"from {msg['From']!r}")
    got_to = [(a.display_name, a.addr_spec) for a in msg["To"].addresses]
    want_to = [(n.decode(), a.decode()) for n, a in expect["to"]]
    need(got_to == want_to, f"to {got_to!r} != {want_to!r}")
    need(msg["Message-ID"] == "<" + expect["message-id"][0][0].decode() + ">", f"message-id {msg['Message-ID']!r}")
    when = datetime.datetime.fromtimestamp(int(expect["date"][0][0]), datetime.timezone.utc)
    need(email.utils.format_datetime(msg["Date"].datetime) == email.utils.format_datetime(when), "date")
    need(msg["Date"] == email.utils.format_datetime(when), f"date text {msg['Date']!r}")
    need(msg["MIME-Version"] == "1.0", "MIME-Version")

    text = expect["text"][0][0].decode()
    html = expect["html"][0][0].decode()
    images = expect.get("image", [])
    plain_part = msg.get_body(("plain",))
    html_part = msg.get_body(("html",))
    if text:
        need(plain_part is not None and same(plain_part.get_content(), text, not msg.is_multipart()), "text part differs")
        need(plain_part is None or plain_part.get_content_charset() == "utf-8", "text charset")
    else:
        need(plain_part is None, "an unexpected text part")
    if html:
        need(html_part is not None and same(html_part.get_content(), html, not msg.is_multipart()), "html part differs")
    else:
        need(html_part is None, "an unexpected html part")

    found = [p for p in msg.walk() if p.get_content_maintype() == "image"]
    need(len(found) == len(images), f"{len(found)} images, wanted {len(images)}")
    for part, (cid, fname, ctype, data) in zip(found, images):
        need(part["Content-Id"] == "<" + cid.decode() + ">", f"content-id {part['Content-Id']!r}")
        need(part.get_filename() == fname.decode(), f"filename {part.get_filename()!r}")
        need(part.get_content_type() == ctype.decode(), "image type")
        need(part.get_content_disposition() == "inline", "image disposition")
        need(part.get_content() == data, "image bytes differ")
        # The HTML must be able to find it.
        need(("cid:" + cid.decode()) in html, "the HTML does not refer to the image")

    # The structure: alternative(text, related(html, images...)).
    if images:
        related = [p for p in msg.walk() if p.get_content_type() == "multipart/related"]
        need(len(related) == 1, "no multipart/related")
        if related:
            kinds = [p.get_content_type() for p in related[0].iter_parts()]
            need(kinds == ["text/html"] + ["image/png"] * len(images), f"related holds {kinds}")
    if text and html:
        need(msg.get_content_type() == "multipart/alternative", f"top is {msg.get_content_type()}")
        need([p.get_content_type() for p in msg.iter_parts()][0] == "text/plain", "text must come first")
    return problems


def main():
    directory = sys.argv[1]
    failed = False
    names = sorted(f[:-4] for f in os.listdir(directory) if f.endswith(".eml"))
    if not names:
        print("no messages to check")
        sys.exit(2)
    for name in names:
        raw = open(os.path.join(directory, name + ".eml"), "rb").read()
        problems = check(name, raw, load_expect(os.path.join(directory, name + ".expect")))
        if problems:
            failed = True
            print(f"FAIL {name}: " + "; ".join(problems))
        else:
            print(f"ok {name}")
    sys.exit(1 if failed else 0)


main()
