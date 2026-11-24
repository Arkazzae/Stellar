"""Writes openssl_fixtures.json: a registration and an assertion for six of
the algorithms, with every key and every signature made by the `openssl`
command line tool and the CBOR written by the small encoder below, so that
nothing in them came from the crate under test or from aws-lc-rs.

    python3 -I generate_openssl.py > openssl_fixtures.json
"""
import base64, hashlib, json, os, subprocess, tempfile


def b64u(b):
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode()


def head(major, n):
    if n < 24:
        return bytes([(major << 5) | n])
    if n < 256:
        return bytes([(major << 5) | 24, n])
    return bytes([(major << 5) | 25]) + n.to_bytes(2, "big")


def integer(n):
    return head(0, n) if n >= 0 else head(1, -1 - n)


def byte_string(b):
    return head(2, len(b)) + b


def text_string(s):
    return head(3, len(s.encode())) + s.encode()


def cbor_map(pairs):
    return head(5, len(pairs)) + b"".join(k + v for k, v in pairs)


def openssl(*args, data=None):
    return subprocess.run(["openssl", *args], input=data, check=True, capture_output=True).stdout


RP_ID = "chat.test"
ORIGIN = "https://chat.test"
USER_HANDLE = bytes.fromhex("00112233445566778899aabbccddeeff")
sha256 = lambda b: hashlib.sha256(b).digest()

SUITES = {
    "ES256": dict(cose=-7, genkey=["ecparam", "-name", "prime256v1", "-genkey", "-noout"], size=32, crv=1, sign=["-sha256"]),
    "ES384": dict(cose=-35, genkey=["ecparam", "-name", "secp384r1", "-genkey", "-noout"], size=48, crv=2, sign=["-sha384"]),
    "ES512": dict(cose=-36, genkey=["ecparam", "-name", "secp521r1", "-genkey", "-noout"], size=66, crv=3, sign=["-sha512"]),
    "EdDSA": dict(cose=-8, genkey=["genpkey", "-algorithm", "ed25519"], sign=None),
    "RS256": dict(cose=-257, genkey=["genrsa", "2048"], sign=["-sha256"]),
    "PS256": dict(cose=-37, genkey=["genrsa", "2048"], sign=["-sha256", "-sigopt", "rsa_padding_mode:pss", "-sigopt", "rsa_pss_saltlen:32"]),
}


def client_data(kind, challenge):
    return json.dumps({"type": kind, "challenge": b64u(challenge), "origin": ORIGIN, "crossOrigin": False}, separators=(",", ":")).encode()


out = []
with tempfile.TemporaryDirectory() as tmp:
    for name, s in SUITES.items():
        key = os.path.join(tmp, name + ".pem")
        with open(key, "wb") as f:
            f.write(openssl(*s["genkey"]))
        pub_der = openssl("pkey", "-in", key, "-pubout", "-outform", "DER")  # SubjectPublicKeyInfo

        def sign(data):
            path = os.path.join(tmp, "msg")
            with open(path, "wb") as f:
                f.write(data)
            if s["sign"] is None:
                return openssl("pkeyutl", "-sign", "-inkey", key, "-rawin", "-in", path)
            return openssl("dgst", *s["sign"], "-sign", key, path)

        if name == "EdDSA":
            cose = cbor_map([(integer(1), integer(1)), (integer(3), integer(-8)), (integer(-1), integer(6)), (integer(-2), byte_string(pub_der[-32:]))])
        elif name.startswith("ES"):
            point = pub_der[-(1 + 2 * s["size"]):]
            assert point[0] == 4
            cose = cbor_map([
                (integer(1), integer(2)), (integer(3), integer(s["cose"])), (integer(-1), integer(s["crv"])),
                (integer(-2), byte_string(point[1:1 + s["size"]])), (integer(-3), byte_string(point[1 + s["size"]:])),
            ])
        else:
            modulus = openssl("rsa", "-in", key, "-noout", "-modulus").decode().strip().split("=")[1]
            n = bytes.fromhex(modulus)
            assert n[0] != 0 and len(n) == 256
            cose = cbor_map([(integer(1), integer(3)), (integer(3), integer(s["cose"])), (integer(-1), byte_string(n)), (integer(-2), byte_string(bytes([1, 0, 1])))])

        credential_id = os.urandom(32)
        reg_challenge = os.urandom(32)
        attested = bytes([0x22]) * 16 + len(credential_id).to_bytes(2, "big") + credential_id + cose
        reg_auth = sha256(RP_ID.encode()) + bytes([0x45]) + (0).to_bytes(4, "big") + attested
        reg_client = client_data("webauthn.create", reg_challenge)
        reg_sig = sign(reg_auth + sha256(reg_client))
        attestation = cbor_map([
            (text_string("fmt"), text_string("packed")),
            (text_string("attStmt"), cbor_map([(text_string("alg"), integer(s["cose"])), (text_string("sig"), byte_string(reg_sig))])),
            (text_string("authData"), byte_string(reg_auth)),
        ])

        as_challenge = os.urandom(32)
        as_auth = sha256(RP_ID.encode()) + bytes([0x05]) + (1).to_bytes(4, "big")
        as_client = client_data("webauthn.get", as_challenge)
        as_sig = sign(as_auth + sha256(as_client))

        out.append({
            "algorithm": name,
            "cose_alg": s["cose"],
            "rp_id": RP_ID,
            "origin": ORIGIN,
            "registration_challenge": b64u(reg_challenge),
            "registration": {
                "id": b64u(credential_id), "rawId": b64u(credential_id), "type": "public-key",
                "clientExtensionResults": {},
                "response": {"attestationObject": b64u(attestation), "clientDataJSON": b64u(reg_client), "transports": ["usb"]},
            },
            "stored": {"public_key": b64u(cose), "user_handle": b64u(USER_HANDLE), "sign_count": 0, "backup_eligible": False},
            "assertion_challenge": b64u(as_challenge),
            "assertion": {
                "id": b64u(credential_id), "rawId": b64u(credential_id), "type": "public-key",
                "clientExtensionResults": {},
                "response": {"authenticatorData": b64u(as_auth), "clientDataJSON": b64u(as_client), "signature": b64u(as_sig), "userHandle": b64u(USER_HANDLE)},
            },
        })
print(json.dumps(out, indent=1))
