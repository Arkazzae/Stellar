"""Makes the gzip bodies that tests/fetch.rs serves, with Python's gzip,
which is independent of the decoder under test:

    python3 -I tests/fixtures/gzip.py tests/fixtures/gzip

Deterministic (no timestamps), so running it again changes nothing.
"""
import gzip
import os
import sys

MIB = 1 << 20
PNG = b"\x89PNG\r\n\x1a\n\x00\x01\x02\x03"


def main(directory):
    os.makedirs(directory, exist_ok=True)

    def write(name, data):
        with open(os.path.join(directory, name), "wb") as handle:
            handle.write(gzip.compress(data, 9, mtime=0))

    write("page.html.gz", (
        '<html><head><title>Fallback</title>'
        '<meta property="og:title" content="Compressed card">'
        '<meta property="og:description" content="Said in gzip">'
        "</head></html>").encode())
    # The description sits past a cap of 1024 bytes.
    write("long-page.html.gz", (
        '<html><head><meta property="og:title" content="Streamed">' + " " * 2048 +
        '<meta property="og:description" content="Past the cap"></head></html>').encode())
    write("picture.png.gz", PNG)
    write("big-picture.png.gz", PNG + bytes(4096))
    # Small on the wire, tens of mebibytes decoded.
    write("bomb-picture.png.gz", PNG + bytes(8 * MIB))
    write("bomb-page.html.gz", b"<title>Bomb</title>" + b" " * (8 * MIB))


if __name__ == "__main__":
    main(sys.argv[1])
