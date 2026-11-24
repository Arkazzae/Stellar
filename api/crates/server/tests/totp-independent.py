#!/usr/bin/env python3
"""An independent TOTP, for checking the one in src/auth/totp.rs.

Uses nothing but Python's hmac, hashlib, struct and base64. Two jobs:

  totp-independent.py fixture   > fixtures/totp-cases.tsv
      A fixed set of cases (seed 6238): secret, the moment, the code typed, and
      the step the server must report (or - for a refusal), worked out here.
      The Rust unit test `auth::totp::tests::an_independent_implementation_agrees`
      reads the file.

  totp-independent.py cases N SEED > cases.txt
      Lines for the differential runner (see src/auth/differential.rs): the
      same lines go to that runner and to a program built from the first
      server's own source, and the answers are diffed. Strings are written as
      hex of UTF-16BE, one operation per line.
"""
import base64
import hashlib
import hmac
import random
import struct
import sys

WINDOW = (0, -1, 1)  # the order of preference: now, before, after


def hotp(key: bytes, counter: int, digits: int = 6) -> str:
    mac = hmac.new(key, struct.pack(">q", counter), hashlib.sha1).digest()
    offset = mac[-1] & 0x0F
    number = struct.unpack(">I", mac[offset:offset + 4])[0] & 0x7FFFFFFF
    return str(number % 10**digits).zfill(digits)


def b32(key: bytes) -> str:
    return base64.b32encode(key).decode().rstrip("=")


def verify(key: bytes, code: str, now: int):
    """The step a six-digit code is accepted for, or None."""
    code = code.strip()
    if len(code) != 6 or not all(c in "0123456789" for c in code):
        return None
    present = now // 30
    for offset in WINDOW:
        if hotp(key, present + offset) == code:
            return present + offset
    return None


def hexed(text: str) -> str:
    return text.encode("utf-16-be", "surrogatepass").hex()


def fixture() -> None:
    rng = random.Random(6238)
    for _ in range(400):
        key = rng.randbytes(20)
        now = rng.randrange(1_500_000_000, 2_500_000_000)
        present = now // 30
        kind = rng.randrange(7)
        if kind < 5:
            offset = [0, -1, 1, -2, 2][kind]
            code = hotp(key, present + offset)
        elif kind == 5:
            code = " " + hotp(key, present) + "\t"
        else:
            code = "%06d" % rng.randrange(1_000_000)
        step = verify(key, code, now)
        print(f"{b32(key)}\t{now}\t{code.replace(chr(9), '<TAB>')}\t{'-' if step is None else step}")


def find_collisions(rng: random.Random, wanted: int):
    """Keys and moments where two neighbouring steps give the same code."""
    found = []
    while len(found) < wanted:
        key = rng.randbytes(20)
        base = rng.randrange(50_000_000, 80_000_000)
        codes = [hotp(key, base + i) for i in (-1, 0, 1)]
        if len(set(codes)) < 3:
            found.append((key, base * 30 + 7))
    return found


def cases(count: int, seed: int) -> None:
    rng = random.Random(seed)
    alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"
    junk = alphabet + alphabet.lower() + "=01890 -_/é١"
    for key, now in find_collisions(rng, 6):
        present = now // 30
        for offset in (-1, 0, 1):
            print("\t".join(["verify", hexed(b32(key)), hexed(hotp(key, present + offset)), str(now)]))
    for _ in range(count):
        key = rng.randbytes(20)
        now = rng.randrange(0, 4_000_000_000)
        present = now // 30
        secret = b32(key)
        offset = rng.choice([0, 0, -1, 1, -2, 2, 3])
        code = hotp(key, present + offset)
        mutation = rng.randrange(10)
        if mutation == 1:
            code = " \t" + code + "\n"
        elif mutation == 2:
            code = code[:5]
        elif mutation == 3:
            code = code + "0"
        elif mutation == 4:
            code = "%06d" % rng.randrange(1_000_000)
        elif mutation == 5:
            code = code[:3] + "٣" + code[4:]
        elif mutation == 6:
            code = " " + code + " "
        print("\t".join(["verify", hexed(secret), hexed(code), str(now)]))
        print("\t".join(["code", hexed(secret), str(now)]))
        print("\t".join(["b32enc", key.hex()]))
        text = "".join(rng.choice(junk) for _ in range(rng.randrange(0, 40)))
        print("\t".join(["b32dec", hexed(text)]))
        # A secret in lower case or padded, which a hand-typed one may be.
        print("\t".join(["b32dec", hexed(secret.lower() + "=" * rng.randrange(0, 5))]))
        bad = "".join(rng.choice(junk) for _ in range(rng.randrange(1, 40)))
        print("\t".join(["verify", hexed(bad), hexed(code), str(now)]))
        issuer = "".join(chr(rng.choice([rng.randrange(32, 127), rng.randrange(0xA0, 0x800), rng.randrange(0x4E00, 0x4F00), 0x1F600])) for _ in range(rng.randrange(1, 12)))
        account = "".join(chr(rng.choice([rng.randrange(32, 127), rng.randrange(0xA0, 0x800)])) for _ in range(rng.randrange(1, 12)))
        print("\t".join(["uri", hexed(issuer), hexed(account), hexed(secret)]))
        print("\t".join(["hash", hexed(code + issuer)]))
        print("\t".join(["normalize", hexed(account + code + issuer)]))
    # What Trim() takes off: every character of the basic plane that is not a
    # surrogate, and a few outside it.
    for point in list(range(0, 0xD800)) + list(range(0xE000, 0x10000)) + [0x10000, 0x1F600]:
        print("\t".join(["trim", hexed(chr(point))]))


if __name__ == "__main__":
    if sys.argv[1:2] == ["fixture"]:
        fixture()
    elif sys.argv[1:2] == ["cases"]:
        cases(int(sys.argv[2]), int(sys.argv[3]))
    else:
        sys.exit(__doc__)
