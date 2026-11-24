//! Signature Version 4 against the values Amazon published.
//!
//! Every expected value below is copied from the S3 documentation ("Signature
//! Calculations for the Authorization Header", single chunk and multiple
//! chunks) or from the general test suite (as mirrored by botocore's
//! `aws4_testsuite`), and `oracle.py` checks the same values against an
//! independent implementation.

use stellarr_s3::internals::{Request, Scope, hex, query_strings, sign};
use stellarr_s3::{Credentials, sha256};

const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn s3_credentials() -> Credentials {
    Credentials {
        access_key: "AKIAIOSFODNN7EXAMPLE".into(),
        secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
    }
}

fn s3_scope() -> Scope<'static> {
    Scope {
        amz_date: "20130524T000000Z",
        region: "us-east-1",
        service: "s3",
    }
}

const HOST: &str = "examplebucket.s3.amazonaws.com";
const DATE: &str = "20130524T000000Z";

#[test]
fn get_object_with_a_range() {
    let signed = sign(
        &s3_credentials(),
        &s3_scope(),
        &Request {
            method: "GET",
            path: "/test.txt",
            query: "",
            headers: &[
                ("host", HOST),
                ("range", "bytes=0-9"),
                ("x-amz-content-sha256", EMPTY),
                ("x-amz-date", DATE),
            ],
            payload_hash: EMPTY,
        },
    );
    assert_eq!(
        signed.canonical_request,
        "GET\n/test.txt\n\nhost:examplebucket.s3.amazonaws.com\nrange:bytes=0-9\nx-amz-content-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\nx-amz-date:20130524T000000Z\n\nhost;range;x-amz-content-sha256;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        signed.string_to_sign,
        "AWS4-HMAC-SHA256\n20130524T000000Z\n20130524/us-east-1/s3/aws4_request\n7344ae5b7ee6c3e7e6b0fe0640412a37625d1fbfff95c48bbb2dc43964946972"
    );
    assert_eq!(
        signed.signature,
        "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
    );
    assert_eq!(
        signed.authorization.replace(", ", ","),
        "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
    );
}

#[test]
fn put_object_with_a_dollar_in_the_key() {
    let body_hash = hex(&sha256(b"Welcome to Amazon S3."));
    assert_eq!(
        body_hash,
        "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"
    );
    let signed = sign(
        &s3_credentials(),
        &s3_scope(),
        &Request {
            method: "PUT",
            path: "/test%24file.text",
            query: "",
            headers: &[
                ("Date", "Fri, 24 May 2013 00:00:00 GMT"),
                ("Host", HOST),
                ("x-amz-content-sha256", &body_hash),
                ("x-amz-date", DATE),
                ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
            ],
            payload_hash: &body_hash,
        },
    );
    assert_eq!(
        signed.string_to_sign.lines().last(),
        Some("9e0e90d9c76de8fa5b200d8c849cd5b8dc7a3be3951ddb7f6a76b4158342019d")
    );
    assert_eq!(
        signed.signature,
        "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
    );
    assert_eq!(
        signed.signed_headers,
        "date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class"
    );
}

#[test]
fn a_subresource_query_is_signed_with_an_empty_value() {
    let (canonical, _) = query_strings(&[("lifecycle", "")]);
    assert_eq!(canonical, "lifecycle=");
    let signed = sign(
        &s3_credentials(),
        &s3_scope(),
        &Request {
            method: "GET",
            path: "/",
            query: &canonical,
            headers: &[
                ("host", HOST),
                ("x-amz-content-sha256", EMPTY),
                ("x-amz-date", DATE),
            ],
            payload_hash: EMPTY,
        },
    );
    assert_eq!(
        signed.string_to_sign.lines().last(),
        Some("9766c798316ff2757b517bc739a67f6213b4ab36dd5da2f94eaebf79c77395ca")
    );
    assert_eq!(
        signed.signature,
        "fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543"
    );
}

#[test]
fn a_listing_with_two_parameters() {
    let (canonical, _) = query_strings(&[("prefix", "J"), ("max-keys", "2")]);
    assert_eq!(canonical, "max-keys=2&prefix=J");
    let signed = sign(
        &s3_credentials(),
        &s3_scope(),
        &Request {
            method: "GET",
            path: "/",
            query: &canonical,
            headers: &[
                ("host", HOST),
                ("x-amz-content-sha256", EMPTY),
                ("x-amz-date", DATE),
            ],
            payload_hash: EMPTY,
        },
    );
    assert_eq!(
        signed.string_to_sign.lines().last(),
        Some("df57d21db20da04d7fa30298dd4488ba3a2b47ca3a489c74750e0f1e7df1b9b7")
    );
    assert_eq!(
        signed.signature,
        "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
    );
}

// The general suite: service "service", its own keys and date. Only cases that
// do not depend on the generic suite's path rules (S3 never normalises a path,
// and encodes it once) are used; the cases left out are get-relative,
// get-slash, get-slashes, get-space, get-utf8, get-unreserved and the like.
// (name, method, path, query as sent, headers as sent, body, string-to-sign hash, signature)
type SuiteCase = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static str,
    &'static str,
    &'static str,
);

const SUITE: &[SuiteCase] = &[
    (
        "get-vanilla",
        "GET",
        "/",
        "",
        &[
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ],
        "",
        "bb579772317eb040ac9ed261061d46c1f17a8133879d6129b6e1c25292927e63",
        "5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31",
    ),
    (
        "get-vanilla-empty-query-key",
        "GET",
        "/",
        "Param1=value1",
        &[
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ],
        "",
        "1e24db194ed7d0eec2de28d7369675a243488e08526e8c1c73571282f7c517ab",
        "a67d582fa61cc504c4bae71f336f98b97f1ea3c7a6bfe1b6e45aec72011b9aeb",
    ),
    (
        "get-vanilla-query-order-key-case",
        "GET",
        "/",
        "Param2=value2&Param1=value1",
        &[
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ],
        "",
        "816cd5b414d056048ba4f7c5386d6e0533120fb1fcfa93762cf0fc39e2cf19e0",
        "b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500",
    ),
    (
        "get-vanilla-query-order-key",
        "GET",
        "/",
        "Param1=value2&Param1=Value1",
        &[
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ],
        "",
        "704b4cef673542d84cdff252633f065e8daeba5f168b77116f8b1bcaf3d38f89",
        "eedbc4e291e521cf13422ffca22be7d2eb8146eecf653089df300a15b2382bd1",
    ),
    (
        "get-vanilla-query-order-value",
        "GET",
        "/",
        "Param1=value2&Param1=value1",
        &[
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ],
        "",
        "c968629d70850097a2d8781c9bf7edcb988b04cac14cca9be4acc3595f884606",
        "5772eed61e12b33fae39ee5e7012498b51d56abc0abb7c60486157bd471c4694",
    ),
    (
        "get-header-value-trim",
        "GET",
        "/",
        "",
        &[
            ("Host", "example.amazonaws.com"),
            ("My-Header1", " value1"),
            ("My-Header2", " \"a   b   c\""),
            ("X-Amz-Date", "20150830T123600Z"),
        ],
        "",
        "a726db9b0df21c14f559d0a978e563112acb1b9e05476f0a6a1c7d68f28605c7",
        "acc3ed3afb60bb290fc8d2dd0098b9911fcaa05412b367055dee359757a9c736",
    ),
    (
        "post-vanilla",
        "POST",
        "/",
        "",
        &[
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ],
        "",
        "553f88c9e4d10fc9e109e2aeb65f030801b70c2f6468faca261d401ae622fc87",
        "5da7c1a2acd57cee7505fc6676e4e544621c30862966e37dddb68e92efbe5d6b",
    ),
    (
        "post-vanilla-query",
        "POST",
        "/",
        "Param1=value1",
        &[
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ],
        "",
        "9d659678c1756bb3113e2ce898845a0a79dbbc57b740555917687f1b3340fbbd",
        "28038455d6de14eafc1f9222cf5aa6f1a96197d7deb8263271d420d138af7f11",
    ),
    (
        "post-x-www-form-urlencoded",
        "POST",
        "/",
        "",
        &[
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ],
        "Param1=value1",
        "42a5e5bb34198acb3e84da4f085bb7927f2bc277ca766e6d19c73c2154021281",
        "ff11897932ad3f4e8b18135d722051e5ac45fc38421b1da7b9d196a0fe09473a",
    ),
    (
        "post-header-key-sort",
        "POST",
        "/",
        "",
        &[
            ("Host", "example.amazonaws.com"),
            ("My-Header1", "value1"),
            ("X-Amz-Date", "20150830T123600Z"),
        ],
        "",
        "9368318c2967cf6de74404b30c65a91e8f6253e0a8659d6d5319f1a812f87d65",
        "c5410059b04c1ee005303aed430f6e6645f61f4dc9e1461ec8f8916fdf18852c",
    ),
    (
        "post-header-value-case",
        "POST",
        "/",
        "",
        &[
            ("Host", "example.amazonaws.com"),
            ("My-Header1", "VALUE1"),
            ("X-Amz-Date", "20150830T123600Z"),
        ],
        "",
        "d51ced243e649e3de6ef63afbbdcbca03131a21a7103a1583706a64618606a93",
        "cdbc9802e29d2942e5e10b5bccfdd67c5f22c7c4e8ae67b53629efa58b974b7d",
    ),
];

#[test]
fn the_general_suite_cases_that_do_not_hinge_on_path_rules() {
    let credentials = Credentials {
        access_key: "AKIDEXAMPLE".into(),
        secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
    };
    let scope = Scope {
        amz_date: "20150830T123600Z",
        region: "us-east-1",
        service: "service",
    };
    for (name, method, path, query, headers, body, sts_hash, signature) in SUITE {
        let pairs: Vec<(&str, &str)> = if query.is_empty() {
            Vec::new()
        } else {
            query
                .split('&')
                .map(|p| p.split_once('=').unwrap())
                .collect()
        };
        let (canonical, _) = query_strings(&pairs);
        let payload_hash = hex(&sha256(body.as_bytes()));
        let signed = sign(
            &credentials,
            &scope,
            &Request {
                method,
                path,
                query: &canonical,
                headers,
                payload_hash: &payload_hash,
            },
        );
        assert_eq!(
            signed.string_to_sign.lines().last(),
            Some(*sts_hash),
            "{name}: string to sign"
        );
        assert_eq!(signed.signature, *signature, "{name}");
    }
}

#[test]
fn the_chunked_upload_example_of_the_documentation() {
    use stellarr_s3::{ChunkError, ChunkSigner, chunked_length};

    // 66560 bytes of 'a' in chunks of 65536: the example's content length.
    assert_eq!(chunked_length(66560, 65536), 66824);

    let scope = s3_scope();
    let seed = sign(
        &s3_credentials(),
        &scope,
        &Request {
            method: "PUT",
            path: "/examplebucket/chunkObject.txt",
            query: "",
            headers: &[
                ("content-encoding", "aws-chunked"),
                ("content-length", "66824"),
                ("host", "s3.amazonaws.com"),
                ("x-amz-content-sha256", "STREAMING-AWS4-HMAC-SHA256-PAYLOAD"),
                ("x-amz-date", DATE),
                ("x-amz-decoded-content-length", "66560"),
                ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
            ],
            payload_hash: "STREAMING-AWS4-HMAC-SHA256-PAYLOAD",
        },
    );
    assert_eq!(
        seed.string_to_sign.lines().last(),
        Some("cee3fed04b70f867d036f722359b0b1f2f0e5dc0efadbc082b76c4c60e316455")
    );
    assert_eq!(
        seed.signature,
        "4f232c4386841ef735655705268965c44a0e4690baa4adea153f7db9fa80a0a9"
    );

    let mut signer = ChunkSigner::new(&seed, &scope, 66560, 65536);
    let mut wire = Vec::new();
    signer.chunk(&[b'a'; 65536], &mut wire).unwrap();
    signer.chunk(&[b'a'; 1024], &mut wire).unwrap();
    signer.finish(&mut wire).unwrap();
    assert_eq!(wire.len(), 66824);

    let text = String::from_utf8_lossy(&wire);
    let frames: Vec<&str> = text
        .split("\r\n")
        .filter(|l| l.contains(";chunk-signature="))
        .collect();
    assert_eq!(
        frames,
        [
            "10000;chunk-signature=ad80c730a21e5b8d04586a2213dd63b9a0e99e0e2307b0ade35a65485a288648",
            "400;chunk-signature=0055627c9e194cb4542bae2aa5492e3c1575bbb81b612b7d234b86a503ef5497",
            "0;chunk-signature=b6c6ea8a5354eaf15b3cb7646744f4275b71ea724fed81ceb9323e279d449df9",
        ]
    );
    assert!(wire.ends_with(b"\r\n\r\n"));

    // The signer is strict about how it is fed.
    assert_eq!(signer.chunk(b"a", &mut wire), Err(ChunkError::Finished));
    assert_eq!(signer.finish(&mut wire), Err(ChunkError::Finished));
}

#[test]
fn a_chunk_signer_refuses_to_be_misfed() {
    use stellarr_s3::{ChunkError, ChunkSigner};

    let scope = s3_scope();
    let seed = sign(
        &s3_credentials(),
        &scope,
        &Request {
            method: "PUT",
            path: "/b/k",
            query: "",
            headers: &[("host", "h")],
            payload_hash: "x",
        },
    );
    let mut out = Vec::new();
    let make = || ChunkSigner::new(&seed, &scope, 20000, 8192);

    let mut signer = make();
    assert_eq!(
        signer.chunk(&[0; 100], &mut out),
        Err(ChunkError::WrongSize),
        "short chunk that is not the last"
    );
    assert_eq!(
        signer.chunk(&[], &mut out),
        Err(ChunkError::WrongSize),
        "empty chunk"
    );
    assert_eq!(
        signer.chunk(&[0; 20001], &mut out),
        Err(ChunkError::TooMuch)
    );
    assert_eq!(signer.finish(&mut out), Err(ChunkError::TooLittle));
    signer.chunk(&[0; 8192], &mut out).unwrap();
    signer.chunk(&[0; 8192], &mut out).unwrap();
    assert_eq!(signer.chunk(&[0; 8192], &mut out), Err(ChunkError::TooMuch));
    assert_eq!(signer.remaining(), 3616);
    signer.chunk(&[0; 3616], &mut out).unwrap();
    signer.finish(&mut out).unwrap();

    let mut bigger = ChunkSigner::new(&seed, &scope, 0, 8192);
    bigger.finish(&mut out).unwrap();
}
