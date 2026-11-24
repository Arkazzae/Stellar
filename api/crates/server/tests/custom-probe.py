#!/usr/bin/env python3
"""Asks a server about custom emoji, the soundboard, the own profile and the
settings a client keeps, and prints what a client could tell.

  custom-probe.py <base-url> <setup-code> [database-file]

Founds the instance with `boss` (needs a fresh server), registers `maja` and
`jo`, then sends one corpus of requests, byte for byte the same to whichever
server it is given, and prints one normalized line per request. Run it against
the first server and this one and diff the two outputs. `custom-compare.sh`
does that, and compares what the two leave in their data folders.

The first server makes the invites itself; for a server that has no route for
invites yet, name its database file and the invites are written there.

Both servers must have the rate limit of uploads lifted and no minimum of
free disk space (Uploads__MinimumFreeDataRootBytes=0).
"""
import importlib.util
import json
import os
import re
import struct
import sys
import threading

here = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("uploads_probe", os.path.join(here, "uploads-probe.py"))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)
# Somebody who is no member of the server asked about is the same person with
# another server named, which is how a client says it.
OUTSIDER = "; outsider"
_send = probe.send


def _outsider_aware(base, method, path, body=None, headers=None, cookie=None, timeout=60):
    if cookie and cookie.endswith(OUTSIDER):
        cookie = cookie[: -len(OUTSIDER)]
        headers = dict(headers or {}, **{"X-Stellarr-Server": "00000000-0000-0000-0000-0000000000aa"})
    return _send(base, method, path, body, headers, cookie, timeout)


probe.send = _outsider_aware
_scrub = probe.scrub
_STRING = re.compile(r'"(?:[^"\\\x00-\x1f]|\\.)*"')


def _canonical(match):
    """A string as one canonical text. The first server wrote strings in its
    answers with a mind of its own about which characters to escape (it left
    some outside ASCII as they were and escaped others); both are the same
    JSON, so what is compared is what the string says."""
    try:
        value = json.loads(match.group(0))
    except ValueError:
        return match.group(0)
    out = ['"']
    for ch in value:
        if ch in '"\\':
            out.append("\\" + ch)
        elif ch in "\n\t\r\b\f":
            out.append({"\n": "\\n", "\t": "\\t", "\r": "\\r", "\b": "\\b", "\f": "\\f"}[ch])
        elif 0x20 <= ord(ch) < 0x7F and ch not in "&'+<>`":
            out.append(ch)
        else:
            data = ch.encode("utf-16-le")
            out.extend("\\u%04X" % unit for unit in struct.unpack("<%dH" % (len(data) // 2), data))
    out.append('"')
    return "".join(out)


def _scrub_escaped(text):
    return _scrub(_STRING.sub(_canonical, text))


probe.scrub = _scrub_escaped
send, jcall, describe, part, multipart, post = probe.send, probe.jcall, probe.describe, probe.part, probe.multipart, probe.post
PNG, JPEG, WEBP, WAVE, PDF = probe.PNG, probe.JPEG, probe.WEBP, probe.WAVE, probe.PDF
GIF = b"GIF89a...."


def show(label, answer, limit=12000):
    describe(label, answer, limit)
    status, _, body = answer
    try:
        return json.loads(body)
    except ValueError:
        return {}


def raw_json(base, method, path, body, cookie, ctype="application/json", headers=None):
    h = {"Content-Type": ctype} if ctype else {}
    h.update(headers or {})
    return send(base, method, path, body, h, cookie)


def wave(base, cookie, duration="9", ctype="audio/wav", content=WAVE):
    answer = post(base, "/api/uploads", [part("file", content, "s.wav", ctype), part("durationSeconds", duration)], cookie)
    return json.loads(answer[2]).get("id")


def emoji(base, cookie, code, content=PNG, ctype="image/png", name="e.png", extra=(), label=None, field="shortcode"):
    parts = [] if code is None else [part(field, code)]
    if content is not None:
        parts.append(part("file", content, name, ctype))
    parts.extend(extra)
    return show(label or f"emoji {code!r} {ctype}", post(base, "/api/emoji", parts, cookie))


def emoji_corpus(base, boss, maja, outsider):
    show("emoji list empty", send(base, "GET", "/api/emoji", None, None, maja))
    show("emoji list anonymous", send(base, "GET", "/api/emoji", None, None, None))
    emoji(base, maja, "member")
    emoji(base, outsider, "outsider")
    first = emoji(base, boss, ":Mushris:")
    emoji(base, boss, "mushris", label="emoji same name")
    emoji(base, boss, "MUSHRIS", label="emoji same name, other case")
    for typed in [" :abc: ", "::abc2::", "ab", "a", "", "   ", "a-b", "a b", "ab\n", ":ab2\n:", ":ab3\n\n:", ":\nab4:", "ab5\r", "x" * 32, "x" * 33,
                  "K" + "kk", "İ" + "ab", "ÅÄ", "ab ", "　ab6　", "áb", "9_9", "___", "z_" * 16, "z_" * 17, "ab:cd", "a\x00b"]:
        emoji(base, boss, typed, label=f"emoji name {typed!r}")
    emoji(base, boss, None, label="emoji no shortcode")
    emoji(base, boss, "nofile", content=None, label="emoji no file")
    emoji(base, boss, "twice", extra=[part("shortcode", "other")], label="emoji shortcode twice")
    emoji(base, boss, "twofiles", extra=[part("file", PDF, "x.pdf", "application/pdf")], label="emoji two files")
    emoji(base, boss, "SHORTCODE", field="Shortcode", label="emoji field in other case")
    emoji(base, boss, "asfile", content=None, extra=[part("file", PNG)], label="emoji file as text")
    # Types, with the right bytes and with the wrong ones.
    samples = {"image/jpeg": JPEG, "image/png": PNG, "image/gif": GIF, "image/webp": WEBP, "image/svg+xml": b"<svg/>", "image/bmp": b"BM....",
               "audio/wav": WAVE, "application/pdf": PDF, "text/plain": b"hello", "video/mp4": probe.MP4}
    for i, (ctype, sample) in enumerate(samples.items()):
        emoji(base, boss, f"type_ok_{i}", sample, ctype, label=f"emoji type {ctype}")
        emoji(base, boss, f"type_bad_{i}", b"\x00\x01\x02\x03wrong\x00", ctype, label=f"emoji type {ctype} wrong bytes")
    emoji(base, boss, "typeupper", PNG, "IMAGE/PNG; charset=x", label="emoji type upper case")
    show("emoji no content type", post(base, "/api/emoji", [part("shortcode", "notype"), part("file", PNG, "e.png")], boss))
    emoji(base, boss, "empty", b"", label="emoji empty")
    emoji(base, boss, "empty_bad_type", b"", "text/plain", label="emoji empty text")
    emoji(base, boss, "atlimit", PNG + b"x" * (512 * 1024 - len(PNG)), label="emoji at limit")
    emoji(base, boss, "overlimit", PNG + b"x" * (512 * 1024 - len(PNG) + 1), label="emoji over limit")
    emoji(base, boss, "overbad", b"x" * (512 * 1024 + 1), label="emoji over limit wrong bytes")
    # The order of the refusals.
    emoji(base, boss, "mushris", b"bad", label="emoji taken, wrong bytes")
    emoji(base, boss, "!", b"", "text/plain", label="emoji bad name, empty text")
    emoji(base, maja, "!", b"", "text/plain", label="emoji member, bad name, empty")
    emoji(base, outsider, "!", b"", "text/plain", label="emoji outsider, bad name, empty")
    show("emoji not multipart", raw_json(base, "POST", "/api/emoji", b'{"shortcode":"j"}', boss))
    show("emoji no body", send(base, "POST", "/api/emoji", None, None, boss))
    show("emoji anonymous", post(base, "/api/emoji", [part("shortcode", "anon"), part("file", PNG, "e.png", "image/png")], None))
    listing = show("emoji list", send(base, "GET", "/api/emoji", None, None, maja))
    show("emoji list outsider", send(base, "GET", "/api/emoji", None, None, outsider))
    show("emoji list unknown server", send(base, "GET", "/api/emoji?serverId=00000000-0000-0000-0000-000000000000", None, None, maja))
    show("emoji list bad server id", send(base, "GET", "/api/emoji?serverId=x", None, None, maja))
    # The image.
    glyph = first.get("glyph", "")
    emoji_id = first.get("id", "")
    for label, path, cookie in [("member", glyph, maja), ("owner", glyph, boss), ("outsider", glyph, outsider), ("anonymous", glyph, None),
                                ("no server id", f"/api/emoji/{emoji_id}/image", maja), ("unknown id", f"/api/emoji/{emoji_id[:-1]}0/image", maja),
                                ("unknown id anonymous", f"/api/emoji/{emoji_id[:-1]}0/image", None), ("not an id", "/api/emoji/x/image", maja),
                                ("not an id anonymous", "/api/emoji/x/image", None), ("braces", "/api/emoji/%7B" + emoji_id + "%7D/image", maja),
                                ("without hyphens", f"/api/emoji/{emoji_id.replace('-', '')}/image", maja), ("upper case", f"/api/emoji/{emoji_id.upper()}/image", maja),
                                ("trailing slash", glyph.replace("/image", "/image/"), maja)]:
        describe(f"emoji image {label}", send(base, "GET", path, None, None, cookie), 300)
    describe("emoji image head", send(base, "HEAD", glyph, None, None, maja), 300)
    describe("emoji image range", send(base, "GET", glyph, None, {"Range": "bytes=0-3"}, maja), 300)
    describe("emoji image conditional", send(base, "GET", glyph, None, {"If-None-Match": "*", "If-Modified-Since": "Wed, 21 Oct 2037 07:28:00 GMT"}, maja), 300)
    # Deleting.
    for label, path, cookie in [("member", f"/api/emoji/{emoji_id}", maja), ("outsider", f"/api/emoji/{emoji_id}", outsider), ("anonymous", f"/api/emoji/{emoji_id}", None),
                                ("not an id", "/api/emoji/x", boss), ("unknown", f"/api/emoji/{emoji_id[:-1]}0", boss), ("member unknown", f"/api/emoji/{emoji_id[:-1]}0", maja)]:
        show(f"emoji delete {label}", send(base, "DELETE", path, None, None, cookie))
    show("emoji delete", send(base, "DELETE", f"/api/emoji/{emoji_id}", None, None, boss))
    show("emoji delete again", send(base, "DELETE", f"/api/emoji/{emoji_id}", None, None, boss))
    describe("emoji image after delete", send(base, "GET", glyph, None, None, maja), 300)
    # Eight uploads of one name at once.
    results = []

    def go():
        results.append(post(base, "/api/emoji", [part("shortcode", "race"), part("file", PNG, "e.png", "image/png")], boss)[0])
    threads = [threading.Thread(target=go) for _ in range(8)]
    [t.start() for t in threads]
    [t.join() for t in threads]
    print("emoji race |", sorted(results))
    show("emoji list at the end", send(base, "GET", "/api/emoji", None, None, boss))
    emoji(base, boss, "fresh", label="emoji after delete, a new one")
    return listing


def sound_body(attachment, name="Gong", glyph="G", start=0, end=2, **more):
    body = {"attachmentId": attachment, "name": name, "glyph": glyph, "trimStartSeconds": start, "trimEndSeconds": end}
    body.update(more)
    return body


def soundboard_corpus(base, boss, maja, outsider, ids):
    show("board empty owner", send(base, "GET", "/api/soundboard", None, None, boss))
    show("board empty member", send(base, "GET", "/api/soundboard", None, None, maja))
    show("board outsider", send(base, "GET", "/api/soundboard", None, None, outsider))
    show("board anonymous", send(base, "GET", "/api/soundboard", None, None, None))
    a = wave(base, boss, "8.25")
    b = wave(base, boss, "20")
    c = wave(base, boss, "3")
    d = wave(base, boss, "30")
    mine = wave(base, maja, "3")
    image = json.loads(post(base, "/api/uploads", [part("file", PNG, "a.png", "image/png")], boss)[2])["id"]
    unmeasured = json.loads(post(base, "/api/uploads", [part("file", WAVE, "n.wav", "audio/wav")], boss)[2])["id"]
    upper = json.loads(post(base, "/api/uploads", [part("file", WAVE, "n.wav", "AUDIO/WAV"), part("durationSeconds", "5")], boss)[2])["id"]
    path = "/api/admin/soundboard/sounds"
    created = show("sound create", jcall(base, "POST", path, sound_body(a, "Start rakiety", "\U0001F680", 1.25, 5.5), boss))
    sid = created.get("id", "")
    show("sound create member", jcall(base, "POST", path, sound_body(c), maja))
    show("sound create outsider", jcall(base, "POST", path, sound_body(c), outsider))
    show("sound create anonymous", jcall(base, "POST", path, sound_body(c), None))
    show("sound create taken clip", jcall(base, "POST", path, sound_body(a), boss))
    show("sound create someone else's clip", jcall(base, "POST", path, sound_body(mine), boss))
    show("sound create an image", jcall(base, "POST", path, sound_body(image), boss))
    show("sound create unmeasured", jcall(base, "POST", path, sound_body(unmeasured), boss))
    show("sound create unknown clip", jcall(base, "POST", path, sound_body("00000000-0000-0000-0000-000000000009"), boss))
    show("sound create no clip", jcall(base, "POST", path, {"name": "x", "glyph": "g", "trimEndSeconds": 1}, boss))
    show("sound create nil clip", jcall(base, "POST", path, sound_body("00000000-0000-0000-0000-000000000000"), boss))
    show("sound create upper case type", jcall(base, "POST", path, sound_body(upper, "Upper", "U", 0, 1), boss))
    for label, kw in [("name 32", dict(name="n" * 32)), ("name 33", dict(name="n" * 33)), ("name blank", dict(name="  ")), ("name null", dict(name=None)),
                      ("name trimmed", dict(name="  pad  ")), ("name emoji x16", dict(name="\U0001F680" * 16)), ("name emoji x17", dict(name="\U0001F680" * 17)),
                      ("glyph 16", dict(glyph="g" * 16)), ("glyph 17", dict(glyph="g" * 17)), ("glyph 8 emoji", dict(glyph="\U0001F680" * 8)), ("glyph 9 emoji", dict(glyph="\U0001F680" * 9)),
                      ("glyph blank", dict(glyph=" ")), ("glyph null", dict(glyph=None)),
                      ("trim negative start", dict(start=-0.001)), ("trim negative zero", dict(start=-0.0, end=1)), ("trim too short", dict(start=0, end=0.149)), ("trim shortest", dict(start=0, end=0.15)),
                      ("trim longest", dict(start=0, end=15.01)), ("trim too long", dict(start=0, end=15.011)), ("trim at the end", dict(start=5.0, end=20.0)),
                      ("trim past the end", dict(start=5.0, end=20.011)), ("trim tolerance", dict(start=5.01, end=20.01)), ("trim rounding", dict(start=0.0004, end=1.0005)),
                      ("trim half even", dict(start=0.0005, end=1.0015)), ("trim reversed", dict(start=2, end=1)), ("trim huge", dict(start=0, end=1e300)),
                      ("trim ints", dict(start=1, end=2)), ("trim strings", dict(start="1", end="2")), ("trim null", dict(start=None, end=1)), ("no trim", None)]:
        if kw is None:
            body = {"attachmentId": b, "name": "n", "glyph": "g"}
        else:
            kw = dict(kw)
            start = kw.pop("start", 0)
            end = kw.pop("end", 1)
            body = {"attachmentId": b, "name": kw.get("name", "n"), "glyph": kw.get("glyph", "g"), "trimStartSeconds": start, "trimEndSeconds": end}
        answer = show(f"sound create {label}", jcall(base, "POST", path, body, boss))
        # Free the clip again so that the next case can use it.
        if answer.get("id"):
            send(base, "DELETE", f"{path}/{answer['id']}", None, None, boss)
            b = wave(base, boss, "20")
    for label, body, ctype in [("not json", b"{", "application/json"), ("array", b"[]", "application/json"), ("null", b"null", "application/json"), ("empty", b"", "application/json"),
                               ("form type", b"{}", "text/plain"), ("no type", b"{}", None), ("duplicate member", ('{"attachmentId":"%s","name":"d","name":"dd","glyph":"g","trimEndSeconds":1}' % c).encode(), "application/json"),
                               ("bad id", b'{"attachmentId":"x"}', "application/json"), ("id braces", ('{"attachmentId":"{%s}"}' % c).encode(), "application/json"),
                               ("id without hyphens", ('{"attachmentId":"%s"}' % c.replace("-", "")).encode(), "application/json"), ("name number", b'{"name":1}', "application/json"),
                               ("charset", ('{"attachmentId":"%s","name":"c","glyph":"g","trimEndSeconds":1}' % c).encode(), "application/json; charset=utf-8"), ("case", ('{"AttachmentId":"%s","Name":"c","Glyph":"g","TrimEndSeconds":1}' % c).encode(), "application/json")]:
        show(f"sound create body {label}", raw_json(base, "POST", path, body, boss, ctype))
    show("sound create bare 1 MiB", raw_json(base, "POST", path, b'{"name":"' + b"a" * (1024 * 1024) + b'"}', boss))
    show("board after create", send(base, "GET", "/api/soundboard", None, None, boss))
    second = show("sound create second", jcall(base, "POST", path, sound_body(c, "Ping", "P", 0.5, 2.5), boss))
    third = show("sound create third", jcall(base, "POST", path, sound_body(d, "Long", "L", 3, 18), boss))
    s2, s3 = second.get("id", ""), third.get("id", "")
    # Changing.
    show("sound update", jcall(base, "PATCH", f"{path}/{sid}", {"name": "Rakieta", "trimStartSeconds": 2.0, "trimEndSeconds": 4.0}, boss))
    show("sound update nothing", jcall(base, "PATCH", f"{path}/{sid}", {}, boss))
    show("sound update trim only start", jcall(base, "PATCH", f"{path}/{sid}", {"trimStartSeconds": 1.005}, boss))
    show("sound update midpoint trim", jcall(base, "PATCH", f"{path}/{sid}", {"trimStartSeconds": 2.675, "trimEndSeconds": 8.255}, boss))
    show("sound update glyph", jcall(base, "PATCH", f"{path}/{sid}", {"glyph": " ❤ "}, boss))
    show("sound update same", jcall(base, "PATCH", f"{path}/{sid}", {"name": "Rakieta", "glyph": "❤"}, boss))
    for label, body in [("blank name", {"name": " "}), ("long name", {"name": "n" * 33}), ("null name", {"name": None}), ("trim bad", {"trimEndSeconds": 30}),
                        ("trim past clip", {"trimStartSeconds": 0, "trimEndSeconds": 9}), ("trim too short", {"trimEndSeconds": 8.3, "trimStartSeconds": 8.26}), ("trim null", {"trimStartSeconds": None}),
                        ("trim string", {"trimStartSeconds": "1"})]:
        show(f"sound update {label}", jcall(base, "PATCH", f"{path}/{sid}", body, boss))
    show("sound update member", jcall(base, "PATCH", f"{path}/{sid}", {"name": "x"}, maja))
    show("sound update outsider", jcall(base, "PATCH", f"{path}/{sid}", {"name": "x"}, outsider))
    show("sound update anonymous", jcall(base, "PATCH", f"{path}/{sid}", {"name": "x"}, None))
    show("sound update unknown", jcall(base, "PATCH", f"{path}/{sid[:-1]}0", {"name": "x"}, boss))
    show("sound update member unknown", jcall(base, "PATCH", f"{path}/{sid[:-1]}0", {"name": "x"}, maja))
    show("sound update not an id", jcall(base, "PATCH", f"{path}/x", {"name": "x"}, boss))
    show("sound update not an id anonymous", jcall(base, "PATCH", f"{path}/x", {"name": "x"}, None))
    show("sound update bad body", raw_json(base, "PATCH", f"{path}/{sid}", b"[", boss))
    # Order.
    for label, body in [("incomplete", {"soundIds": [sid, s2]}), ("repeats", {"soundIds": [sid, sid, s2]}), ("unknown", {"soundIds": [sid, s2, s3, "00000000-0000-0000-0000-000000000009"]}),
                        ("empty", {"soundIds": []}), ("missing", {}), ("null", {"soundIds": None}), ("null element", {"soundIds": [sid, s2, None]}), ("bad element", {"soundIds": [sid, s2, "x"]}),
                        ("reordered", {"soundIds": [s3, sid, s2]}), ("same", {"soundIds": [s3, sid, s2]})]:
        show(f"sound order {label}", jcall(base, "PUT", f"{path[:-7]}/order", body, boss))
    show("sound order member", jcall(base, "PUT", f"{path[:-7]}/order", {"soundIds": [s3, sid, s2]}, maja))
    show("sound order anonymous", jcall(base, "PUT", f"{path[:-7]}/order", {"soundIds": [s3, sid, s2]}, None))
    show("board after order", send(base, "GET", "/api/soundboard", None, None, maja))
    # Settings.
    sp = f"{path[:-7]}/settings"
    for label, body, cookie in [("empty", {}, boss), ("off", {"everyoneCanPlay": False, "rateLimited": True}, boss), ("same", {"everyoneCanPlay": False}, boss),
                                ("null", {"everyoneCanPlay": None, "rateLimited": None}, boss), ("string", {"everyoneCanPlay": "yes"}, boss), ("member", {"everyoneCanPlay": True}, maja),
                                ("outsider", {"everyoneCanPlay": True}, outsider), ("anonymous", {"everyoneCanPlay": True}, None), ("array", [], boss), ("on", {"everyoneCanPlay": True, "rateLimited": False}, boss)]:
        show(f"sound settings {label}", jcall(base, "PATCH", sp, body, cookie))
    show("sound settings not json", raw_json(base, "PATCH", sp, b"{", boss))
    show("board member after settings", send(base, "GET", "/api/soundboard", None, None, maja))
    # The join sound.
    show("join sound library empty selection", send(base, "GET", "/api/me/join-sound", None, None, maja))
    show("join sound library anonymous", send(base, "GET", "/api/me/join-sound", None, None, None))
    show("join sound library outsider", send(base, "GET", "/api/me/join-sound", None, None, outsider))
    prefs = "/api/me/preferences"
    show("join sound choose", jcall(base, "PATCH", prefs, {"joinSoundId": sid}, maja))
    show("join sound library chosen", send(base, "GET", "/api/me/join-sound", None, None, maja))
    show("join sound choose unknown", jcall(base, "PATCH", prefs, {"joinSoundId": "00000000-0000-0000-0000-000000000009"}, maja))
    show("join sound choose outsider", jcall(base, "PATCH", prefs, {"joinSoundId": sid}, outsider))
    show("join sound choose bad id", jcall(base, "PATCH", prefs, {"joinSoundId": "x"}, maja))
    show("join sound choose braces", jcall(base, "PATCH", prefs, {"joinSoundId": "  {" + s2 + "}  "}, maja))
    show("join sound blank", jcall(base, "PATCH", prefs, {"joinSoundId": "   "}, maja))
    show("join sound null", jcall(base, "PATCH", prefs, {"joinSoundId": None}, maja))
    show("join sound after blank", send(base, "GET", "/api/me", None, None, maja))
    show("join sound choose again", jcall(base, "PATCH", prefs, {"joinSoundId": sid}, maja))
    show("join sound owner chooses", jcall(base, "PATCH", prefs, {"joinSoundId": s3}, boss))
    # Deleting.
    show("sound delete member", send(base, "DELETE", f"{path}/{sid}", None, None, maja))
    show("sound delete outsider", send(base, "DELETE", f"{path}/{sid}", None, None, outsider))
    show("sound delete anonymous", send(base, "DELETE", f"{path}/{sid}", None, None, None))
    show("sound delete not an id", send(base, "DELETE", f"{path}/x", None, None, boss))
    show("sound delete unknown", send(base, "DELETE", f"{path}/{sid[:-1]}0", None, None, boss))
    show("sound delete member unknown", send(base, "DELETE", f"{path}/{sid[:-1]}0", None, None, maja))
    show("sound delete", send(base, "DELETE", f"{path}/{sid}", None, None, boss))
    show("sound delete again", send(base, "DELETE", f"{path}/{sid}", None, None, boss))
    show("join sound after delete", send(base, "GET", "/api/me", None, None, maja))
    describe("sound clip after delete", send(base, "GET", f"/assets/{a}", None, None, boss), 100)
    show("board at the end", send(base, "GET", "/api/soundboard", None, None, boss))
    show("join sound library at the end", send(base, "GET", "/api/me/join-sound", None, None, boss))


def account_corpus(base, boss, maja, outsider):
    show("me", send(base, "GET", "/api/me", None, None, maja))
    for label, body in [("name", {"displayName": "  Maja Nowak  "}), ("nothing", {}), ("email", {"email": "x@y.z"}), ("email empty", {"email": ""}), ("email null", {"email": None}),
                        ("email and name", {"email": "x@y.z", "displayName": "X"}), ("empty name", {"displayName": " "}), ("long name", {"displayName": "n" * 65}),
                        ("name at 64", {"displayName": "n" * 64}), ("name emoji 32", {"displayName": "\U0001F680" * 32}), ("name emoji 33", {"displayName": "\U0001F680" * 33}),
                        ("all too long", {"displayName": " ", "bio": "b" * 2001, "customStatus": "s" * 129, "pronouns": "p" * 33}),
                        ("bio 2000", {"bio": "b" * 2000}), ("status 128", {"customStatus": "s" * 128}), ("pronouns 32", {"pronouns": "p" * 32}),
                        ("clear", {"bio": "  ", "customStatus": "", "pronouns": "\t"}), ("set", {"bio": " Hello ", "customStatus": " busy ", "pronouns": " she/her "}),
                        ("null fields", {"bio": None, "displayName": None}), ("number", {"displayName": 5}), ("username", {"username": "other"}),
                        ("name whitespace kinds", {"displayName": "  x　"}), ("array", []), ("string", "x")]:
        show(f"me patch {label}", jcall(base, "PATCH", "/api/me", body, maja))
    show("me patch anonymous", jcall(base, "PATCH", "/api/me", {"displayName": "x"}, None))
    show("me patch not json", raw_json(base, "PATCH", "/api/me", b"{", maja))
    show("me patch text type", raw_json(base, "PATCH", "/api/me", b"{}", maja, "text/plain"))
    show("me after", send(base, "GET", "/api/me", None, None, maja))
    # Preferences.
    prefs = "/api/me/preferences"
    show("prefs patch nothing", jcall(base, "PATCH", prefs, {}, boss))
    show("prefs fresh account", send(base, "GET", "/api/me", None, None, outsider))
    for label, body in [("all off", {"desktopNotifications": False, "emailNotifications": False, "pushNotifications": False, "mentionsOnly": False, "messageSound": False, "joinLeaveSound": False,
                                      "allowDirectMessages": False, "allowFriendRequests": False, "developerMode": True, "reducedMotion": True, "mentionPopups": False, "callRingtone": False}),
                        ("vad 0", {"vadSensitivity": 0}), ("vad 100", {"vadSensitivity": 100}), ("vad 101", {"vadSensitivity": 101}), ("vad -1", {"vadSensitivity": -1}),
                        ("vad float", {"vadSensitivity": 50.5}), ("vad big", {"vadSensitivity": 3000000000}), ("vad string", {"vadSensitivity": "5"}), ("vad null", {"vadSensitivity": None}),
                        ("muted", {"mutedChannelIds": ["20000000-0000-0000-0000-000000000002", "20000000-0000-0000-0000-000000000002", "20000000-0000-0000-0000-000000000003"]}),
                        ("muted empty", {"mutedChannelIds": []}), ("muted bad", {"mutedChannelIds": ["x"]}), ("muted unhyphenated", {"mutedChannelIds": ["20000000000000000000000000000002"]}),
                        ("bool string", {"developerMode": "true"}), ("bool number", {"developerMode": 1}), ("presence off", {"showPresence": False}), ("presence on", {"showPresence": True}),
                        ("vad and bad sound", {"vadSensitivity": 101, "joinSoundId": "x"}), ("bad sound and good", {"joinSoundId": "x", "vadSensitivity": 5}),
                        ("array", []), ("null body", None)]:
        show(f"prefs patch {label}", jcall(base, "PATCH", prefs, body, maja))
    show("prefs patch anonymous", jcall(base, "PATCH", prefs, {}, None))
    show("prefs patch not json", raw_json(base, "PATCH", prefs, b"{", maja))
    show("prefs after", send(base, "GET", "/api/me", None, None, maja))


def settings_corpus(base, boss, maja, outsider):
    path = "/api/me/settings"
    res = show("settings empty", send(base, "GET", path, None, None, outsider))
    for section in ["appearance", "language", "keybinds", "voice", "privacy", "Appearance", "VOICE", "passwords", "", "voice%20", "%76oice", "voice/", "a%2Fb", "settings"]:
        show(f"settings put {section!r}", raw_json(base, "PUT", f"{path}/{section}", b'{"a":1}', outsider))
    bodies = [("simple", b'{ "palette": "ember", "size": "lg" }'), ("whitespace", b" \n\t{ }\r\n "), ("array", b"[1, 2]"), ("string", b'"dark"'), ("number", b"1"), ("null", b"null"),
              ("true", b"true"), ("broken", b"{ not json"), ("empty", b""), ("space", b"   "), ("trailing", b'{"a":1} x'), ("two", b'{"a":1}{"b":2}'), ("comment", b'{"a":1 /* c */}'),
              ("trailing comma", b'{"a":1,}'), ("single quotes", b"{'a':1}"), ("nan", b'{"a":NaN}'), ("number forms", b'{"a":[1.0,-0,1E5,2e+3,0.10,1e999,00]}'),
              ("number forms ok", b'{"a":[1.0,-0,1E5,2e+3,0.10,1e999,123456789012345678901234567890,-1.5e-7]}'), ("keys", b'{"b":1,"a":2,"b":3}'),
              ("escapes", '{"a":"é<a href=\'x\'>&\\n\\t\\"\\\\\\/\\u0041\\ud83d\\ude80 \U0001F680 \x7f  "}'.encode()),
              ("key escapes", '{"é<":1}'.encode()), ("control char", b'{"a":"\x01"}'), ("lone surrogate", b'{"a":"\\ud800"}'), ("lone low surrogate", b'{"a":"\\udc00"}'),
              ("bad escape", b'{"a":"\\x"}'), ("short escape", b'{"a":"\\u12"}'), ("bom", b'\xef\xbb\xbf{"a":1}'), ("utf16 bom", b'\xff\xfe' + '{"a":"é"}'.encode("utf-16-le")),
              ("utf16 be bom", b'\xfe\xff' + '{"a":"é"}'.encode("utf-16-be")), ("utf32 bom", b'\xff\xfe\x00\x00' + '{"a":"é"}'.encode("utf-32-le")),
              ("invalid utf8", b'{"a":"\xff\xfe"}'), ("nul", b'{"a":"\x00"}'), ("depth 64", b'{"a":' * 63 + b"1" + b"}" * 63), ("depth 64 exact", b'{"a":' * 64 + b"1" + b"}" * 64),
              ("depth 65", b'{"a":' * 65 + b"1" + b"}" * 65), ("arrays 63", b'{"a":' + b"[" * 63 + b"]" * 63 + b"}"), ("arrays 64", b'{"a":' + b"[" * 64 + b"]" * 64 + b"}"),
              ("at the limit", b'{"a":"' + b"x" * (8192 - 8) + b'"}'), ("over the limit", b'{"a":"' + b"x" * (8192 - 7) + b'"}'),
              ("escapes at the limit", b'{"a":"' + "é".encode() * ((8192 - 8) // 6) + b'"}'), ("escapes over the limit", b'{"a":"' + "é".encode() * ((8192 - 8) // 6 + 1) + b'"}'),
              ("padding", b"{" + b" " * 9000 + b'"palette": "ember"}'), ("body 32768", b'{"a":1}' + b" " * (32768 - 7)), ("body 32769", b'{"a":1}' + b" " * (32769 - 7)),
              ("body 33000 padded", b'{"a":1}' + b" " * 33000), ("big", b'{"a":"' + b"x" * 200000 + b'"}'), ("big 2 MiB", b'{"a":"' + b"x" * (2 * 1024 * 1024) + b'"}')]
    for label, body in bodies:
        answer = raw_json(base, "PUT", f"{path}/voice", body, maja)
        describe(f"settings voice {label}", answer, 300)
        describe(f"settings read after {label}", send(base, "GET", path, None, None, maja), 400)
    for ctype in [None, "text/plain", "application/x-www-form-urlencoded", "application/json; charset=utf-16"]:
        describe(f"settings type {ctype}", raw_json(base, "PUT", f"{path}/privacy", b'{"giphy":false}', maja, ctype), 300)
    chunked = b"7\r\n{\"a\":1}\r\n0\r\n\r\n"
    describe("settings chunked", send(base, "PUT", f"{path}/language", chunked, {"Content-Type": "application/json", "Transfer-Encoding": "chunked"}, maja), 300)
    big = b'{"a":"' + b"x" * 40000 + b'"}'
    chunks = b"".join(b"%x\r\n" % len(big[i:i + 4000]) + big[i:i + 4000] + b"\r\n" for i in range(0, len(big), 4000)) + b"0\r\n\r\n"
    describe("settings chunked big", send(base, "PUT", f"{path}/language", chunks, {"Content-Type": "application/json", "Transfer-Encoding": "chunked"}, maja), 300)
    describe("settings put anonymous", raw_json(base, "PUT", f"{path}/voice", b"{}", None), 300)
    describe("settings get anonymous", send(base, "GET", path, None, None, None), 300)
    for section in ["appearance", "language", "keybinds", "privacy"]:
        describe(f"settings fill {section}", raw_json(base, "PUT", f"{path}/{section}", ('{"s":"%s"}' % section).encode(), maja), 300)
    for method in ["POST", "PATCH", "DELETE"]:
        describe(f"settings {method}", raw_json(base, method, f"{path}/voice", b"{}", maja), 300)
    describe("settings read all", send(base, "GET", path, None, None, maja), 2000)
    describe("settings read as another", send(base, "GET", path, None, None, boss), 2000)
    describe("settings head", send(base, "HEAD", path, None, None, maja), 300)
    describe("settings cached", send(base, "GET", path, None, {"If-None-Match": "*"}, maja), 300)
    # Two saves of one section at once.
    results = []

    def go(i):
        results.append(raw_json(base, "PUT", f"{path}/appearance", ('{"who":%d}' % i).encode(), boss)[0])
    threads = [threading.Thread(target=go, args=(i,)) for i in range(6)]
    [t.start() for t in threads]
    [t.join() for t in threads]
    print("settings race |", sorted(results))
    # Which of the saves came last is not for the probe to say.
    raw_json(base, "PUT", f"{path}/appearance", b'{"who":"last"}', boss)
    # A fresh account saves everything at once.
    results = []

    def one(section):
        results.append(raw_json(base, "PUT", f"{path}/{section}", b'{"v":1}', outsider)[0])
    threads = [threading.Thread(target=one, args=(s,)) for s in ["appearance", "language", "keybinds", "voice", "privacy"]]
    [t.start() for t in threads]
    [t.join() for t in threads]
    print("settings fresh account all at once |", sorted(results))
    describe("settings read after the race", send(base, "GET", path, None, None, outsider), 1000)


def main():
    base, setup = sys.argv[1], sys.argv[2]
    boss, _ = probe.register(base, "boss", {"setupCode": setup})
    if len(sys.argv) > 3:
        invites = [probe.invite_in(sys.argv[3]) for _ in range(2)]
    else:
        invites = [json.loads(jcall(base, "POST", "/api/admin/invites", {}, boss)[2])["url"].rsplit("/", 1)[1] for _ in range(2)]
    maja, _ = probe.register(base, "maja", {"inviteCode": invites[0]})
    jo, _ = probe.register(base, "joe", {"inviteCode": invites[1]})
    outsider = maja + OUTSIDER
    sections = [s for s in os.environ.get("CUSTOM_PROBE_ONLY", "emoji,soundboard,account,settings").split(",")]
    if "emoji" in sections:
        emoji_corpus(base, boss, maja, outsider)
    if "soundboard" in sections:
        soundboard_corpus(base, boss, maja, outsider, None)
    if "account" in sections:
        account_corpus(base, boss, maja, jo)
    if "settings" in sections:
        settings_corpus(base, boss, maja, jo)


if __name__ == "__main__":
    main()
