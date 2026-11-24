#!/usr/bin/env python3
"""Asks a server about uploads and downloads and prints what a client could tell.

  uploads-probe.py <base-url> <setup-code> [database-file]

Founds the instance with `boss` (needs a fresh server), registers `maja`, then
sends one corpus of requests, byte for byte the same to whichever server it is
given, and prints one normalized line per request: the status, the headers a
browser looks at, and the body with ids, times and trace identifiers replaced.
Run it against the first server and this one and diff the two outputs.

The first server makes the invite itself; for a server that has no route for
invites yet, name its database file and the invite is written there.

Both servers must have the rate limit of uploads lifted and no minimum of free
disk space (Uploads__MinimumFreeDataRootBytes=0).
"""
import json
import re
import socket
import sys

PASSWORD = "Corona!2026-long"
CLIENT = {"X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0"}
PNG = bytes([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4])
JPEG = bytes([0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0])
WEBP = b"RIFF\0\0\0\0WEBP\0\0\0\0"
WAVE = b"RIFF\0\0\0\0WAVE\0\0\0\0"
MP4 = bytes([0, 0, 0, 16]) + b"ftyp" + bytes([1, 2, 3, 4])
PDF = b"%PDF-1.7\n%%EOF\n"
SAMPLES = {
    "image/jpeg": JPEG, "image/png": PNG, "image/gif": b"GIF89a....", "image/webp": WEBP,
    "audio/mpeg": b"ID3\x04....", "audio/ogg": b"OggS....", "audio/wav": WAVE, "audio/x-wav": WAVE,
    "audio/webm": b"\x1a\x45\xdf\xa3....", "audio/mp4": MP4, "audio/x-m4a": MP4, "audio/aac": b"\xff\xf1....",
    "video/mp4": MP4, "video/x-m4v": MP4, "video/webm": b"\x1a\x45\xdf\xa3....", "video/ogg": b"OggS....",
    "video/quicktime": MP4, "application/pdf": PDF, "application/zip": b"PK\x03\x04....",
    "application/x-zip-compressed": b"PK\x03\x04....", "text/plain": b"hello", "application/json": b' {"a":1}',
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document": b"PK\x03\x04..",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet": b"PK\x05\x06..",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation": b"PK\x07\x08..",
}


def send(base, method, path, body=None, headers=None, cookie=None, timeout=60):
    host, port = base.split("//")[1].split(":")
    h = dict(CLIENT)
    h.update(headers or {})
    if cookie:
        h["Cookie"] = cookie
    h["Host"] = f"{host}:{port}"
    if body is not None and "Content-Length" not in h and "Transfer-Encoding" not in h:
        h["Content-Length"] = str(len(body))
    head = f"{method} {path} HTTP/1.1\r\n" + "".join(f"{k}: {v}\r\n" for k, v in h.items()) + "Connection: close\r\n\r\n"
    s = socket.create_connection((host, int(port)), timeout=timeout)
    s.settimeout(timeout)
    buf = b""
    try:
        s.sendall(head.encode("latin1", "replace"))
        if body:
            try:
                s.sendall(body)
            except (BrokenPipeError, ConnectionResetError):
                pass
        while True:
            c = s.recv(1 << 20)
            if not c:
                break
            buf += c
    except (socket.timeout, ConnectionResetError):
        pass
    s.close()
    return parse(buf)


def parse(buf):
    if not buf:
        return 0, {}, b""
    head, _, rest = buf.partition(b"\r\n\r\n")
    lines = head.decode("latin1").split("\r\n")
    status = int(lines[0].split()[1])
    headers = {}
    for line in lines[1:]:
        k, _, v = line.partition(":")
        headers.setdefault(k.strip().lower(), []).append(v.strip())
    if "chunked" in headers.get("transfer-encoding", [""])[0]:
        out = b""
        while rest:
            size_line, _, rest = rest.partition(b"\r\n")
            try:
                size = int(size_line, 16)
            except ValueError:
                break
            if size == 0:
                break
            out += rest[:size]
            rest = rest[size + 2:]
        rest = out
    return status, headers, rest


def jcall(base, method, path, obj=None, cookie=None):
    body = json.dumps(obj).encode() if obj is not None else None
    return send(base, method, path, body, {"Content-Type": "application/json"}, cookie)


def session_cookie(headers):
    for line in headers.get("set-cookie", []):
        first = line.split(";")[0]
        if "session" in first.split("=")[0].lower():
            return first
    raise SystemExit("no session cookie")


def register(base, name, extra):
    status, body, _ = None, None, None
    status, _, body = jcall(base, "POST", "/api/auth/register", dict({
        "username": name, "displayName": name, "email": f"{name}@example.test", "password": PASSWORD}, **extra))
    assert status in (200, 201), (status, body)
    status, headers, body = jcall(base, "POST", "/api/auth/login", {"username": name, "password": PASSWORD})
    assert status == 200, (status, body)
    return session_cookie(headers), json.loads(body)["id"]


UUID = re.compile(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
TRACE = re.compile(r"00-[0-9a-f]{32}-[0-9a-f]{16}-00")
TIME = re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?Z?")
HEX32 = re.compile(r"[0-9a-f]{32}")


def scrub(text):
    text = UUID.sub("<id>", text)
    text = TRACE.sub("<trace>", text)
    text = TIME.sub("<time>", text)
    return HEX32.sub("<n>", text)


# What the two servers do differently by nature (chunking, whether a bodiless
# answer closes the connection, the length of a JSON answer) is not shown.
SHOWN = ["content-type", "content-length", "location", "content-disposition", "content-range", "accept-ranges",
         "x-content-type-options", "last-modified", "etag", "cache-control"]


def describe(label, answer, body_limit=1500):
    status, headers, body = answer
    json_answer = "json" in headers.get("content-type", [""])[0]
    shown = [f"{k}={scrub(headers[k][0])}" for k in SHOWN if k in headers and not (k == "content-length" and (json_answer or status in (400, 401, 403, 404, 415)))]
    try:
        text = body[:body_limit].decode("utf-8")
    except UnicodeDecodeError:
        text = repr(body[:body_limit])
    print(f"{label} | {status} | {' '.join(shown)} | {scrub(text)}")


def multipart(parts, boundary="XXB", end=True):
    out = b"".join(b"--" + boundary.encode() + b"\r\n" + p for p in parts)
    return out + (b"--" + boundary.encode() + b"--\r\n" if end else b"")


def part(name, value, filename=None, ctype=None, extra=None):
    head = f'Content-Disposition: form-data; name="{name}"' if name is not None else "Content-Disposition: form-data"
    if filename is not None:
        head += f'; filename="{filename}"'
    head += "\r\n"
    if ctype:
        head += f"Content-Type: {ctype}\r\n"
    for line in extra or []:
        head += line + "\r\n"
    return head.encode("utf-8", "surrogateescape") + b"\r\n" + (value if isinstance(value, bytes) else value.encode()) + b"\r\n"


def raw_part(head, value):
    return head + b"\r\n\r\n" + value + b"\r\n"


def post(base, path, parts, cookie, boundary="XXB", ctype=None, end=True, extra_headers=None, raw=None):
    data = raw if raw is not None else multipart(parts, boundary, end)
    headers = {"Content-Type": ctype or f"multipart/form-data; boundary={boundary}"}
    headers.update(extra_headers or {})
    return send(base, "POST", path, data, headers, cookie)


def file_part(content=PNG, name="a.png", ctype="image/png", field="file"):
    return part(field, content, name, ctype)


def corpus(base, boss, maja, outsider):
    def up(label, parts, cookie=boss, path="/api/uploads", **k):
        answer = post(base, path, parts, cookie, **k)
        describe(label, answer)
        try:
            return json.loads(answer[2])
        except ValueError:
            return {}

    # Every type, with its sample and with a wrong one.
    for ctype, sample in SAMPLES.items():
        up(f"type {ctype}", [file_part(sample, "x", ctype)])
        up(f"type {ctype} wrong bytes", [file_part(b"\x00\x01\x02\x03wrong\x00", "x", ctype)])
    up("type upper case", [file_part(PNG, "x", "IMAGE/PNG; charset=x")])
    up("type unsupported", [file_part(b"<html>", "x", "text/html")])
    up("type none", [part("file", PNG, "x")])
    up("empty", [file_part(b"")])
    up("no file", [part("x", "1")])
    up("file as text", [part("file", PNG)])
    up("two files", [file_part(PNG, "first.png"), file_part(PDF, "second.pdf", "application/pdf")])
    up("legacy path", [file_part()], path="/api/uploads/images")
    # Names.
    for name in ["release notes.exe", "my:file*name?.png", "../../etc/passwd.png", "////", "   ...   .png", "zażółć gęślą.png",
                 "a" * 400 + ".png", "x\x00y.png", "C:\\dir\\a b.png", ".png", "a.b.c.png", "日本語.png", "emoji😀.png"]:
        up(f"name {name[:30]!r}", [file_part(PNG, name)])
    # Metadata.
    up("video meta", [file_part(MP4, "c.mp4", "video/mp4"), part("durationSeconds", "42.1254"), part("width", "1920"), part("height", "1080")])
    for field, value in [("durationSeconds", "abc"), ("durationSeconds", "1,2"), ("durationSeconds", "NaN"), ("durationSeconds", "-1"),
                         ("durationSeconds", "1e999"), ("durationSeconds", " 3 "), ("durationSeconds", ""), ("durationSeconds", "0.0004"),
                         ("width", "0"), ("width", "40000"), ("width", "x"), ("voiceMessage", "yes"), ("voiceMessage", "TRUE"),
                         ("voiceMessage", "false")]:
        up(f"audio {field}={value!r}", [file_part(WAVE, "a.wav", "audio/wav"), part("durationSeconds", "5") if field != "durationSeconds" else b"", part(field, value)])
    up("voice ok", [file_part(WAVE, "a.wav", "audio/wav"), part("durationSeconds", "5"), part("voiceMessage", "true")])
    up("voice too long", [file_part(WAVE, "a.wav", "audio/wav"), part("durationSeconds", "601"), part("voiceMessage", "true")])
    up("voice image", [file_part(), part("durationSeconds", "5"), part("voiceMessage", "true")])
    up("image duration", [file_part(), part("durationSeconds", "5")])
    up("width without height", [file_part(), part("width", "5")])
    up("pdf dimensions", [file_part(PDF, "a.pdf", "application/pdf"), part("width", "5"), part("height", "5")])
    up("duplicate duration", [file_part(MP4, "c.mp4", "video/mp4"), part("durationSeconds", "1"), part("durationSeconds", "2")])
    # Previews and posters.
    up("poster ok", [file_part(MP4, "c.mp4", "video/mp4"), file_part(JPEG, "p.jpg", "image/jpeg", "poster")])
    up("poster on image", [file_part(), file_part(JPEG, "p.jpg", "image/jpeg", "poster")])
    up("poster bad bytes", [file_part(MP4, "c.mp4", "video/mp4"), file_part(b"nope", "p.jpg", "image/jpeg", "poster")])
    up("poster empty", [file_part(MP4, "c.mp4", "video/mp4"), file_part(b"", "p.jpg", "image/jpeg", "poster")])
    up("poster too large", [file_part(MP4, "c.mp4", "video/mp4"), file_part(JPEG + b"x" * (2 * 1024 * 1024), "p.jpg", "image/jpeg", "poster")])
    up("poster at limit", [file_part(MP4, "c.mp4", "video/mp4"), file_part(JPEG + b"x" * (2 * 1024 * 1024 - len(JPEG)), "p.jpg", "image/jpeg", "poster")])
    up("preview webp", [file_part(), file_part(WEBP, "p.webp", "image/webp", "preview")])
    up("preview jpeg", [file_part(), file_part(JPEG, "p.jpg", "image/jpeg", "preview")])
    up("preview png", [file_part(), file_part(PNG, "p.png", "image/png", "preview")])
    up("preview too large", [file_part(), file_part(JPEG + b"x" * (1024 * 1024), "p.jpg", "image/jpeg", "preview")])
    up("preview on video", [file_part(MP4, "c.mp4", "video/mp4"), file_part(JPEG, "p.jpg", "image/jpeg", "preview")])
    up("placeholder ok", [file_part(), part("placeholder", "AAAA")])
    for value in ["", "AAA", "AA==", "A===", "AAAA AAAA", "A" * 44, "A" * 43 + "=", "A" * 42 + "==", "A" * 48, "AA=A", "????"]:
        up(f"placeholder {value[:12]!r}/{len(value)}", [file_part(), part("placeholder", value)])
    up("placeholder on pdf", [file_part(PDF, "a.pdf", "application/pdf"), part("placeholder", "AAAA")])
    # Limits one byte over and under.
    up("image at limit", [file_part(PNG + b"x" * (10 * 1024 * 1024 - len(PNG)))])
    up("image over limit", [file_part(PNG + b"x" * (10 * 1024 * 1024 - len(PNG) + 1))])
    up("pdf over limit", [file_part(PDF + b"x" * (25 * 1024 * 1024), "a.pdf", "application/pdf")])
    up("audio over limit", [file_part(WAVE + b"x" * (25 * 1024 * 1024), "a.wav", "audio/wav")])
    # Encrypted.
    up("encrypted", [part("file", b"\x01\x02ciphertext", "x", "application/octet-stream")], path="/api/uploads/encrypted")
    up("encrypted any type", [part("file", b"bytes", "x", "text/html")], path="/api/uploads/encrypted")
    up("encrypted empty", [part("file", b"", "x", "application/octet-stream")], path="/api/uploads/encrypted")
    up("encrypted over", [part("file", b"x" * (100 * 1024 * 1024 + 17), "x")], path="/api/uploads/encrypted")
    up("encrypted at limit", [part("file", b"x" * (100 * 1024 * 1024 + 16), "x")], path="/api/uploads/encrypted")
    # Forms that are broken.
    up("boundary 128", [file_part()], boundary="B" * 128)
    up("boundary 129", [file_part()], boundary="B" * 129)
    up("no final boundary", [file_part()], end=False)
    up("preamble", [], raw=b"junk before\r\n" + multipart([file_part()]))
    up("epilogue", [], raw=multipart([file_part()]) + b"epilogue\r\n")
    up("bare LF", [], raw=multipart([file_part()]).replace(b"\r\n", b"\n"))
    up("not multipart", [], ctype="application/json", raw=b"{}")
    up("urlencoded", [], ctype="application/x-www-form-urlencoded", raw=b"a=b")
    up("no boundary", [], ctype="multipart/form-data", raw=multipart([file_part()]))
    up("part without name", [part(None, "1"), file_part()])
    up("16 headers", [file_part_with(16)])
    up("17 headers", [file_part_with(17)])
    up("1024 sections", [part(f"f{i}", "1") for i in range(1023)] + [file_part()])
    up("1025 sections", [part(f"f{i}", "1") for i in range(1024)] + [file_part()])
    up("NAME in capitals", [raw_part(b'Content-Disposition: form-data; NAME="FILE"; FILENAME="a.png"\r\nContent-Type: image/png', PNG)])
    up("filename star", [raw_part(b"Content-Disposition: form-data; name=\"file\"; filename*=UTF-8''z%C3%B3%C5%82w.png\r\nContent-Type: image/png", PNG)])
    up("filename empty", [file_part(PNG, "")])
    up("big ignored field", [part("zzz", "a" * (3 * 1024 * 1024)), file_part()])
    # Unauthorized.
    describe("anonymous", post(base, "/api/uploads", [file_part()], None))
    describe("anonymous encrypted", post(base, "/api/uploads/encrypted", [file_part()], None))
    # Who may download what.
    mine = up("download source png", [file_part(PNG, "tiny.png")])
    pdf = up("download source pdf", [file_part(PDF, "Ünï cødé \"q\" x.pdf", "application/pdf")])
    wav = up("download source wav", [file_part(WAVE, "s.wav", "audio/wav")])
    video = up("download source video", [file_part(MP4, "c.mp4", "video/mp4"), file_part(JPEG, "p.jpg", "image/jpeg", "poster")])
    image = up("download source image", [file_part(), file_part(WEBP, "p.webp", "image/webp", "preview")])
    enc = up("download source encrypted", [part("file", b"\x01\x02ciphertext", "x")], path="/api/uploads/encrypted")
    long_name = up("download source long name", [file_part(PDF, "n" * 300 + ".pdf", "application/pdf")])
    slashes = up("download source slashes", [file_part(PDF, "a/b\\c%d;e'f.pdf", "application/pdf")])
    for who, cookie in [("owner", boss), ("other", maja), ("outsider", outsider), ("anonymous", None)]:
        for label, item in [("png", mine), ("pdf", pdf), ("wav", wav), ("video", video), ("enc", enc)]:
            if "id" in item:
                describe(f"get {label} as {who}", send(base, "GET", f"/assets/{item['id']}", None, None, cookie))
    describe("get unknown", send(base, "GET", "/assets/00000000-0000-0000-0000-000000000000", None, None, boss))
    describe("get not an id", send(base, "GET", "/assets/nope", None, None, boss))
    describe("head", send(base, "HEAD", f"/assets/{mine['id']}", None, None, boss))
    describe("post", send(base, "POST", f"/assets/{mine['id']}", None, None, boss))
    for label, item in [("poster", video), ("preview", image)]:
        describe(f"get {label}", send(base, "GET", f"/assets/{item['id']}/{label}", None, None, boss))
        describe(f"get {label} as other", send(base, "GET", f"/assets/{item['id']}/{label}", None, None, maja))
    describe("get poster of png", send(base, "GET", f"/assets/{mine['id']}/poster", None, None, boss))
    describe("get preview of png", send(base, "GET", f"/assets/{mine['id']}/preview", None, None, boss))
    for item in (long_name, slashes):
        describe("get awkward name", send(base, "GET", f"/assets/{item['id']}", None, None, boss))
    # Ranges and conditions on a 12-byte file.
    big = up("range source", [file_part(PNG + b"abcdefghijklmnopqrstuvwxyz0123456789", "r.png")])
    for value in ["bytes=0-4", "bytes=5-", "bytes=-5", "bytes=0-99999", "bytes=40-", "bytes=48-", "bytes=47-47", "bytes=0-0", "bytes=4-2",
                  "bytes=0-1,5-6", "BYTES=2-3", "items=0-1", "bytes=a-b", "bytes=1", "", "bytes=0-4, ", " bytes=0-4", "bytes=09-12", "bytes=2- 5",
                  "bytes=9223372036854775807-", "bytes=0-9223372036854775808"]:
        describe(f"range {value!r}", send(base, "GET", f"/assets/{big['id']}", None, {"Range": value}, boss))
    for header, value in [("If-None-Match", '"x"'), ("If-Modified-Since", "Fri, 20 Nov 2090 10:11:12 GMT"), ("If-Modified-Since", "Fri, 20 Nov 2000 10:11:12 GMT"),
                          ("If-Unmodified-Since", "Fri, 20 Nov 2000 10:11:12 GMT"), ("If-Match", '"x"'), ("If-Range", '"x"')]:
        describe(f"condition {header}", send(base, "GET", f"/assets/{big['id']}", None, {header: value, "Range": "bytes=0-1"}, boss))
    # Profile pictures.
    for kind in ["avatar", "banner", "wallpaper", "AVATAR", " avatar ", "nope"]:
        up(f"picture {kind!r}", [file_part(PNG, "me.png")], path=f"/api/me/images/{kind.replace(' ', '%20')}")
    up("avatar over 4 MiB", [file_part(PNG + b"x" * (4 * 1024 * 1024))], path="/api/me/images/avatar")
    up("avatar at 4 MiB", [file_part(PNG + b"x" * (4 * 1024 * 1024 - len(PNG)))], path="/api/me/images/avatar")
    up("banner at 8 MiB", [file_part(PNG + b"x" * (8 * 1024 * 1024 - len(PNG)))], path="/api/me/images/banner")
    up("banner over 8 MiB", [file_part(PNG + b"x" * (8 * 1024 * 1024 - len(PNG) + 1))], path="/api/me/images/banner")
    up("avatar pdf", [file_part(PDF, "a.pdf", "application/pdf")], path="/api/me/images/avatar")
    up("avatar wrong bytes", [file_part(b"nope", "a.png")], path="/api/me/images/avatar")
    up("avatar empty", [file_part(b"")], path="/api/me/images/avatar")
    up("avatar no file", [part("x", "1")], path="/api/me/images/avatar")
    up("unknown kind no file", [part("x", "1")], path="/api/me/images/nope")
    describe("avatar anonymous", post(base, "/api/me/images/avatar", [file_part()], None))
    for kind in ["avatar", "banner", "wallpaper", "nope"]:
        describe(f"remove {kind}", send(base, "DELETE", f"/api/me/images/{kind}", None, None, boss))
    describe("remove avatar again", send(base, "DELETE", "/api/me/images/avatar", None, None, boss))
    # The server icon.
    up("icon", [file_part(PNG, "i.png")], path="/api/admin/server/icon")
    up("icon pdf", [file_part(PDF, "a.pdf", "application/pdf")], path="/api/admin/server/icon")
    up("icon over", [file_part(PNG + b"x" * (4 * 1024 * 1024))], path="/api/admin/server/icon")
    up("icon as a member", [file_part()], cookie=maja, path="/api/admin/server/icon")
    up("icon as outsider", [file_part()], cookie=outsider, path="/api/admin/server/icon")
    describe("icon remove as member", send(base, "DELETE", "/api/admin/server/icon", None, None, maja))
    describe("icon remove", send(base, "DELETE", "/api/admin/server/icon", None, None, boss))
    describe("icon remove again", send(base, "DELETE", "/api/admin/server/icon", None, None, boss))


def file_part_with(headers):
    extra = [f"X-H{i}: 1" for i in range(headers - 2)]
    return part("file", PNG, "a.png", "image/png", extra)


def invite_in(database):
    import sqlite3
    import uuid
    from datetime import datetime, timezone
    db = sqlite3.connect(database)
    server, owner = db.execute('SELECT "ServerId", "UserId" FROM "ServerMembers" WHERE "OwnerSlot" = 1 LIMIT 1').fetchone()
    code = "invite" + uuid.uuid4().hex[:10]
    now = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S.%f0")
    db.execute('INSERT INTO "ServerInvites" ("Id", "ServerId", "CreatedById", "ChannelId", "Code", "Uses", "MaxUses", "ExpiresAt", "CreatedAt") '
               "VALUES (?, ?, ?, NULL, ?, 0, NULL, NULL, ?)", (str(uuid.uuid4()).upper(), server, owner, code, now))
    db.commit()
    db.close()
    return code


def main():
    base, setup = sys.argv[1], sys.argv[2]
    boss, _ = register(base, "boss", {"setupCode": setup})
    if len(sys.argv) > 3:
        invite = invite_in(sys.argv[3])
    else:
        status, _, body = jcall(base, "POST", "/api/admin/invites", {}, boss)
        invite = json.loads(body)["url"].rsplit("/", 1)[1]
    maja, _ = register(base, "maja", {"inviteCode": invite})
    # A person who is on no server at all: owns nothing here.
    outsider = None
    corpus(base, boss, maja, outsider)


if __name__ == "__main__":
    main()
