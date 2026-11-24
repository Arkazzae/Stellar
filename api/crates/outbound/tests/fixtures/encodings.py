"""Makes the compressed answers that tests/encoding.rs and tests/bombs.rs
serve, with Python's gzip and zlib, which are independent of the decoder
under test. Run it from anywhere:

    python3 -I fixtures/encodings.py fixtures/encodings

Everything is deterministic (no timestamps, fixed levels), so running it
again changes nothing. The page is about 6 KB of text; the bombs are the
reason the script exists, since nothing else produces them cheaply.
"""
import gzip
import os
import sys
import zlib

GIB = 1 << 30
MIB = 1 << 20


def page():
    lines = []
    for i in range(120):
        lines.append("<p id=\"%d\">Stellarr link preview, line %d: %s</p>" % (
            i, i, "the quick brown fox jumps over the lazy dog " * (1 + i % 3)))
    return ("<!doctype html><title>Compressed page</title>\n" + "\n".join(lines) + "\n").encode()


def raw_deflate(data, level=6):
    c = zlib.compressobj(level, zlib.DEFLATED, -15)
    return c.compress(data) + c.flush()


def zeros_gzip(total):
    """gzip of `total` zero bytes, produced a mebibyte at a time."""
    c = zlib.compressobj(9, zlib.DEFLATED, 31)
    chunk = bytes(MIB)
    out = []
    for _ in range(total // MIB):
        out.append(c.compress(chunk))
    out.append(c.flush())
    return b"".join(out)


def main(directory):
    os.makedirs(directory, exist_ok=True)
    data = page()

    def write(name, content):
        with open(os.path.join(directory, name), "wb") as handle:
            handle.write(content)

    write("page.txt", data)
    write("page.gz", gzip.compress(data, 9, mtime=0))
    write("page.zlib", zlib.compress(data, 9))
    write("page.deflate", raw_deflate(data))
    write("page.gz.gz", gzip.compress(gzip.compress(data, 9, mtime=0), 9, mtime=0))
    # "Content-Encoding: deflate, gzip": zlib applied first, then gzip.
    write("page.zlib.gz", gzip.compress(zlib.compress(data, 9), 9, mtime=0))
    write("page.gz-multi", gzip.compress(data[:3000], 9, mtime=0) + gzip.compress(data[3000:], 1, mtime=0))
    write("empty.gz", gzip.compress(b"", 9, mtime=0))
    write("empty.zlib", zlib.compress(b"", 9))
    # One layer: 8 MiB of zeros in about 8 KB.
    write("bomb-8m.gz", zeros_gzip(8 * MIB))
    # Two layers: 4 GiB of zeros, which is about 4 MiB gzipped, gzipped
    # again into a few kilobytes. "Content-Encoding: gzip, gzip".
    inner = zeros_gzip(4 * GIB)
    write("bomb-4g.gz.gz", gzip.compress(inner, 9, mtime=0))
    print("wrote", len(os.listdir(directory)), "files")


if __name__ == "__main__":
    main(sys.argv[1])
