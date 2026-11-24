#!/usr/bin/env python3
"""A mail server that keeps what it is sent: every message goes to a file in
the folder given, named by the order it came in. Plain SMTP, no TLS."""
import socketserver, sys, pathlib, itertools

folder = pathlib.Path(sys.argv[1]); folder.mkdir(parents=True, exist_ok=True)
port = int(sys.argv[2])
counter = itertools.count(1)

class Handler(socketserver.StreamRequestHandler):
    def reply(self, text): self.wfile.write((text + "\r\n").encode()); self.wfile.flush()
    def handle(self):
        self.reply("220 sink ready")
        while True:
            line = self.rfile.readline()
            if not line: return
            cmd = line.decode(errors="replace").strip().upper()
            if cmd.startswith(("EHLO", "HELO")): self.reply("250 sink")
            elif cmd.startswith(("MAIL", "RCPT", "RSET", "NOOP")): self.reply("250 ok")
            elif cmd.startswith("DATA"):
                self.reply("354 go on")
                data = b""
                while True:
                    l = self.rfile.readline()
                    if l in (b".\r\n", b".\n", b""): break
                    data += l
                (folder / f"{next(counter):04d}.eml").write_bytes(data)
                self.reply("250 queued")
            elif cmd.startswith("QUIT"): self.reply("221 bye"); return
            else: self.reply("250 ok")

class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

Server(("0.0.0.0", port), Handler).serve_forever()
