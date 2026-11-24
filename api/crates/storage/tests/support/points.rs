//! Questions about how a server treats what `stellarr-s3` sends, asked with
//! the library's own requests and answered with the server's own words.
//!
//! Each point writes what it sent and what came back to a transcript, and
//! records a finding: what was expected and whether the server did it. The
//! run does not stop at the first surprise, so one run answers every point.

use std::fmt::Write as _;

use stellarr_outbound::{AddressPolicy, Body, Client, Request};
use stellarr_s3 as s3;
use stellarr_s3::{
    ByteRange, Config, Credentials, ListRequest, Part, Payload, Prepared, RequestBody,
};

use super::pattern;

const MIB: usize = 1024 * 1024;
const REGION: &str = "us-east-1";

pub struct Finding {
    pub point: u8,
    pub said: String,
    pub as_expected: bool,
}

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub struct Probe {
    endpoint: String,
    credentials: Credentials,
    bucket: String,
    http: Client,
    pub transcript: String,
    pub findings: Vec<Finding>,
}

fn now() -> s3::Timestamp {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    s3::Timestamp::from_unix(seconds).unwrap()
}

impl Probe {
    pub fn new(
        endpoint: &str,
        access_key: &str,
        secret_key: &str,
        bucket: &str,
        http: Client,
    ) -> Self {
        Self {
            endpoint: endpoint.to_owned(),
            credentials: Credentials {
                access_key: access_key.to_owned(),
                secret_key: secret_key.to_owned(),
            },
            bucket: bucket.to_owned(),
            http,
            transcript: String::new(),
            findings: Vec::new(),
        }
    }

    fn https(&self) -> bool {
        self.endpoint.starts_with("https://")
    }

    fn client_for(&self, region: &str, bucket: &str) -> s3::Client {
        s3::Client::new(
            Config::new(&self.endpoint, region, bucket, self.credentials.clone())
                .expect("the endpoint is usable"),
        )
    }

    fn client(&self) -> s3::Client {
        self.client_for(REGION, &self.bucket)
    }

    fn note(&mut self, text: &str) {
        let _ = writeln!(self.transcript, "{text}");
    }

    fn find(&mut self, point: u8, said: &str, as_expected: bool) {
        let mark = if as_expected {
            "as expected"
        } else {
            "NOT AS EXPECTED"
        };
        self.note(&format!("  => point {point}: {said} [{mark}]"));
        self.findings.push(Finding {
            point,
            said: said.to_owned(),
            as_expected,
        });
    }

    /// Sends a prepared request and writes both sides of it to the
    /// transcript, the signature and the credential masked.
    pub async fn send(&mut self, label: &str, prepared: &Prepared, body: Body) -> Reply {
        let length = match &body {
            Body::Bytes(bytes) => bytes.len(),
            Body::Stream { length, .. } => *length as usize,
            Body::Empty => 0,
        };
        let _ = writeln!(
            self.transcript,
            "\n>>> [{label}] {} {}",
            prepared.method.as_str(),
            prepared.url
        );
        let _ = writeln!(self.transcript, "    host: {}", prepared.host);
        for (name, value) in &prepared.headers {
            let shown = if name == "authorization" {
                mask(value)
            } else {
                value.clone()
            };
            let _ = writeln!(self.transcript, "    {name}: {shown}");
        }
        let _ = writeln!(self.transcript, "    body: {length} bytes");

        let mut request = Request::new(prepared.method.as_str().parse().unwrap(), &prepared.url)
            .unwrap()
            .policy(AddressPolicy::Anywhere)
            .max_redirects(0)
            .timeout(std::time::Duration::from_secs(120))
            .max_response_bytes(16 * 1024 * 1024);
        for (name, value) in &prepared.headers {
            if name != "content-length" {
                request = request.header(name.parse().unwrap(), value.parse().unwrap());
            }
        }
        let collected = self
            .http
            .send_collected(request.body(body))
            .await
            .unwrap_or_else(|error| panic!("[{label}] the request failed: {error}"));
        let reply = Reply {
            status: collected.status.as_u16(),
            headers: collected
                .headers
                .iter()
                .map(|(n, v)| {
                    (
                        n.as_str().to_owned(),
                        v.to_str().unwrap_or("<not text>").to_owned(),
                    )
                })
                .collect(),
            body: collected.body.to_vec(),
        };
        let _ = writeln!(self.transcript, "<<< {}", reply.status);
        for (name, value) in &reply.headers {
            let _ = writeln!(self.transcript, "    {name}: {value}");
        }
        let text = reply.text();
        let shown: String = text.chars().take(700).collect();
        let _ = writeln!(
            self.transcript,
            "    body ({} bytes): {shown:?}",
            reply.body.len()
        );
        reply
    }

    fn body_of(prepared: &mut Prepared) -> Body {
        match std::mem::replace(&mut prepared.body, RequestBody::Empty) {
            RequestBody::Bytes(bytes) => Body::bytes(bytes),
            _ => Body::empty(),
        }
    }

    async fn put(
        &mut self,
        label: &str,
        key: &str,
        data: &[u8],
        metadata: &[(&str, &str)],
    ) -> Reply {
        let mut prepared = self
            .client()
            .put_object(
                key,
                data.len() as u64,
                None,
                metadata,
                Payload::hash_of(data),
                now(),
            )
            .unwrap();
        let _ = Self::body_of(&mut prepared);
        self.send(label, &prepared, Body::bytes(data.to_vec()))
            .await
    }

    async fn get(&mut self, label: &str, key: &str, range: Option<ByteRange>) -> Reply {
        let mut prepared = self.client().get_object(key, range, now()).unwrap();
        let body = Self::body_of(&mut prepared);
        self.send(label, &prepared, body).await
    }

    async fn head(&mut self, label: &str, key: &str) -> Reply {
        let prepared = self.client().head_object(key, now()).unwrap();
        self.send(label, &prepared, Body::empty()).await
    }

    /// Runs every point and returns. The bucket is made if it is not there.
    pub async fn run_all(&mut self) {
        let created = {
            let prepared = self.client().create_bucket(now()).unwrap();
            self.send("make the bucket", &prepared, Body::empty()).await
        };
        assert!(
            created.status == 200 || created.text().contains("BucketAlreadyOwnedByYou"),
            "the bucket cannot be made: {} {}",
            created.status,
            created.text()
        );
        self.point_1().await;
        self.point_2().await;
        self.point_3().await;
        self.point_4().await;
        self.point_5().await;
        self.point_6().await;
        self.point_7().await;
        self.point_8().await;
        self.point_9().await;
        self.point_10().await;
        self.point_11().await;
        self.point_12().await;
        self.point_13().await;
    }

    pub fn summary(&self) -> String {
        let mut out = String::from("\nFINDINGS\n");
        for finding in &self.findings {
            let _ = writeln!(
                out,
                "  point {:>2} {} {}",
                finding.point,
                if finding.as_expected { "ok " } else { "BAD" },
                finding.said
            );
        }
        out
    }

    // 1. UNSIGNED-PAYLOAD over https, chunk-signed bodies over http.
    async fn point_1(&mut self) {
        self.note("\n=== point 1: unsigned payload over https, chunk-signed bodies");
        let data = pattern(200_000, 1);
        if self.https() {
            let mut prepared = self
                .client()
                .put_object(
                    "p1/unsigned",
                    data.len() as u64,
                    None,
                    &[],
                    Payload::Unsigned,
                    now(),
                )
                .unwrap();
            let _ = Self::body_of(&mut prepared);
            let reply = self
                .send(
                    "unsigned payload over https",
                    &prepared,
                    Body::bytes(data.clone()),
                )
                .await;
            self.find(
                1,
                &format!("UNSIGNED-PAYLOAD over https answered {}", reply.status),
                reply.status == 200,
            );
            let back = self.get("read it back", "p1/unsigned", None).await;
            self.find(
                1,
                "the unsigned object reads back unchanged",
                back.body == data,
            );
        } else {
            let refused =
                self.client()
                    .put_object("p1/unsigned", 1, None, &[], Payload::Unsigned, now());
            self.find(
                1,
                "UNSIGNED-PAYLOAD over plain http is refused by the library before sending",
                matches!(refused, Err(s3::RequestError::UnsignedOverPlainHttp)),
            );
        }
        // Chunk-signed, whatever the scheme.
        let chunk = 64 * 1024;
        let mut prepared = self
            .client()
            .put_object(
                "p1/chunked",
                data.len() as u64,
                None,
                &[],
                Payload::Chunked { chunk_size: chunk },
                now(),
            )
            .unwrap();
        let RequestBody::Chunked(mut signer) =
            std::mem::replace(&mut prepared.body, RequestBody::Empty)
        else {
            panic!("a chunked payload comes with a signer");
        };
        let mut framed = Vec::new();
        for piece in data.chunks(chunk) {
            signer.chunk(piece, &mut framed).unwrap();
        }
        signer.finish(&mut framed).unwrap();
        assert_eq!(
            prepared.header("content-length").unwrap(),
            framed.len().to_string(),
            "the framing is as long as the library said"
        );
        let scheme = if self.https() { "https" } else { "http" };
        let reply = self
            .send(
                &format!("chunk-signed body over {scheme}"),
                &prepared,
                Body::bytes(framed),
            )
            .await;
        self.find(
            1,
            &format!(
                "STREAMING-AWS4-HMAC-SHA256-PAYLOAD over {scheme} answered {}",
                reply.status
            ),
            reply.status == 200,
        );
        let back = self
            .get("read the chunked object back", "p1/chunked", None)
            .await;
        self.find(
            1,
            "the chunk-signed object reads back unchanged (no framing stored)",
            back.body == data,
        );
        self.find(
            1,
            &format!(
                "a read of it carries content-encoding {:?}",
                back.header("content-encoding")
            ),
            back.header("content-encoding").is_none(),
        );
    }

    // 2. encoding-type=url on listings.
    async fn point_2(&mut self) {
        self.note("\n=== point 2: encoding-type=url");
        let keys = [
            "p2/a b", "p2/a+b", "p2/a%2Bb", "p2/é/ü", "p2/a&b=c", "p2/q?r#s", "p2/x;y,z",
        ];
        for key in keys {
            let reply = self.put("a key with odd characters", key, b"1", &[]).await;
            assert_eq!(reply.status, 200, "{key}");
        }
        let request = ListRequest {
            prefix: Some("p2/"),
            ..Default::default()
        };
        let prepared = self.client().list_objects(&request, now()).unwrap();
        let reply = self
            .send("list with encoding-type=url", &prepared, Body::empty())
            .await;
        let page = s3::read_list_objects(reply.status, &reply.headers, &reply.body);
        let raw = reply.text();
        self.find(
            2,
            "the answer says <EncodingType>url</EncodingType>",
            raw.contains("<EncodingType>url</EncodingType>"),
        );
        self.find(
            2,
            "a plus in a key is written %2B",
            raw.contains("p2/a%2Bb") || raw.contains("p2%2Fa%2Bb"),
        );
        self.find(
            2,
            "a space in a key is written + (or %20)",
            raw.contains("p2/a+b</Key>")
                || raw.contains("p2/a%20b</Key>")
                || raw.contains("p2%2Fa+b")
                || raw.contains("p2%2Fa%20b"),
        );
        let mut expected: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        expected.sort();
        let got: Vec<String> = page
            .map(|p| p.objects.into_iter().map(|o| o.key).collect())
            .unwrap_or_default();
        let mut got_sorted = got.clone();
        got_sorted.sort();
        self.find(
            2,
            "the library reads every key back as it was written",
            got_sorted == expected,
        );
        self.note(&format!("  keys read back: {got:?}"));
    }

    // 3. Open-ended ranges.
    async fn point_3(&mut self) {
        self.note("\n=== point 3: open-ended and clamped ranges");
        self.put("ten bytes", "p3/ten", b"0123456789", &[]).await;
        let open = self
            .get("bytes=7-", "p3/ten", Some(ByteRange { from: 7, to: None }))
            .await;
        self.find(
            3,
            &format!(
                "bytes=7- answered {} with content-range {:?}",
                open.status,
                open.header("content-range")
            ),
            open.status == 206
                && open.header("content-range") == Some("bytes 7-9/10")
                && open.body == b"789",
        );
        let clamped = self
            .get(
                "bytes=7-100",
                "p3/ten",
                Some(ByteRange {
                    from: 7,
                    to: Some(100),
                }),
            )
            .await;
        self.find(
            3,
            &format!(
                "bytes=7-100 answered {} with content-range {:?}",
                clamped.status,
                clamped.header("content-range")
            ),
            clamped.status == 206
                && clamped.header("content-range") == Some("bytes 7-9/10")
                && clamped.body == b"789",
        );
        let all = self
            .get("bytes=0-", "p3/ten", Some(ByteRange { from: 0, to: None }))
            .await;
        self.note(&format!(
            "  bytes=0- answered {} {:?}",
            all.status,
            all.header("content-range")
        ));
        let past = self
            .get(
                "bytes=10-",
                "p3/ten",
                Some(ByteRange { from: 10, to: None }),
            )
            .await;
        self.find(
            3,
            &format!("bytes=10- on ten bytes answered {}", past.status),
            past.status == 416,
        );
    }

    // 4. Ranges on an empty object, Content-Range of a clamped range.
    async fn point_4(&mut self) {
        self.note("\n=== point 4: a range on an empty object");
        self.put("an empty object", "p4/empty", b"", &[]).await;
        let whole = self.get("the empty object, whole", "p4/empty", None).await;
        self.find(
            4,
            &format!("the empty object read whole answers {}", whole.status),
            whole.status == 200 && whole.body.is_empty(),
        );
        let open = self
            .get(
                "bytes=0- on empty",
                "p4/empty",
                Some(ByteRange { from: 0, to: None }),
            )
            .await;
        self.find(
            4,
            &format!("bytes=0- on an empty object answered {}", open.status),
            open.status == 416,
        );
        let first = self
            .get(
                "bytes=0-0 on empty",
                "p4/empty",
                Some(ByteRange {
                    from: 0,
                    to: Some(0),
                }),
            )
            .await;
        self.find(
            4,
            &format!("bytes=0-0 on an empty object answered {}", first.status),
            first.status == 416,
        );
        let error = s3::Error::from_response(open.status, &open.headers, &open.body);
        self.note(&format!("  as the library classifies it: {:?}", error.kind));
    }

    // 5. Dot segments and double slashes in keys.
    async fn point_5(&mut self) {
        self.note("\n=== point 5: dot segments and double slashes (hostile input only)");
        for (n, key) in ["p5/a/../b", "p5/./x", "p5/a//b"].into_iter().enumerate() {
            let data = format!("object {n}");
            let put = self
                .put(&format!("put {key}"), key, data.as_bytes(), &[])
                .await;
            let get = self.get(&format!("get {key}"), key, None).await;
            self.find(
                5,
                &format!(
                    "{key}: put answered {}, get answered {}",
                    put.status, get.status
                ),
                (put.status == 200 && get.status == 200 && get.body == data.as_bytes())
                    || (put.status >= 400 && get.status >= 400),
            );
        }
        let collapsed = self.get("get the collapsed p5/b", "p5/b", None).await;
        let collapsed_x = self.get("get the collapsed p5/x", "p5/x", None).await;
        let collapsed_ab = self.get("get the collapsed p5/a/b", "p5/a/b", None).await;
        self.find(
            5,
            &format!(
                "collapsed forms answered p5/b {}, p5/x {}, p5/a/b {} (404 means the server kept the dots)",
                collapsed.status, collapsed_x.status, collapsed_ab.status
            ),
            collapsed.status == 404 && collapsed_x.status == 404 && collapsed_ab.status == 404,
        );
        let request = ListRequest {
            prefix: Some("p5/"),
            ..Default::default()
        };
        let prepared = self.client().list_objects(&request, now()).unwrap();
        let listed = self
            .send("what p5/ now holds", &prepared, Body::empty())
            .await;
        self.note(&format!("  listing: {}", listed.text()));
    }

    // 6. A bare ?uploads signed as uploads=.
    async fn point_6(&mut self) {
        self.note("\n=== point 6: ?uploads signed as uploads=");
        let prepared = self
            .client()
            .create_multipart_upload("p6/multi", None, &[], now())
            .unwrap();
        self.find(
            6,
            &format!("the URL sent ends in {:?}", prepared.url.rsplit('/').next()),
            prepared.url.ends_with("?uploads"),
        );
        let reply = self
            .send("create a multipart upload", &prepared, Body::empty())
            .await;
        let id = s3::read_create_multipart_upload(reply.status, &reply.headers, &reply.body);
        self.find(
            6,
            &format!(
                "the signature of a bare ?uploads was accepted: {}",
                reply.status
            ),
            reply.status == 200 && id.is_ok(),
        );
        if let Ok(id) = id {
            let prepared = self
                .client()
                .abort_multipart_upload("p6/multi", &id, now())
                .unwrap();
            let aborted = self.send("abort it", &prepared, Body::empty()).await;
            self.note(&format!("  abort answered {}", aborted.status));
        }
    }

    async fn start_upload(&mut self, key: &str) -> String {
        let prepared = self
            .client()
            .create_multipart_upload(key, None, &[], now())
            .unwrap();
        let reply = self
            .send(
                &format!("create an upload for {key}"),
                &prepared,
                Body::empty(),
            )
            .await;
        s3::read_create_multipart_upload(reply.status, &reply.headers, &reply.body)
            .expect("an upload id")
    }

    async fn upload_part(&mut self, key: &str, id: &str, number: u32, data: &[u8]) -> Reply {
        let mut prepared = self
            .client()
            .upload_part(
                key,
                id,
                number,
                data.len() as u64,
                Payload::hash_of(data),
                now(),
            )
            .unwrap();
        let _ = Self::body_of(&mut prepared);
        self.send(
            &format!("part {number} of {key}"),
            &prepared,
            Body::bytes(data.to_vec()),
        )
        .await
    }

    // 7. CompleteMultipartUpload that answers 200 with an error, blanks first.
    async fn point_7(&mut self) {
        self.note("\n=== point 7: completing an upload");
        let id = self.start_upload("p7/multi").await;
        let part = self
            .upload_part("p7/multi", &id, 1, &pattern(1000, 7))
            .await;
        let etag = s3::read_upload_part(part.status, &part.headers, &part.body).unwrap();

        let wrong = vec![Part {
            number: 1,
            etag: "\"00000000000000000000000000000000\"".into(),
        }];
        let prepared = self
            .client()
            .complete_multipart_upload("p7/multi", &id, &wrong, now())
            .unwrap();
        let body = match &prepared.body {
            RequestBody::Bytes(b) => Body::bytes(b.clone()),
            _ => Body::empty(),
        };
        let reply = self
            .send("complete with a wrong etag", &prepared, body)
            .await;
        let read = s3::read_complete_multipart_upload(reply.status, &reply.headers, &reply.body);
        self.find(
            7,
            &format!(
                "a wrong etag answered {} and the library read it as {:?}",
                reply.status,
                read.as_ref().err().map(|e| e.kind)
            ),
            read.is_err(),
        );

        let right = vec![Part { number: 1, etag }];
        let prepared = self
            .client()
            .complete_multipart_upload("p7/multi", &id, &right, now())
            .unwrap();
        let body = match &prepared.body {
            RequestBody::Bytes(b) => Body::bytes(b.clone()),
            _ => Body::empty(),
        };
        let reply = self.send("complete properly", &prepared, body).await;
        let first = reply
            .body
            .iter()
            .copied()
            .find(|b| !b.is_ascii_whitespace());
        let lead = reply
            .body
            .iter()
            .take_while(|b| b.is_ascii_whitespace())
            .count();
        self.note(&format!(
            "  the answer starts with {lead} blank bytes before {:?}",
            first.map(char::from)
        ));
        let read = s3::read_complete_multipart_upload(reply.status, &reply.headers, &reply.body);
        self.find(
            7,
            &format!(
                "completing answered {} and reads as {:?}",
                reply.status,
                read.as_ref().map(|_| "an etag").map_err(|e| e.kind)
            ),
            read.is_ok(),
        );
        self.note("  A 200 that carries an error cannot be provoked on demand: the library's reading of it is shown on the fake only.");
    }

    // 8. Equal parts, part numbers up to 10000.
    async fn point_8(&mut self) {
        self.note("\n=== point 8: equal parts, part numbers up to 10000");
        let id = self.start_upload("p8/multi").await;
        let full = pattern(5 * MIB, 8);
        let mut parts = Vec::new();
        for number in [1u32, 2, 10_000] {
            let data = if number == 10_000 {
                &full[..MIB]
            } else {
                &full[..]
            };
            let reply = self.upload_part("p8/multi", &id, number, data).await;
            let etag = s3::read_upload_part(reply.status, &reply.headers, &reply.body);
            self.find(
                8,
                &format!("part {number} answered {}", reply.status),
                etag.is_ok(),
            );
            if let Ok(etag) = etag {
                parts.push(Part { number, etag });
            }
        }
        let refused =
            self.client()
                .upload_part("p8/multi", &id, 10_001, 1, Payload::hash_of(b"x"), now());
        self.find(
            8,
            "part 10001 is refused by the library before sending",
            refused.is_err(),
        );
        let prepared = self
            .client()
            .complete_multipart_upload("p8/multi", &id, &parts, now())
            .unwrap();
        let body = match &prepared.body {
            RequestBody::Bytes(b) => Body::bytes(b.clone()),
            _ => Body::empty(),
        };
        let reply = self
            .send("complete parts 1, 2 and 10000", &prepared, body)
            .await;
        let read = s3::read_complete_multipart_upload(reply.status, &reply.headers, &reply.body);
        self.find(
            8,
            &format!(
                "parts 1, 2 and 10000 (5 MiB, 5 MiB, 1 MiB) completed: {}",
                reply.status
            ),
            read.is_ok(),
        );
        let head = self.head("the assembled object", "p8/multi").await;
        self.find(
            8,
            &format!("its length is {:?}", head.header("content-length")),
            head.header("content-length") == Some(&(11 * MIB).to_string()),
        );

        // The smallest a part may be, when it is not the last.
        let id = self.start_upload("p8/small").await;
        let one = self.upload_part("p8/small", &id, 1, &full[..MIB]).await;
        let two = self.upload_part("p8/small", &id, 2, &full[..MIB]).await;
        let parts = vec![
            Part {
                number: 1,
                etag: s3::read_upload_part(one.status, &one.headers, &one.body).unwrap(),
            },
            Part {
                number: 2,
                etag: s3::read_upload_part(two.status, &two.headers, &two.body).unwrap(),
            },
        ];
        let prepared = self
            .client()
            .complete_multipart_upload("p8/small", &id, &parts, now())
            .unwrap();
        let body = match &prepared.body {
            RequestBody::Bytes(b) => Body::bytes(b.clone()),
            _ => Body::empty(),
        };
        let reply = self
            .send(
                "complete with a 1 MiB part that is not last",
                &prepared,
                body,
            )
            .await;
        self.note(&format!(
            "  a 1 MiB first part: completing answered {} (the 5 MiB minimum {})",
            reply.status,
            if reply.status == 400 {
                "is enforced"
            } else {
                "is not enforced here"
            }
        ));
        let prepared = self
            .client()
            .abort_multipart_upload("p8/small", &id, now())
            .unwrap();
        let _ = self
            .send("abort what is left", &prepared, Body::empty())
            .await;
    }

    // 9. x-amz-meta-sha256 on HEAD.
    async fn point_9(&mut self) {
        self.note("\n=== point 9: x-amz-meta-sha256");
        let data = pattern(1000, 9);
        let hash = s3::internals::hex(&s3::sha256(&data));
        self.put(
            "an object with a hash in its metadata",
            "p9/hashed",
            &data,
            &[("sha256", &hash)],
        )
        .await;
        let reply = self.head("head it", "p9/hashed").await;
        let head = s3::read_head_object(reply.status, &reply.headers);
        let echoed = head
            .as_ref()
            .ok()
            .and_then(|h| h.metadata_value("sha256"))
            .map(str::to_owned);
        self.find(
            9,
            &format!("HEAD echoes the metadata: {echoed:?}"),
            echoed.as_deref() == Some(hash.as_str()),
        );
    }

    // 10. A 404 on HEAD: key or bucket?
    async fn point_10(&mut self) {
        self.note("\n=== point 10: HEAD of a missing key and of a missing bucket");
        let key = self
            .head("a missing key in this bucket", "p10/missing")
            .await;
        let prepared = self
            .client_for(REGION, "stellarr-no-such-bucket-0")
            .head_object("p10/missing", now())
            .unwrap();
        let bucket = self
            .send(
                "a key in a bucket that is not there",
                &prepared,
                Body::empty(),
            )
            .await;
        let names = |r: &Reply| {
            let mut n: Vec<String> = r
                .headers
                .iter()
                .map(|(n, _)| n.to_ascii_lowercase())
                .collect();
            n.sort();
            n
        };
        let (a, b) = (names(&key), names(&bucket));
        let only_key: Vec<_> = a.iter().filter(|n| !b.contains(n)).collect();
        let only_bucket: Vec<_> = b.iter().filter(|n| !a.contains(n)).collect();
        self.note(&format!("  header names only on the missing key: {only_key:?}; only on the missing bucket: {only_bucket:?}"));
        self.find(
            10,
            &format!("statuses are {} and {}", key.status, bucket.status),
            key.status == 404 && bucket.status == 404,
        );
        // GET does tell them apart, which is what the library relies on.
        let get_bucket = {
            let prepared = self
                .client_for(REGION, "stellarr-no-such-bucket-0")
                .get_object("p10/missing", None, now())
                .unwrap();
            self.send("GET in the missing bucket", &prepared, Body::empty())
                .await
        };
        let error =
            s3::Error::from_response(get_bucket.status, &get_bucket.headers, &get_bucket.body);
        self.find(
            10,
            &format!("a GET in the missing bucket is classified {:?}", error.kind),
            error.kind == s3::ErrorKind::BucketNotFound,
        );
    }

    // 11. The region.
    async fn point_11(&mut self) {
        self.note("\n=== point 11: the region used for signing");
        for region in [REGION, "auto", "eu-west-1"] {
            let prepared = self
                .client_for(region, &self.bucket.clone())
                .list_buckets(now())
                .unwrap();
            let reply = self
                .send(
                    &format!("list buckets, signed for {region}"),
                    &prepared,
                    Body::empty(),
                )
                .await;
            self.find(
                11,
                &format!("signing for {region} answered {}", reply.status),
                if region == REGION {
                    reply.status == 200
                } else {
                    true
                },
            );
        }
    }

    async fn put_if_absent(&mut self, label: &str, key: &str, data: &[u8]) -> Reply {
        let mut prepared = self
            .client()
            .put_object_if_absent(
                key,
                data.len() as u64,
                None,
                &[],
                Payload::hash_of(data),
                now(),
            )
            .unwrap();
        let _ = Self::body_of(&mut prepared);
        self.send(label, &prepared, Body::bytes(data.to_vec()))
            .await
    }

    // 13. Creating only if the key is free (If-None-Match: *).
    async fn point_13(&mut self) {
        self.note("\n=== point 13: If-None-Match: * on PutObject and CompleteMultipartUpload");
        let first = self
            .put_if_absent("conditional create, key free", "p13/one", b"first")
            .await;
        self.find(
            13,
            &format!(
                "a conditional create of a free key answered {}",
                first.status
            ),
            first.status == 200,
        );
        let second = self
            .put_if_absent("conditional create, key taken", "p13/one", b"second")
            .await;
        let honoured = second.status == 412;
        self.find(
            13,
            &format!(
                "a conditional create of a taken key answered {} {:?}",
                second.status,
                second.text().chars().take(120).collect::<String>()
            ),
            true,
        );
        self.note(&format!(
            "  the condition on PutObject is {}",
            if honoured { "HONOURED" } else { "NOT honoured" }
        ));
        let back = self.get("what the key holds now", "p13/one", None).await;
        self.note(&format!("  the key holds {:?}", back.text()));
        self.find(
            13,
            &format!(
                "PutObject: honoured = {honoured}; the key holds {:?}",
                back.text()
            ),
            honoured == (back.body == b"first"),
        );

        // Complete an upload over a key that is taken.
        self.put("a plain object", "p13/two", b"plain", &[]).await;
        let id = self.start_upload("p13/two").await;
        let part = self
            .upload_part("p13/two", &id, 1, &pattern(1000, 13))
            .await;
        let etag = s3::read_upload_part(part.status, &part.headers, &part.body).unwrap();
        let parts = vec![Part { number: 1, etag }];
        let prepared = self
            .client()
            .complete_multipart_upload_if_absent("p13/two", &id, &parts, now())
            .unwrap();
        let body = match &prepared.body {
            RequestBody::Bytes(b) => Body::bytes(b.clone()),
            _ => Body::empty(),
        };
        let reply = self
            .send("conditional complete over a taken key", &prepared, body)
            .await;
        let read = s3::read_complete_multipart_upload(reply.status, &reply.headers, &reply.body);
        let held = self.get("what the key holds now", "p13/two", None).await;
        let honoured = read
            .as_ref()
            .is_err_and(|e| e.kind == s3::ErrorKind::PreconditionFailed);
        self.find(13, &format!("CompleteMultipartUpload: answered {} ({:?}); the key holds {} bytes; honoured = {honoured}", reply.status, read.as_ref().map(|_| "ok").map_err(|e| e.kind), held.body.len()), true);
        let prepared = self
            .client()
            .abort_multipart_upload("p13/two", &id, now())
            .unwrap();
        let _ = self
            .send("abort what is left", &prepared, Body::empty())
            .await;

        // Many writers at once: how many win?
        let mut tasks = Vec::new();
        for n in 0..8u8 {
            let client = self.client();
            let http = self.http.clone();
            tasks.push(tokio::spawn(async move {
                let data = vec![n; 1000];
                let prepared = client
                    .put_object_if_absent(
                        "p13/race",
                        1000,
                        None,
                        &[],
                        Payload::hash_of(&data),
                        now(),
                    )
                    .unwrap();
                let mut request = Request::new("PUT".parse().unwrap(), &prepared.url)
                    .unwrap()
                    .policy(AddressPolicy::Anywhere);
                for (name, value) in &prepared.headers {
                    if name != "content-length" {
                        request = request.header(name.parse().unwrap(), value.parse().unwrap());
                    }
                }
                http.send_collected(request.body(Body::bytes(data)))
                    .await
                    .unwrap()
                    .status
                    .as_u16()
            }));
        }
        let mut statuses = Vec::new();
        for task in tasks {
            statuses.push(task.await.unwrap());
        }
        statuses.sort();
        let winners = statuses.iter().filter(|s| **s == 200).count();
        self.find(
            13,
            &format!("8 concurrent conditional creates answered {statuses:?}: {winners} winner(s)"),
            true,
        );
    }

    // 12. No checksum trailer, application/octet-stream.
    async fn point_12(&mut self) {
        self.note("\n=== point 12: no checksum trailer, application/octet-stream");
        let reply = self.put("a plain put", "p12/plain", b"plain", &[]).await;
        self.find(
            12,
            &format!("a put with no checksum header answered {}", reply.status),
            reply.status == 200,
        );
        let head = self.head("head it", "p12/plain").await;
        self.find(
            12,
            &format!("its content type is {:?}", head.header("content-type")),
            head.header("content-type") == Some("application/octet-stream"),
        );
    }
}

/// The credential and the signature of an Authorization header, hidden.
fn mask(value: &str) -> String {
    value
        .split(", ")
        .map(|part| match part.split_once('=') {
            Some(("Signature", _)) => "Signature=<hidden>".to_owned(),
            _ if part.contains("Credential=") => {
                part.split_once('/').map_or(part.to_owned(), |(_, rest)| {
                    format!("Credential=<key>/{rest}")
                })
            }
            _ => part.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
