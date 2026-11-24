"""Derives every encoding of an Ed25519 point of small order, with plain
integer arithmetic, and checks the table in src/cose.rs against it.

    python3 -I ed25519_small_order.py

The curve is -x^2 + y^2 = 1 + d x^2 y^2 over GF(2^255 - 19) (RFC 8032). A
point is small order when eight times it is the identity (0, 1). The script
finds the eight such points without trusting any remembered constant: it
takes points of the curve, multiplies them by the order l of the prime
subgroup (which leaves only the part of order dividing 8) and collects what
comes out. It then proves 8*P == identity for each, lists every spelling a
decoder may accept (a y at or above p when it still fits in 255 bits, and a
sign bit set where x is 0) and compares the list with the table in cose.rs.
"""
import pathlib
import re

p = 2**255 - 19
l = 2**252 + 27742317777372353535851937790883648493
d = (-121665 * pow(121666, -1, p)) % p
I = pow(2, (p - 1) // 4, p)  # sqrt(-1)


def add(a, b):
    (x1, y1), (x2, y2) = a, b
    t = d * x1 * x2 * y1 * y2 % p
    x3 = (x1 * y2 + x2 * y1) * pow(1 + t, -1, p) % p
    y3 = (y1 * y2 + x1 * x2) * pow(1 - t, -1, p) % p
    return (x3, y3)


def mul(k, pt):
    r = (0, 1)
    while k:
        if k & 1:
            r = add(r, pt)
        pt = add(pt, pt)
        k >>= 1
    return r


def on_curve(pt):
    x, y = pt
    return (-x * x + y * y - 1 - d * x * x * y * y) % p == 0


def x_from_y(y):
    u = (y * y - 1) % p
    v = (d * y * y + 1) % p
    x = pow(u * pow(v, -1, p), (p + 3) // 8, p)
    if (v * x * x - u) % p:
        x = x * I % p
    if (v * x * x - u) % p:
        return None
    return x


identity = (0, 1)
torsion = {identity}
y = 2
while len(torsion) < 8:
    x = x_from_y(y)
    y += 1
    if x is None:
        continue
    t = mul(l, (x, y - 1))
    assert on_curve(t)
    if t not in torsion and mul(8, t) == identity:
        for k in range(8):
            torsion.add(mul(k, t))
assert len(torsion) == 8
for pt in torsion:
    assert on_curve(pt) and mul(8, pt) == identity

encodings = set()
for x, y in torsion:
    for y_spelling in (y, y + p):
        if y_spelling >= 2**255:
            continue
        for sign in (x & 1, 1 - (x & 1)):
            # A sign bit that disagrees with x is only a spelling of the same
            # point when x is 0 (there is no negative zero to pick).
            if sign != (x & 1) and x != 0:
                continue
            encodings.add((y_spelling | (sign << 255)).to_bytes(32, "little"))

table = sorted(encodings)
print(f"{len(table)} encodings of {len(torsion)} points")
for e in table:
    print("    [" + ", ".join(f"0x{b:02x}" for b in e) + "],")

source = (pathlib.Path(__file__).resolve().parents[2] / "src" / "cose.rs").read_text()
block = re.search(r"SMALL_ORDER_ED25519: \[\[u8; 32\]; \d+\] = \[(.*?)\n\];", source, re.S)
assert block, "table not found in cose.rs"
found = sorted(
    bytes(int(h, 16) for h in re.findall(r"0x([0-9a-f]{2})", row))
    for row in re.findall(r"\[([^\[\]]*)\]", block.group(1))
)
assert found == table, "the table in cose.rs differs from the derived one"
print("cose.rs table matches")
