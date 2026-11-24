import json, time, urllib.request, urllib.error, urllib.parse
PASSWORD = "Corona!2026-long"
CLIENT = {"X-Stellarr-Client": "web", "X-Stellarr-Client-Version": "2.0.0"}
class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k): return None
def call(base, method, path, body=None, cookie=None, headers=None, raw=False):
    while True:
        r = send(base, method, path, body, cookie, headers)
        if r[0] != 429: return r
        time.sleep(float(r[2].get("Retry-After") or 2) + 0.5)
def send(base, method, path, body=None, cookie=None, headers=None):
    path = urllib.parse.quote(path, safe="/?&=%:+{}()-_.~,;!*'$@[]")
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(base + path, method=method, data=data)
    h = dict(CLIENT)
    if body is not None: h["Content-Type"] = "application/json"
    h.update(headers or {})
    for k, v in h.items(): req.add_header(k, v)
    if cookie: req.add_header("Cookie", cookie)
    op = urllib.request.build_opener(NoRedirect)
    try:
        with op.open(req) as resp: return resp.status, resp.read(), resp.headers
    except urllib.error.HTTPError as e: return e.code, e.read(), e.headers
def cookie_of(headers):
    for line in headers.get_all("Set-Cookie") or []:
        first = line.split(";")[0]
        if "session" in first.split("=")[0]: return first
def login(base, name):
    s, b, h = call(base, "POST", "/api/auth/login", {"username": name, "password": PASSWORD})
    assert s == 200, (s, b)
    return cookie_of(h), json.loads(b)["id"]

import zlib, struct, uuid
def png(w, h, shade):
    raw = b"".join(b"\x00" + bytes([shade, (x * 7) % 256, 90] * 1) * 0 + b"\x00" + bytes([(shade + x) % 256, 80, (y * 5) % 256] * 1) for y in range(h) for x in range(w)) if False else b""
    rows = []
    for y in range(h):
        row = bytearray([0])
        for x in range(w):
            row += bytes([(shade + x) % 256, (80 + y) % 256, (x * y) % 256])
        rows.append(bytes(row))
    data = b"".join(rows)
    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(data)) + chunk(b"IEND", b"")
def upload(base, cookie, name, content, ctype="image/png", fields=None):
    boundary = uuid.uuid4().hex
    parts = []
    for k, v in (fields or {}).items():
        parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{k}"\r\n\r\n{v}\r\n'.encode())
    parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="{name}"\r\nContent-Type: {ctype}\r\n\r\n'.encode() + content + b"\r\n")
    body = b"".join(parts) + f"--{boundary}--\r\n".encode()
    req = urllib.request.Request(base + "/api/uploads", method="POST", data=body)
    for k, v in CLIENT.items(): req.add_header(k, v)
    req.add_header("Content-Type", "multipart/form-data; boundary=" + boundary)
    req.add_header("Cookie", cookie)
    try:
        with urllib.request.build_opener(NoRedirect).open(req) as r: return r.status, r.read()
    except urllib.error.HTTPError as e: return e.code, e.read()
