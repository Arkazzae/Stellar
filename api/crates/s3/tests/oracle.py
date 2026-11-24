#!/usr/bin/env python3
"""An independent Signature Version 4 for the S3 crate, written with only hmac
and hashlib, and the proof the Rust code agrees with it.

Run `python3 -I oracle.py` to check three things; it exits non-zero on any
mismatch:

1. Its own signer against the published examples: the signed-header and
   chunked-upload examples of the S3 documentation (Authenticating Requests:
   Using the Authorization Header), and cases of the general Signature Version 4
   test suite (as mirrored in botocore's aws4_testsuite). Every expected value
   below was copied from that published text, not computed.
2. That `request_vectors.txt` is exactly what this script generates now.
3. Nothing else: the Rust tests read `request_vectors.txt` and must build the
   same URL, headers and body for each case, so the Rust requests are checked
   against this independent encoder and signer, key by awkward key.

`python3 -I oracle.py --write` regenerates `request_vectors.txt`.
"""
import hashlib
import hmac
import os
import sys
import urllib.parse

HERE = os.path.dirname(os.path.abspath(__file__))
VECTORS = os.path.join(HERE, "request_vectors.txt")


def sha(data):
    return hashlib.sha256(data).hexdigest()


def mac(key, text):
    return hmac.new(key, text.encode(), hashlib.sha256).digest()


EMPTY = sha(b"")


def signing_key(secret, date, region, service):
    k = mac(("AWS4" + secret).encode(), date)
    for part in (region, service, "aws4_request"):
        k = mac(k, part)
    return k


def canonical_headers(headers):
    merged = {}
    for name, value in headers:
        value = " ".join(value.split())
        merged.setdefault(name.lower(), []).append(value)
    names = sorted(merged)
    text = "".join(f"{n}:{','.join(merged[n])}\n" for n in names)
    return text, ";".join(names)


def sigv4(secret, access, amz_date, region, service, method, path, query, headers, payload_hash):
    """`path` and `query` are already in canonical form."""
    ch, sh = canonical_headers(headers)
    creq = "\n".join([method, path, query, ch, sh, payload_hash])
    date = amz_date[:8]
    scope = f"{date}/{region}/{service}/aws4_request"
    sts = "\n".join(["AWS4-HMAC-SHA256", amz_date, scope, sha(creq.encode())])
    key = signing_key(secret, date, region, service)
    sig = hmac.new(key, sts.encode(), hashlib.sha256).hexdigest()
    auth = f"AWS4-HMAC-SHA256 Credential={access}/{scope}, SignedHeaders={sh}, Signature={sig}"
    return {"sig": sig, "auth": auth, "sts": sts, "creq": creq, "key": key, "scope": scope}


def quote(text, slash):
    # urllib's quote, with the unreserved set S3 uses; it is a different
    # implementation from the one in the Rust crate.
    return urllib.parse.quote(text.encode("utf-8"), safe="/~" if slash else "~")


def canonical_query(pairs):
    enc = sorted((quote(k, False), quote(v, False)) for k, v in pairs)
    canonical = "&".join(f"{k}={v}" for k, v in enc)
    sent = "&".join(k if v == "" else f"{k}={v}" for k, v in enc)
    return canonical, sent


# --- 1. Published values -----------------------------------------------------

S3_AK = "AKIAIOSFODNN7EXAMPLE"
S3_SK = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
S3_DATE = "20130524T000000Z"
S3_HOST = "examplebucket.s3.amazonaws.com"

# (name, method, path, query, extra headers, payload hash, string-to-sign hash line, signature)
S3_EXAMPLES = [
    ("GET Object", "GET", "/test.txt", "",
     [("host", S3_HOST), ("range", "bytes=0-9"), ("x-amz-content-sha256", EMPTY), ("x-amz-date", S3_DATE)], EMPTY,
     "7344ae5b7ee6c3e7e6b0fe0640412a37625d1fbfff95c48bbb2dc43964946972",
     "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"),
    ("PUT Object", "PUT", "/test%24file.text", "",
     [("date", "Fri, 24 May 2013 00:00:00 GMT"), ("host", S3_HOST),
      ("x-amz-content-sha256", "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"),
      ("x-amz-date", S3_DATE), ("x-amz-storage-class", "REDUCED_REDUNDANCY")],
     "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072",
     "9e0e90d9c76de8fa5b200d8c849cd5b8dc7a3be3951ddb7f6a76b4158342019d",
     "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"),
    ("GET Bucket Lifecycle", "GET", "/", "lifecycle=",
     [("host", S3_HOST), ("x-amz-content-sha256", EMPTY), ("x-amz-date", S3_DATE)], EMPTY,
     "9766c798316ff2757b517bc739a67f6213b4ab36dd5da2f94eaebf79c77395ca",
     "fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543"),
    ("Get Bucket (List Objects)", "GET", "/", "max-keys=2&prefix=J",
     [("host", S3_HOST), ("x-amz-content-sha256", EMPTY), ("x-amz-date", S3_DATE)], EMPTY,
     "df57d21db20da04d7fa30298dd4488ba3a2b47ca3a489c74750e0f1e7df1b9b7",
     "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"),
]

# The chunked upload example: PUT /examplebucket/chunkObject.txt, 66560 bytes of 'a'.
STREAM_HEADERS = [
    ("content-encoding", "aws-chunked"), ("content-length", "66824"), ("host", "s3.amazonaws.com"),
    ("x-amz-content-sha256", "STREAMING-AWS4-HMAC-SHA256-PAYLOAD"), ("x-amz-date", S3_DATE),
    ("x-amz-decoded-content-length", "66560"), ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
]
STREAM_SEED = "4f232c4386841ef735655705268965c44a0e4690baa4adea153f7db9fa80a0a9"
STREAM_SEED_STS_HASH = "cee3fed04b70f867d036f722359b0b1f2f0e5dc0efadbc082b76c4c60e316455"
STREAM_CHUNKS = [
    (65536, "ad80c730a21e5b8d04586a2213dd63b9a0e99e0e2307b0ade35a65485a288648"),
    (1024, "0055627c9e194cb4542bae2aa5492e3c1575bbb81b612b7d234b86a503ef5497"),
    (0, "b6c6ea8a5354eaf15b3cb7646744f4275b71ea724fed81ceb9323e279d449df9"),
]

SUITE_AK = "AKIDEXAMPLE"
SUITE_SK = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY"
SUITE_DATE = "20150830T123600Z"
SUITE = [
    ("get-vanilla", "GET", "/", "", [('Host', 'example.amazonaws.com'), ('X-Amz-Date', '20150830T123600Z')], '', "bb579772317eb040ac9ed261061d46c1f17a8133879d6129b6e1c25292927e63", "5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"),
    ("get-vanilla-empty-query-key", "GET", "/", "Param1=value1", [('Host', 'example.amazonaws.com'), ('X-Amz-Date', '20150830T123600Z')], '', "1e24db194ed7d0eec2de28d7369675a243488e08526e8c1c73571282f7c517ab", "a67d582fa61cc504c4bae71f336f98b97f1ea3c7a6bfe1b6e45aec72011b9aeb"),
    ("get-vanilla-query-order-key-case", "GET", "/", "Param2=value2&Param1=value1", [('Host', 'example.amazonaws.com'), ('X-Amz-Date', '20150830T123600Z')], '', "816cd5b414d056048ba4f7c5386d6e0533120fb1fcfa93762cf0fc39e2cf19e0", "b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500"),
    ("get-vanilla-query-order-key", "GET", "/", "Param1=value2&Param1=Value1", [('Host', 'example.amazonaws.com'), ('X-Amz-Date', '20150830T123600Z')], '', "704b4cef673542d84cdff252633f065e8daeba5f168b77116f8b1bcaf3d38f89", "eedbc4e291e521cf13422ffca22be7d2eb8146eecf653089df300a15b2382bd1"),
    ("get-vanilla-query-order-value", "GET", "/", "Param1=value2&Param1=value1", [('Host', 'example.amazonaws.com'), ('X-Amz-Date', '20150830T123600Z')], '', "c968629d70850097a2d8781c9bf7edcb988b04cac14cca9be4acc3595f884606", "5772eed61e12b33fae39ee5e7012498b51d56abc0abb7c60486157bd471c4694"),
    ("get-header-value-trim", "GET", "/", "", [('Host', 'example.amazonaws.com'), ('My-Header1', ' value1'), ('My-Header2', ' "a   b   c"'), ('X-Amz-Date', '20150830T123600Z')], '', "a726db9b0df21c14f559d0a978e563112acb1b9e05476f0a6a1c7d68f28605c7", "acc3ed3afb60bb290fc8d2dd0098b9911fcaa05412b367055dee359757a9c736"),
    ("post-vanilla", "POST", "/", "", [('Host', 'example.amazonaws.com'), ('X-Amz-Date', '20150830T123600Z')], '', "553f88c9e4d10fc9e109e2aeb65f030801b70c2f6468faca261d401ae622fc87", "5da7c1a2acd57cee7505fc6676e4e544621c30862966e37dddb68e92efbe5d6b"),
    ("post-vanilla-query", "POST", "/", "Param1=value1", [('Host', 'example.amazonaws.com'), ('X-Amz-Date', '20150830T123600Z')], '', "9d659678c1756bb3113e2ce898845a0a79dbbc57b740555917687f1b3340fbbd", "28038455d6de14eafc1f9222cf5aa6f1a96197d7deb8263271d420d138af7f11"),
    ("post-x-www-form-urlencoded", "POST", "/", "", [('Content-Type', 'application/x-www-form-urlencoded'), ('Host', 'example.amazonaws.com'), ('X-Amz-Date', '20150830T123600Z')], 'Param1=value1', "42a5e5bb34198acb3e84da4f085bb7927f2bc277ca766e6d19c73c2154021281", "ff11897932ad3f4e8b18135d722051e5ac45fc38421b1da7b9d196a0fe09473a"),
    ("post-header-key-sort", "POST", "/", "", [('Host', 'example.amazonaws.com'), ('My-Header1', 'value1'), ('X-Amz-Date', '20150830T123600Z')], '', "9368318c2967cf6de74404b30c65a91e8f6253e0a8659d6d5319f1a812f87d65", "c5410059b04c1ee005303aed430f6e6645f61f4dc9e1461ec8f8916fdf18852c"),
    ("post-header-value-case", "POST", "/", "", [('Host', 'example.amazonaws.com'), ('My-Header1', 'VALUE1'), ('X-Amz-Date', '20150830T123600Z')], '', "d51ced243e649e3de6ef63afbbdcbca03131a21a7103a1583706a64618606a93", "cdbc9802e29d2942e5e10b5bccfdd67c5f22c7c4e8ae67b53629efa58b974b7d"),
]


def check_published():
    failures = 0

    def expect(name, got, want):
        nonlocal failures
        if got != want:
            failures += 1
            print(f"MISMATCH {name}: got {got}, published {want}")

    for name, method, path, query, headers, payload, sts_hash, signature in S3_EXAMPLES:
        r = sigv4(S3_SK, S3_AK, S3_DATE, "us-east-1", "s3", method, path, query, headers, payload)
        expect(name + " string to sign", r["sts"].split("\n")[3], sts_hash)
        expect(name + " signature", r["sig"], signature)

    for name, method, path, query, headers, body, sts_hash, signature in SUITE:
        # The suite's query arrives unsorted; the canonical form is sorted.
        pairs = [tuple(p.split("=", 1)) for p in query.split("&")] if query else []
        r = sigv4(SUITE_SK, SUITE_AK, SUITE_DATE, "us-east-1", "service", method, path,
                  canonical_query(pairs)[0], headers, sha(body.encode()))
        expect("suite " + name + " string to sign", r["sts"].split("\n")[3], sts_hash)
        expect("suite " + name + " signature", r["sig"], signature)

    seed = sigv4(S3_SK, S3_AK, S3_DATE, "us-east-1", "s3", "PUT", "/examplebucket/chunkObject.txt", "",
                 STREAM_HEADERS, "STREAMING-AWS4-HMAC-SHA256-PAYLOAD")
    expect("chunked seed string to sign", seed["sts"].split("\n")[3], STREAM_SEED_STS_HASH)
    expect("chunked seed signature", seed["sig"], STREAM_SEED)
    previous = seed["sig"]
    for size, signature in STREAM_CHUNKS:
        previous = chunk_signature(seed["key"], S3_DATE, seed["scope"], previous, b"a" * size)
        expect(f"chunk of {size}", previous, signature)
    return failures


def chunk_signature(key, amz_date, scope, previous, data):
    sts = "\n".join(["AWS4-HMAC-SHA256-PAYLOAD", amz_date, scope, previous, EMPTY, sha(data)])
    return hmac.new(key, sts.encode(), hashlib.sha256).hexdigest()


def chunk_body(key, amz_date, scope, seed_signature, data, size):
    out = b""
    previous = seed_signature
    pieces = [data[i:i + size] for i in range(0, len(data), size)] + [b""]
    for piece in pieces:
        previous = chunk_signature(key, amz_date, scope, previous, piece)
        out += f"{len(piece):x};chunk-signature={previous}\r\n".encode() + piece + b"\r\n"
    return out


# --- 2. The requests the crate builds -----------------------------------------

AK = "AKIAIOSFODNN7EXAMPLE"
SK = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
NOW = "20130524T000000Z"
REGION = "us-east-1"
BUCKET = "stellarr-test"
CONFIGS = {
    "https": ("https", "Store.Example.test:9443", "store.example.test:9443"),
    "http": ("http", "minio.test:9000", "minio.test:9000"),
    "default-port": ("https", "r2.example.test:443", "r2.example.test"),
}

AWKWARD_KEYS = [
    "plain.txt",
    "a b.txt",
    "dir/sub dir/file name.bin",
    "plus+sign.txt",
    "100%.txt",
    "%2F.txt",
    "a%20b",
    "ünïcödé/日本語/файл.txt",
    "emoji\U0001F600.png",
    "dots.in.name...",
    "trailing/",
    "a//b",
    "/leading",
    "..",
    "a/../b",
    "./x",
    "~tilde",
    "tab\there",
    "quote'\"",
    "star*paren()bang!",
    "semi;colon:at@amp&eq=dollar$comma,",
    "?question#hash",
    "back\\slash",
    "[brackets]{braces}<angle>|pipe^caret`tick",
    "test$file.text",
    "attachments/2026/11/0b8a6c1e-6a2f-4d1c-9c1a-6e4c2f0a9d11.png",
    "k" * 1024,
]


def hexs(text):
    return text.encode("utf-8").hex() if text else "-"


class Case:
    def __init__(self, name, config, call):
        self.name, self.config, self.call = name, config, call

    def build(self, method, key, query, extra, length_header=None, payload_hash=EMPTY, body=b"", chunk=None):
        scheme, _, host = CONFIGS[self.config]
        path = "/" + quote(BUCKET, False) + ("/" + quote(key, True) if key is not None else "")
        cq, sq = canonical_query(query)
        headers = list(extra)
        if length_header is not None:
            headers.append(("content-length", str(length_header)))
        headers.append(("x-amz-content-sha256", payload_hash))
        headers.append(("x-amz-date", NOW))
        r = sigv4(SK, AK, NOW, REGION, "s3", method, path, cq, [("host", host)] + headers, payload_hash)
        headers.append(("authorization", r["auth"]))
        url = f"{scheme}://{host}{path}" + (f"?{sq}" if sq else "")
        self.url, self.headers, self.body, self.sig = url, headers, body, r
        return self

    def lines(self):
        out = [f"case {self.name}", f"config {self.config}", f"call {self.call}"]
        out.append(f"url {self.url}")
        for n, v in sorted(self.headers):
            out.append(f"header {n} {v}")
        out.append(f"body_len {len(self.body)}")
        out.append(f"body_sha256 {sha(self.body)}")
        out.append("")
        return out


def cases():
    out = []
    for i, key in enumerate(AWKWARD_KEYS):
        out.append(Case(f"get-awkward-{i}", "https", f"get_object {hexs(key)} -").build("GET", key, [], []))
    for i, key in enumerate(AWKWARD_KEYS):
        cfg = ["https", "http", "default-port"][i % 3]
        out.append(Case(f"head-awkward-{i}", cfg, f"head_object {hexs(key)}").build("HEAD", key, [], []))
    for i, key in enumerate(AWKWARD_KEYS[:8]):
        out.append(Case(f"delete-awkward-{i}", "https", f"delete_object {hexs(key)}").build("DELETE", key, [], []))

    k = "ranges/object.bin"
    out.append(Case("get-range-closed", "https", f"get_object {hexs(k)} 2-5").build("GET", k, [], [("range", "bytes=2-5")]))
    out.append(Case("get-range-open", "https", f"get_object {hexs(k)} 7-").build("GET", k, [], [("range", "bytes=7-")]))
    out.append(_list_buckets("https"))
    out.append(Case("create-bucket", "https", "create_bucket").build("PUT", None, [], [], 0))
    out.append(_list("list-probe", "https", 1, None, None))
    out.append(_list("list-first", "http", None, None, None))
    out.append(_list("list-next", "https", None, "1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM=", None))
    out.append(_list("list-all-args", "https", 1000, "a+b c/d==", "dir name/+x"))

    meta = hexs("0123456789abcdef" * 4)
    key = "attachments/a b+c.png"
    out.append(Case("put-unsigned", "https",
                    f"put_object {hexs(key)} 1234 {hexs('image/png')} sha256={meta} unsigned")
               .build("PUT", key, [], [("content-type", "image/png"), ("x-amz-meta-sha256", "0123456789abcdef" * 4)],
                      1234, "UNSIGNED-PAYLOAD"))
    out.append(Case("put-unsigned-default-port", "default-port",
                    f"put_object {hexs(key)} 0 - - unsigned")
               .build("PUT", key, [], [("content-type", "application/octet-stream")], 0, "UNSIGNED-PAYLOAD"))
    data = b"Welcome to Amazon S3."
    out.append(Case("put-hashed", "http", f"put_object {hexs('test$file.text')} {len(data)} - - hashed:{hexs('Welcome to Amazon S3.')}")
               .build("PUT", "test$file.text", [], [("content-type", "application/octet-stream")], len(data), sha(data)))

    for label, length, chunk_size in [("two-chunks", 66560, 65536), ("one-chunk", 8192, 65536), ("empty", 0, 8192),
                                       ("exact", 16384, 8192), ("small-last", 20000, 8192)]:
        c = Case(f"put-chunked-{label}", "http", f"put_object {hexs('chunk object.txt')} {length} - - chunked:{chunk_size}")
        # The seed signature depends on the content-length, which is the size
        # of the framed body, so build the body once with a trial seed to
        # learn it. Framing is fixed width, so the length does not depend on
        # the signatures.
        wire_len = length + sum(len(f"{min(chunk_size, length - i):x}") + 17 + 64 + 2 + 2
                                for i in range(0, length, chunk_size)) + (1 + 17 + 64 + 2 + 2)
        payload = "STREAMING-AWS4-HMAC-SHA256-PAYLOAD"
        c.build("PUT", "chunk object.txt", [],
                [("content-type", "application/octet-stream"), ("content-encoding", "aws-chunked"),
                 ("x-amz-decoded-content-length", str(length))], wire_len, payload)
        c.body = chunk_body(c.sig["key"], NOW, c.sig["scope"], c.sig["sig"], b"a" * length, chunk_size)
        assert len(c.body) == wire_len, (label, len(c.body), wire_len)
        out.append(c)

    key = "backups/0b8a6c1e/stellarr 2026-11-24.zip"
    out.append(Case("multipart-create", "https",
                    f"create_multipart {hexs(key)} {hexs('application/zip')} sha256={meta}")
               .build("POST", key, [("uploads", "")],
                      [("content-type", "application/zip"), ("x-amz-meta-sha256", "0123456789abcdef" * 4)], 0, EMPTY))
    uid = "VXBsb2FkIElEIGZvciBlbHZpbmcncyBteS1tb3ZpZS5tMnRzIHVwbG9hZA+/=="
    out.append(Case("multipart-part", "https", f"upload_part {hexs(key)} {hexs(uid)} 3 67108864 unsigned")
               .build("PUT", key, [("partNumber", "3"), ("uploadId", uid)], [], 67108864, "UNSIGNED-PAYLOAD"))
    parts = [(1, '"a54357aff0632cce46d942af68356b38"'), (2, '"0c78aef83f66abc1fa1e8477f296d394"'), (10000, '"x&y<z>"')]
    xml = ('<CompleteMultipartUpload xmlns="http://s3.amazonaws.com/doc/2006-03-01/">' +
           "".join(f"<Part><PartNumber>{n}</PartNumber><ETag>{e.replace('&', '&amp;').replace('<', '&lt;').replace('>', '&gt;').replace(chr(34), '&quot;')}</ETag></Part>"
                   for n, e in parts) + "</CompleteMultipartUpload>").encode()
    c = Case("multipart-complete", "https",
             f"complete {hexs(key)} {hexs(uid)} " + ",".join(f"{n}:{hexs(e)}" for n, e in parts))
    c.build("POST", key, [("uploadId", uid)], [("content-type", "application/xml")], len(xml), sha(xml), xml)
    out.append(c)
    out.append(Case("multipart-abort", "https", f"abort {hexs(key)} {hexs(uid)}")
               .build("DELETE", key, [("uploadId", uid)], []))
    return out


def _list_buckets(config):
    c = Case("list-buckets", config, "list_buckets")
    scheme, _, host = CONFIGS[config]
    r = sigv4(SK, AK, NOW, REGION, "s3", "GET", "/", "", [("host", host), ("x-amz-content-sha256", EMPTY), ("x-amz-date", NOW)], EMPTY)
    c.url = f"{scheme}://{host}/"
    c.headers = [("x-amz-content-sha256", EMPTY), ("x-amz-date", NOW), ("authorization", r["auth"])]
    c.body = b""
    return c


def _list(name, config, max_keys, token, prefix):
    q = [("list-type", "2"), ("encoding-type", "url")]
    if max_keys is not None:
        q.append(("max-keys", str(max_keys)))
    if token is not None:
        q.append(("continuation-token", token))
    if prefix is not None:
        q.append(("prefix", prefix))
    call = f"list_objects {max_keys if max_keys is not None else '-'} {hexs(token) if token else '-'} {hexs(prefix) if prefix else '-'}"
    c = Case(name, config, call).build("GET", None, q, [])
    c.body = b""
    return c


def render():
    lines = ["# Generated by oracle.py from an independent implementation. Do not edit.",
             f"# credentials {AK} / {SK}, time {NOW}, region {REGION}, bucket {BUCKET}", ""]
    for c in cases():
        lines.extend(c.lines())
    return "\n".join(lines) + "\n"


def main():
    failures = check_published()
    if failures:
        print(f"{failures} published value(s) did not match")
        return 1
    print(f"published S3 examples ({len(S3_EXAMPLES)}), chunked example ({len(STREAM_CHUNKS) + 1} signatures) and "
          f"suite cases ({len(SUITE)}): all match")
    text = render()
    if "--write" in sys.argv:
        with open(VECTORS, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
        print("wrote", VECTORS)
        return 0
    with open(VECTORS, encoding="utf-8") as f:
        current = f.read()
    if current != text:
        print("request_vectors.txt is out of date; run with --write")
        return 1
    print("request_vectors.txt is current")
    return 0


if __name__ == "__main__":
    sys.exit(main())
