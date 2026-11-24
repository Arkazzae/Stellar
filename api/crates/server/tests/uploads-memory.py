#!/usr/bin/env python3
"""How much memory the server uses while uploads pass.

  uploads-memory.py <setup-code> <uploads-at-once> <size-in-MiB>

Needs a fresh server of this API on 127.0.0.1:5217 (its process is found by
name), founds the instance, sends that many uploads of that size at once, each
a video generated as it is sent, and prints the answers and the peak resident
memory of the server. The files are left in the data folder: remove them.
"""
import os, socket, subprocess, sys, threading, time, json
import importlib.util
spec = importlib.util.spec_from_file_location("probe", os.path.join(os.path.dirname(os.path.abspath(__file__)), "uploads-probe.py"))
probe = importlib.util.module_from_spec(spec); spec.loader.exec_module(probe)
base = "http://127.0.0.1:5217"
setup = sys.argv[1]; count = int(sys.argv[2]); size = int(sys.argv[3]) * 1024 * 1024
cookie, _ = probe.register(base, "boss", {"setupCode": setup})
pid = subprocess.check_output(["pgrep", "-f", "stellarr-api"]).split()[0].decode()
def rss(): return int(subprocess.check_output(["ps", "-o", "rss=", "-p", pid]).strip()) / 1024
peak = [rss()]; stop = [False]
def sampler():
    while not stop[0]:
        peak[0] = max(peak[0], rss()); time.sleep(0.05)
threading.Thread(target=sampler, daemon=True).start()
results = []
def one(i):
    head = (b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"v.mp4\"\r\nContent-Type: video/mp4\r\n\r\n"
            + bytes([0, 0, 0, 16]) + b"ftyp" + b"\1\2\3\4")
    tail = b"\r\n--B--\r\n"
    n = size - 12
    total = len(head) + n + len(tail)
    s = socket.create_connection(("127.0.0.1", 5217), timeout=600)
    h = f"POST /api/uploads HTTP/1.1\r\nHost: x\r\nCookie: {cookie}\r\nX-Stellarr-Client: web\r\nX-Stellarr-Client-Version: 2.0.0\r\nContent-Type: multipart/form-data; boundary=B\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
    s.sendall(h.encode() + head)
    block = os.urandom(1 << 20)
    sent = 0
    while sent < n:
        m = min(len(block), n - sent); s.sendall(block[:m]); sent += m
    s.sendall(tail)
    buf = b""
    while True:
        c = s.recv(65536)
        if not c: break
        buf += c
    results.append(buf.split(b"\r\n")[0].decode())
threads = [threading.Thread(target=one, args=(i,)) for i in range(count)]
t0 = time.time()
[t.start() for t in threads]; [t.join() for t in threads]
stop[0] = True
print(f"{count} uploads of {size>>20} MiB: {results} in {time.time()-t0:.1f}s; server RSS before {peak[0]:.0f} peak MiB (peak includes baseline)")
