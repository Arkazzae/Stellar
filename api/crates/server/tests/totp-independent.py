#!/usr/bin/env python3
"""An independent TOTP, for checking the one in src/auth/totp.rs.

Uses nothing but Python's hmac, hashlib, struct and base64. Two jobs:

  totp-independent.py fixture   > fixtures/totp-cases.tsv
      A fixed set of cases (seed 6238): secret, the moment, the code typed, and
      the step the server must report (or - for a refusal), worked out here.
      The Rust unit test `auth::totp::tests::an_independent_implementation_agrees`
      reads the file.

  totp-independent.py now SECRET [STEPS]
      The code an authenticator app shows now (STEPS later), for a script that
      has to answer a server with a real code.
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


if __name__ == "__main__":
    if sys.argv[1:2] == ["fixture"]:
        fixture()
    elif sys.argv[1:2] == ["now"]:
        import time
        secret = sys.argv[2].upper()
        key = base64.b32decode(secret + "=" * (-len(secret) % 8))
        print(hotp(key, int(time.time()) // 30 + (int(sys.argv[3]) if len(sys.argv) > 3 else 0)))
    else:
        sys.exit(__doc__)
