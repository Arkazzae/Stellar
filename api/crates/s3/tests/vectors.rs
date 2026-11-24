//! The requests the crate builds, against `request_vectors.txt`, which
//! `independent.py` writes with its own encoder and signer. Each case names a call;
//! the URL, every header and the body must come out as the independent signer has them.

use std::collections::BTreeMap;

use stellarr_s3::{
    ByteRange, Client, Config, Credentials, ListRequest, Part, Payload, Prepared, RequestBody,
    Timestamp, sha256,
};

const VECTORS: &str = include_str!("request_vectors.txt");
const NOW: u64 = 1_369_353_600;

fn unhex(text: &str) -> Option<String> {
    if text == "-" {
        return None;
    }
    let bytes: Vec<u8> = (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect();
    Some(String::from_utf8(bytes).unwrap())
}

fn client(config: &str) -> Client {
    let endpoint = match config {
        "https" => "https://Store.Example.test:9443",
        "http" => "http://minio.test:9000/",
        "default-port" => "https://r2.example.test:443",
        other => panic!("{other}"),
    };
    Client::new(
        Config::new(
            endpoint,
            "us-east-1",
            "stellarr-test",
            Credentials {
                access_key: "AKIAIOSFODNN7EXAMPLE".into(),
                secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            },
        )
        .unwrap(),
    )
}

fn payload(spec: &str) -> Payload {
    match spec.split_once(':') {
        None if spec == "unsigned" => Payload::Unsigned,
        Some(("hashed", data)) => Payload::hash_of(unhex(data).unwrap().as_bytes()),
        Some(("chunked", size)) => Payload::Chunked {
            chunk_size: size.parse().unwrap(),
        },
        _ => panic!("{spec}"),
    }
}

fn metadata(spec: &str) -> Vec<(String, String)> {
    if spec == "-" {
        return Vec::new();
    }
    spec.split(',')
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap();
            (name.to_owned(), unhex(value).unwrap())
        })
        .collect()
}

fn run(case: &BTreeMap<&str, Vec<&str>>) -> Prepared {
    let client = client(case["config"][0]);
    let tokens: Vec<&str> = case["call"][0].split(' ').collect();
    let now = Timestamp::from_unix(NOW).unwrap();
    let key = || unhex(tokens[1]).unwrap();
    match tokens[0] {
        "get_object" => {
            let range = match tokens[2] {
                "-" => None,
                span => {
                    let (from, to) = span.split_once('-').unwrap();
                    Some(ByteRange {
                        from: from.parse().unwrap(),
                        to: (!to.is_empty()).then(|| to.parse().unwrap()),
                    })
                }
            };
            client.get_object(&key(), range, now)
        }
        "head_object" => client.head_object(&key(), now),
        "delete_object" => client.delete_object(&key(), now),
        "list_buckets" => client.list_buckets(now),
        "create_bucket" => client.create_bucket(now),
        "list_objects" => {
            let token = unhex(tokens[2]);
            let prefix = unhex(tokens[3]);
            client.list_objects(
                &ListRequest {
                    max_keys: (tokens[1] != "-").then(|| tokens[1].parse().unwrap()),
                    continuation_token: token.as_deref(),
                    prefix: prefix.as_deref(),
                },
                now,
            )
        }
        "put_object" | "put_object_if_absent" => {
            let content_type = unhex(tokens[3]);
            let meta = metadata(tokens[4]);
            let meta: Vec<(&str, &str)> =
                meta.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
            let put = if tokens[0] == "put_object" {
                Client::put_object
            } else {
                Client::put_object_if_absent
            };
            put(
                &client,
                &key(),
                tokens[2].parse().unwrap(),
                content_type.as_deref(),
                &meta,
                payload(tokens[5]),
                now,
            )
        }
        "create_multipart" => {
            let content_type = unhex(tokens[2]);
            let meta = metadata(tokens[3]);
            let meta: Vec<(&str, &str)> =
                meta.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
            client.create_multipart_upload(&key(), content_type.as_deref(), &meta, now)
        }
        "upload_part" => client.upload_part(
            &key(),
            &unhex(tokens[2]).unwrap(),
            tokens[3].parse().unwrap(),
            tokens[4].parse().unwrap(),
            payload(tokens[5]),
            now,
        ),
        "complete" | "complete_if_absent" => {
            let parts: Vec<Part> = tokens[3]
                .split(',')
                .map(|p| {
                    let (number, etag) = p.split_once(':').unwrap();
                    Part {
                        number: number.parse().unwrap(),
                        etag: unhex(etag).unwrap(),
                    }
                })
                .collect();
            let complete = if tokens[0] == "complete" {
                Client::complete_multipart_upload
            } else {
                Client::complete_multipart_upload_if_absent
            };
            complete(&client, &key(), &unhex(tokens[2]).unwrap(), &parts, now)
        }
        "abort" => client.abort_multipart_upload(&key(), &unhex(tokens[2]).unwrap(), now),
        other => panic!("{other}"),
    }
    .unwrap_or_else(|e| panic!("{}: {e}", case["call"][0]))
}

/// The bytes a client would put on the wire for the body.
fn wire_body(prepared: Prepared, case: &BTreeMap<&str, Vec<&str>>) -> Vec<u8> {
    match prepared.body {
        RequestBody::Empty => Vec::new(),
        RequestBody::Bytes(bytes) => bytes,
        // The independent signer does not hold the object; its length and hash are only
        // recorded for bodies that are not the object's own bytes.
        RequestBody::Object { length } => {
            assert_eq!(
                prepared
                    .headers
                    .iter()
                    .find(|(n, _)| n == "content-length")
                    .unwrap()
                    .1,
                length.to_string()
            );
            Vec::new()
        }
        RequestBody::Chunked(mut signer) => {
            let call = case["call"][0];
            let length: u64 = call.split(' ').nth(2).unwrap().parse().unwrap();
            let size = signer.chunk_size();
            let mut wire = Vec::new();
            let mut left = length as usize;
            while left > 0 {
                let take = left.min(size);
                signer.chunk(&vec![b'a'; take], &mut wire).unwrap();
                left -= take;
            }
            signer.finish(&mut wire).unwrap();
            wire
        }
    }
}

#[test]
fn every_request_matches_the_independent_implementation() {
    let mut checked = 0;
    for block in VECTORS.split("\n\n") {
        let block = block.trim_matches('\n');
        if !block.contains("case ") {
            continue;
        }
        let mut case: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for line in block.lines().filter(|l| !l.starts_with('#')) {
            let (field, rest) = line.split_once(' ').unwrap();
            case.entry(field).or_default().push(rest);
        }
        let name = case["case"][0];
        let prepared = run(&case);
        assert_eq!(prepared.url, case["url"][0], "{name}: url");

        let mut got: Vec<String> = prepared
            .headers
            .iter()
            .map(|(n, v)| format!("{n} {v}"))
            .collect();
        got.sort();
        let want: Vec<String> = case["header"].iter().map(|h| (*h).to_owned()).collect();
        assert_eq!(got, want, "{name}: headers");

        let is_object = matches!(prepared.body, RequestBody::Object { .. });
        let wire = wire_body(prepared, &case);
        if !is_object {
            assert_eq!(
                wire.len().to_string(),
                case["body_len"][0],
                "{name}: body length"
            );
            let hash: String = stellarr_s3::internals::hex(&sha256(&wire));
            assert_eq!(hash, case["body_sha256"][0], "{name}: body");
        }
        checked += 1;
    }
    let listed = VECTORS.lines().filter(|l| l.starts_with("case ")).count();
    assert_eq!(checked, listed);
    assert!(checked >= 80, "only {checked} cases were read");
}

#[test]
fn awkward_keys_are_addressed_and_signed_with_one_string() {
    // The URL path and the signed path are the same encoded string by
    // construction; this checks the URL part for keys where encoders disagree.
    let client = client("https");
    let now = Timestamp::from_unix(NOW).unwrap();
    for (key, path) in [
        ("a b", "/stellarr-test/a%20b"),
        ("plus+sign", "/stellarr-test/plus%2Bsign"),
        ("100%", "/stellarr-test/100%25"),
        ("é", "/stellarr-test/%C3%A9"),
        ("a//b/", "/stellarr-test/a//b/"),
        ("a/../b", "/stellarr-test/a/../b"),
        ("x?y#z", "/stellarr-test/x%3Fy%23z"),
    ] {
        let prepared = client.get_object(key, None, now).unwrap();
        assert_eq!(
            prepared.url,
            format!("https://store.example.test:9443{path}"),
            "{key}"
        );
    }
}

/// The independent signer checks itself against the published values and that the
/// vector file is current. It needs only Python's standard library; a machine
/// without Python skips it.
#[test]
fn the_independent_signer_agrees_with_the_published_values_and_the_vectors_are_current() {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/independent.py");
    match std::process::Command::new("python3")
        .args(["-I", script])
        .output()
    {
        Ok(out) => assert!(
            out.status.success(),
            "independent.py failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(error) => eprintln!("skipped: python3 is not available ({error})"),
    }
}
