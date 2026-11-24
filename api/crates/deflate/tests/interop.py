"""The other side of the interoperability tests: Python's zlib and gzip are the
independent implementation the Rust code is checked against.

    interop.py compress DIR   for every NAME.raw in DIR, write NAME.py.*.def
                              (raw deflate at every level and strategy) and
                              the zlib and gzip forms
    interop.py verify DIR     for every NAME.raw, decode NAME.mine.*.def and
                              NAME.mine.*.zz and check the bytes are the same
"""
import glob
import gzip
import os
import sys
import zlib


def raw_deflate(data, level, strategy=zlib.Z_DEFAULT_STRATEGY, mem=8, chunk=None, flush=None):
    c = zlib.compressobj(level, zlib.DEFLATED, -15, mem, strategy)
    out = []
    if chunk is None:
        out.append(c.compress(data))
    else:
        for i in range(0, len(data), chunk):
            out.append(c.compress(data[i:i + chunk]))
            out.append(c.flush(flush))
    out.append(c.flush())
    return b"".join(out)


def variants(data):
    for level in range(10):
        yield "L%d" % level, raw_deflate(data, level)
    for level in (1, 6, 9):
        yield "filtered%d" % level, raw_deflate(data, level, zlib.Z_FILTERED)
        yield "fixed%d" % level, raw_deflate(data, level, zlib.Z_FIXED)
    yield "huffman", raw_deflate(data, 6, zlib.Z_HUFFMAN_ONLY)
    yield "rle", raw_deflate(data, 6, zlib.Z_RLE)
    yield "mem1", raw_deflate(data, 6, mem=1)
    yield "mem9", raw_deflate(data, 6, mem=9)
    yield "sync", raw_deflate(data, 6, chunk=777, flush=zlib.Z_SYNC_FLUSH)
    yield "full", raw_deflate(data, 9, chunk=40000, flush=zlib.Z_FULL_FLUSH)


def compress(directory):
    count = 0
    for path in sorted(glob.glob(os.path.join(directory, "*.raw"))):
        name = path[:-4]
        data = open(path, "rb").read()
        for label, stream in variants(data):
            open("%s.py.%s.def" % (name, label), "wb").write(stream)
            count += 1
        open("%s.py.zlib" % name, "wb").write(zlib.compress(data, 6))
        open("%s.py.gz" % name, "wb").write(gzip.compress(data, 6, mtime=0))
        half = len(data) // 2
        open("%s.py.gzmulti" % name, "wb").write(
            gzip.compress(data[:half], 1, mtime=5) + gzip.compress(data[half:], 9, mtime=7))
    print("compressed", count)


def verify(directory):
    count = 0
    for path in sorted(glob.glob(os.path.join(directory, "*.raw"))):
        name = path[:-4]
        data = open(path, "rb").read()
        for stream_path in sorted(glob.glob(glob.escape(name) + ".mine.*.def")):
            stream = open(stream_path, "rb").read()
            d = zlib.decompressobj(-15)
            out = d.decompress(stream)
            assert d.eof, "%s: stream did not end" % stream_path
            assert d.unused_data == b"", "%s: bytes after the end" % stream_path
            assert out == data, "%s: decoded bytes differ" % stream_path
            count += 1
        for stream_path in sorted(glob.glob(glob.escape(name) + ".mine.*.zz")):
            assert zlib.decompress(open(stream_path, "rb").read()) == data, stream_path
            count += 1
    print("verified", count)


if __name__ == "__main__":
    {"compress": compress, "verify": verify}[sys.argv[1]](sys.argv[2])
