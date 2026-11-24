"""The other side of the archive interoperability tests: Python's zipfile and
the zip command write archives for the Rust reader, and zipfile checks the
archives the Rust writer made.

    interop.py make DIR      DIR/inputs holds the files to pack; write
                             py-*.zip (zipfile) and cli-*.zip (zip) in DIR
    interop.py check DIR     DIR/inputs holds the files that went in; check
                             every DIR/mine-*.zip with testzip() and compare
                             each entry with the file of its name
"""
import glob
import os
import subprocess
import sys
import zipfile


class Unseekable:
    """A sink zipfile cannot seek in, so it writes data descriptors."""

    def __init__(self, path):
        self.f = open(path, "wb")

    def write(self, data):
        return self.f.write(data)

    def flush(self):
        self.f.flush()

    def close(self):
        self.f.close()


def files_of(inputs):
    found = []
    for root, _dirs, names in os.walk(inputs):
        for name in sorted(names):
            path = os.path.join(root, name)
            found.append((os.path.relpath(path, inputs).replace(os.sep, "/"), path))
    return sorted(found)


def make(directory):
    inputs = os.path.join(directory, "inputs")
    files = files_of(inputs)

    for label, method in (("stored", zipfile.ZIP_STORED), ("deflated", zipfile.ZIP_DEFLATED)):
        with zipfile.ZipFile(os.path.join(directory, "py-%s.zip" % label), "w", method) as zf:
            for name, path in files:
                zf.write(path, name)

    with zipfile.ZipFile(os.path.join(directory, "py-zip64.zip"), "w", zipfile.ZIP_DEFLATED) as zf:
        for name, path in files:
            info = zipfile.ZipInfo(name, date_time=(2026, 11, 24, 1, 2, 4))
            info.compress_type = zipfile.ZIP_DEFLATED
            with zf.open(info, "w", force_zip64=True) as out:
                out.write(open(path, "rb").read())

    sink = Unseekable(os.path.join(directory, "py-stream.zip"))
    with zipfile.ZipFile(sink, "w", zipfile.ZIP_DEFLATED) as zf:
        for name, path in files:
            with zf.open(name, "w") as out:
                out.write(open(path, "rb").read())
    sink.close()

    with zipfile.ZipFile(os.path.join(directory, "py-comment.zip"), "w", zipfile.ZIP_STORED) as zf:
        zf.comment = b"PK\x05\x06 an archive comment that looks like a record"
        for name, path in files:
            zf.write(path, name)
        zf.writestr("unicode/üñî.txt", "text")

    # More entries than the 16-bit count of the end record holds.
    with zipfile.ZipFile(os.path.join(directory, "py-many.zip"), "w", zipfile.ZIP_STORED) as zf:
        for i in range(70000):
            zf.writestr("many/%05d" % i, b"" if i % 3 else b"x")

    # Exactly as many as a 16-bit count holds: Python writes no ZIP64 record.
    with zipfile.ZipFile(os.path.join(directory, "py-65535.zip"), "w", zipfile.ZIP_STORED) as zf:
        for i in range(65535):
            zf.writestr("n/%05d" % i, b"")

    for label, method in (("bzip2", zipfile.ZIP_BZIP2), ("lzma", zipfile.ZIP_LZMA)):
        with zipfile.ZipFile(os.path.join(directory, "py-%s.zip" % label), "w", method) as zf:
            zf.writestr("a.txt", b"hello " * 100)

    def cli(name, *args, cwd=directory, sources=("inputs",)):
        subprocess.run(["zip", "-q", "-r", name, *args, *sources], cwd=cwd, check=True)

    cli("cli-deflated.zip")
    cli("cli-stored.zip", "-0")
    cli("cli-best.zip", "-9")
    cli("cli-zip64.zip", "-fz")
    cli("cli-encrypted.zip", "-P", "secret")
    # Streamed from a pipe, so with data descriptors.
    with open(os.path.join(directory, "cli-pipe.zip"), "wb") as out:
        subprocess.run("cat inputs/small.txt | zip -q - -", shell=True, cwd=directory, stdout=out, check=True)
    # A symbolic link kept as one.
    link_dir = os.path.join(directory, "links")
    os.makedirs(link_dir, exist_ok=True)
    if not os.path.lexists(os.path.join(link_dir, "link")):
        os.symlink("../../etc/passwd", os.path.join(link_dir, "link"))
    subprocess.run(["zip", "-q", "-y", "-r", "cli-symlink.zip", "links"], cwd=directory, check=True)
    print("made", len(glob.glob(os.path.join(directory, "*.zip"))))


def check(directory):
    inputs = os.path.join(directory, "inputs")
    count = 0
    for path in sorted(glob.glob(os.path.join(directory, "mine-*.zip"))):
        with zipfile.ZipFile(path) as zf:
            bad = zf.testzip()
            assert bad is None, "%s: bad entry %s" % (path, bad)
            for info in zf.infolist():
                if info.filename.endswith("/"):
                    continue
                source = os.path.join(inputs, info.filename)
                if os.path.exists(source):
                    assert zf.read(info) == open(source, "rb").read(), "%s: %s differs" % (path, info.filename)
                count += 1
    print("checked", count)


if __name__ == "__main__":
    {"make": make, "check": check}[sys.argv[1]](sys.argv[2])
