//! Compressed answers: what is decoded, how, and what is refused. The
//! compressed bodies in `fixtures/encodings` are made by Python's gzip and
//! zlib (`fixtures/encodings.py`), not by anything in this repository.

mod support;

use std::time::Duration;

use http::header::{CONTENT_ENCODING, CONTENT_LENGTH};
use http::{HeaderValue, Method};
use stellarr_outbound::{AddressPolicy, Client, Error, Request};
use support::{Conn, Server, fixture, handler};
use tokio::io::AsyncWriteExt;

fn local(url: &str) -> Request {
    Request::get(url)
        .unwrap()
        .policy(AddressPolicy::Anywhere)
        .timeout(Duration::from_secs(20))
}

fn asset(name: &str) -> Vec<u8> {
    fixture(&format!("encodings/{name}"))
}

#[derive(Clone, Copy, Debug)]
enum Framing {
    /// A `Content-Length`, all at once.
    Length,
    /// Chunked, in pieces of this many bytes.
    Chunked(usize),
    /// A `Content-Length`, one byte per write.
    Drip,
}

/// Answers with `body` as it is, declaring `coding` (when there is one).
async fn answer(conn: &mut Conn, coding: Option<&str>, body: &[u8], framing: Framing) {
    let mut head = String::from("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n");
    if let Some(coding) = coding {
        head.push_str(&format!("Content-Encoding: {coding}\r\n"));
    }
    match framing {
        Framing::Length | Framing::Drip => {
            head.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        Framing::Chunked(_) => head.push_str("Transfer-Encoding: chunked\r\n"),
    }
    head.push_str("Connection: close\r\n\r\n");
    conn.write(head.as_bytes()).await;
    match framing {
        Framing::Length => conn.write(body).await,
        Framing::Drip => {
            for byte in body {
                if conn.stream.write_all(&[*byte]).await.is_err() {
                    return;
                }
                let _ = conn.stream.flush().await;
                tokio::task::yield_now().await;
            }
        }
        Framing::Chunked(size) => {
            for piece in body.chunks(size) {
                conn.write(format!("{:x}\r\n", piece.len()).as_bytes())
                    .await;
                conn.write(piece).await;
                conn.write(b"\r\n").await;
            }
            conn.write(b"0\r\n\r\n").await;
        }
    }
}

async fn serve(coding: Option<&'static str>, body: Vec<u8>, framing: Framing) -> Server {
    Server::plain(handler(move |mut conn| {
        let body = body.clone();
        async move { answer(&mut conn, coding, &body, framing).await }
    }))
    .await
}

/// The body read piece by piece, and what stopped it, if anything.
async fn streamed(request: Request) -> (Vec<u8>, Option<Error>, Vec<usize>) {
    let response = Client::new().send(request).await.unwrap();
    let mut body = response.body;
    let mut all = Vec::new();
    let mut sizes = Vec::new();
    loop {
        match body.chunk().await {
            Ok(Some(piece)) => {
                sizes.push(piece.len());
                all.extend_from_slice(&piece);
            }
            Ok(None) => return (all, None, sizes),
            Err(error) => return (all, Some(error), sizes),
        }
    }
}

async fn collected(request: Request) -> Result<Vec<u8>, Error> {
    let response = Client::new().send(request).await?;
    Ok(response.body.collect().await?.to_vec())
}

fn framings() -> [Framing; 5] {
    [
        Framing::Length,
        Framing::Chunked(1000),
        Framing::Chunked(7),
        Framing::Chunked(1),
        Framing::Drip,
    ]
}

#[tokio::test]
async fn gzip_zlib_and_raw_deflate_are_decoded_however_they_arrive() {
    let page = asset("page.txt");
    for (file, coding) in [
        ("page.gz", "gzip"),
        ("page.zlib", "deflate"),
        ("page.deflate", "deflate"),
        ("page.gz-multi", "gzip"),
        ("page.gz.gz", "gzip, gzip"),
        ("page.zlib.gz", "deflate, gzip"),
    ] {
        let expected = page.clone();
        for framing in framings() {
            let server = serve(Some(coding), asset(file), framing).await;
            let what = format!("{file} {framing:?}");

            let response = Client::new()
                .send(local(&server.url("/")))
                .await
                .unwrap_or_else(|e| panic!("{what}: {e}"));
            // The headers say what the caller reads now.
            assert!(response.headers.get(CONTENT_ENCODING).is_none(), "{what}");
            assert!(response.headers.get(CONTENT_LENGTH).is_none(), "{what}");
            assert_eq!(response.headers["content-type"], "text/html", "{what}");
            let got = response
                .body
                .collect()
                .await
                .unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(got.as_ref(), expected.as_slice(), "{what}: collected");

            let (got, error, _) = streamed(local(&server.url("/"))).await;
            assert!(error.is_none(), "{what}: {error:?}");
            assert_eq!(got, expected, "{what}: streamed");
        }
    }
}

#[tokio::test]
async fn what_is_offered_follows_the_request() {
    let server = serve(None, b"plain".to_vec(), Framing::Length).await;
    collected(local(&server.url("/"))).await.unwrap();
    collected(local(&server.url("/")).decompress(false))
        .await
        .unwrap();
    let seen = server.recorded();
    assert_eq!(seen[0].header("accept-encoding"), Some("gzip, deflate"));
    assert_eq!(seen[1].header("accept-encoding"), Some("identity"));
}

#[tokio::test]
async fn an_answer_that_is_not_compressed_is_left_alone() {
    for coding in [
        None,
        Some("identity"),
        Some("Identity"),
        Some("IDENTITY"),
        Some("  identity  "),
        Some(""),
        Some("identity, identity"),
    ] {
        let server = serve(coding, b"just text".to_vec(), Framing::Length).await;
        let response = Client::new().send(local(&server.url("/"))).await.unwrap();
        // Nothing was decoded, so nothing is taken from the headers.
        assert_eq!(response.headers.get(CONTENT_LENGTH).unwrap(), "9");
        assert_eq!(
            response.body.collect().await.unwrap().as_ref(),
            b"just text",
            "{coding:?}"
        );
    }
    // identity among real codings is nothing.
    let server = serve(Some("identity, gzip"), asset("page.gz"), Framing::Length).await;
    assert_eq!(
        collected(local(&server.url("/"))).await.unwrap(),
        asset("page.txt")
    );
}

#[tokio::test]
async fn a_coding_that_cannot_be_undone_fails_the_body_and_not_the_answer() {
    for coding in ["br", "zstd", "compress", "gzip, br", "br, gzip", "gzip;q=1"] {
        let server = serve(Some(coding), asset("page.gz"), Framing::Length).await;
        let response = Client::new().send(local(&server.url("/"))).await.unwrap();
        assert_eq!(response.status.as_u16(), 200);
        assert_eq!(response.headers[CONTENT_ENCODING], coding);
        let error = response.body.collect().await.unwrap_err();
        assert!(matches!(error, Error::Body(_)), "{coding}: {error}");
    }
    // Five layers are more than are undone.
    let server = serve(
        Some("gzip, gzip, gzip, gzip, gzip"),
        asset("page.gz"),
        Framing::Length,
    )
    .await;
    let error = collected(local(&server.url("/"))).await.unwrap_err();
    assert!(matches!(error, Error::Body(_)), "{error}");
}

#[tokio::test]
async fn a_body_of_no_bytes_with_a_coding_declared_is_an_empty_body() {
    for framing in [Framing::Length, Framing::Chunked(10)] {
        for coding in ["gzip", "deflate", "gzip, gzip", "br"] {
            let server = serve(Some(coding), Vec::new(), framing).await;
            // Nothing to decode, so nothing to object to, even for `br`.
            let got = collected(local(&server.url("/"))).await;
            assert_eq!(got.unwrap(), b"", "{coding} {framing:?}");
            let (all, error, _) = streamed(local(&server.url("/"))).await;
            assert!(all.is_empty() && error.is_none(), "{coding} {framing:?}");
        }
    }
}

#[tokio::test]
async fn a_compressed_empty_body_is_empty() {
    for (file, coding) in [("empty.gz", "gzip"), ("empty.zlib", "deflate")] {
        let server = serve(Some(coding), asset(file), Framing::Length).await;
        assert_eq!(collected(local(&server.url("/"))).await.unwrap(), b"");
    }
}

#[tokio::test]
async fn an_answer_with_no_body_is_not_decoded_whatever_it_declares() {
    let server = Server::plain(handler(|mut conn| async move {
        let status = match conn.request.target.as_str() {
            "/304" => "304 Not Modified",
            "/204" => "204 No Content",
            _ => "200 OK",
        };
        // A HEAD answer states the length of the body a GET would carry.
        let length = if conn.request.method == "HEAD" {
            "Content-Length: 4321\r\n"
        } else {
            ""
        };
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Encoding: br\r\n{length}Connection: close\r\n\r\n"
        );
        conn.write(head.as_bytes()).await;
    }))
    .await;
    for (method, path) in [
        (Method::HEAD, "/"),
        (Method::GET, "/204"),
        (Method::GET, "/304"),
    ] {
        let response = Client::new()
            .send(
                Request::new(method.clone(), &server.url(path))
                    .unwrap()
                    .policy(AddressPolicy::Anywhere),
            )
            .await
            .unwrap();
        // The headers are the server's, untouched.
        assert_eq!(response.headers[CONTENT_ENCODING], "br", "{method} {path}");
        let body = response.body.collect().await.unwrap();
        assert!(body.is_empty(), "{method} {path}");
    }
}

#[tokio::test]
async fn a_body_that_ends_inside_the_stream_is_an_error_after_what_was_good() {
    let page = asset("page.txt");
    for (file, coding) in [
        ("page.gz", "gzip"),
        ("page.zlib", "deflate"),
        ("page.deflate", "deflate"),
    ] {
        let whole = asset(file);
        // Cut in the header, in the middle, and just short of the trailer.
        for cut in [1, 5, whole.len() / 2, whole.len() - 5, whole.len() - 1] {
            for framing in [Framing::Length, Framing::Chunked(100)] {
                let server = serve(Some(coding), whole[..cut].to_vec(), framing).await;
                let what = format!("{file} cut at {cut} {framing:?}");
                let error = collected(local(&server.url("/"))).await.unwrap_err();
                assert!(matches!(error, Error::Body(_)), "{what}: {error}");
                let (got, error, _) = streamed(local(&server.url("/"))).await;
                assert!(matches!(error, Some(Error::Body(_))), "{what}: {error:?}");
                // What arrived before the failure is a prefix of the page.
                assert!(page.starts_with(&got), "{what}");
            }
        }
    }
}

#[tokio::test]
async fn a_body_cut_by_the_connection_is_an_error_too() {
    // A length that promises more than comes: the framing fails first.
    let whole = asset("page.gz");
    let server = Server::plain(handler(move |mut conn| {
        let whole = whole.clone();
        async move {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                whole.len()
            );
            conn.write(head.as_bytes()).await;
            conn.write(&whole[..whole.len() / 2]).await;
        }
    }))
    .await;
    let error = collected(local(&server.url("/"))).await.unwrap_err();
    assert!(matches!(error, Error::Body(_)), "{error}");
}

#[tokio::test]
async fn bytes_after_the_stream_are_an_error_but_another_gzip_member_is_not() {
    for (file, coding) in [
        ("page.gz", "gzip"),
        ("page.zlib", "deflate"),
        ("page.deflate", "deflate"),
    ] {
        for junk in [&b"\n"[..], b"\0\0\0\0", b"trailing text"] {
            let mut body = asset(file);
            body.extend_from_slice(junk);
            for framing in [Framing::Length, Framing::Drip] {
                let server = serve(Some(coding), body.clone(), framing).await;
                let (got, error, _) = streamed(local(&server.url("/"))).await;
                assert!(
                    matches!(error, Some(Error::Body(_))),
                    "{file} {junk:?}: {error:?}"
                );
                // The page itself came through before the refusal.
                assert_eq!(got, asset("page.txt"), "{file} {junk:?}");
            }
        }
    }
    // Members one after the other are one body.
    let server = serve(Some("gzip"), asset("page.gz-multi"), Framing::Length).await;
    assert_eq!(
        collected(local(&server.url("/"))).await.unwrap(),
        asset("page.txt")
    );
}

#[tokio::test]
async fn a_damaged_stream_is_an_error() {
    for (file, coding) in [("page.gz", "gzip"), ("page.zlib", "deflate")] {
        let whole = asset(file);
        let mut failures = 0;
        for at in (0..whole.len()).step_by(37) {
            let mut damaged = whole.clone();
            damaged[at] ^= 0x10;
            let server = serve(Some(coding), damaged, Framing::Length).await;
            match collected(local(&server.url("/"))).await {
                Err(Error::Body(_)) => failures += 1,
                // A flipped bit in a header field that nothing checks (the
                // timestamp, the operating system) changes nothing.
                Ok(got) => assert_eq!(got, asset("page.txt"), "{file} at {at}"),
                Err(other) => panic!("{file} at {at}: {other}"),
            }
        }
        assert!(failures > whole.len() / 37 / 2, "{file}: {failures}");
    }
}

#[tokio::test]
async fn pieces_are_small_whatever_the_answer_inflates_to() {
    // Eight mebibytes from eight kilobytes, handed over a piece at a time.
    let server = serve(Some("gzip"), asset("bomb-8m.gz"), Framing::Length).await;
    let (got, error, sizes) = streamed(local(&server.url("/")).max_response_bytes(16 << 20)).await;
    assert!(error.is_none(), "{error:?}");
    assert_eq!(got.len(), 8 << 20);
    assert!(got.iter().all(|byte| *byte == 0));
    assert!(sizes.iter().all(|size| *size <= 64 * 1024), "{sizes:?}");
}

#[tokio::test]
async fn the_ceiling_counts_what_the_caller_receives() {
    let page = asset("page.txt");
    let wire = asset("page.gz");
    assert!(wire.len() * 4 < page.len());
    let server = serve(Some("gzip"), wire.clone(), Framing::Length).await;

    // Room for the page decoded: arrives.
    let got = collected(local(&server.url("/")).max_response_bytes(page.len() as u64 + 4096))
        .await
        .unwrap();
    assert_eq!(got, page);

    // Room for the compressed bytes and not for the page: refused, though
    // the wire fits many times over.
    let ceiling = (page.len() / 2) as u64;
    assert!(ceiling > wire.len() as u64 + 1024);
    let error = collected(local(&server.url("/")).max_response_bytes(ceiling))
        .await
        .unwrap_err();
    assert!(error.is_too_large(), "{error}");
    let (got, error, _) = streamed(local(&server.url("/")).max_response_bytes(ceiling)).await;
    assert!(error.unwrap().is_too_large());
    assert!(got.len() as u64 <= ceiling, "{} over {ceiling}", got.len());

    // The same page, uncompressed, is held to the same number.
    let plain = serve(None, page.clone(), Framing::Length).await;
    let error = collected(local(&plain.url("/")).max_response_bytes(ceiling))
        .await
        .unwrap_err();
    assert!(error.is_too_large());
}

#[tokio::test]
async fn read_up_to_counts_decoded_bytes_and_leaves_the_rest_unread() {
    let server = serve(Some("gzip"), asset("bomb-8m.gz"), Framing::Length).await;
    let response = Client::new()
        .send(local(&server.url("/")).max_response_bytes(1 << 20))
        .await
        .unwrap();
    let mut body = response.body;
    let (bytes, more) = body.read_up_to(1000).await.unwrap();
    assert_eq!(bytes.len(), 1000);
    assert!(more);

    let page = asset("page.txt");
    let server = serve(Some("gzip"), asset("page.gz"), Framing::Length).await;
    let response = Client::new().send(local(&server.url("/"))).await.unwrap();
    let mut body = response.body;
    let (bytes, more) = body.read_up_to(page.len()).await.unwrap();
    assert_eq!(bytes.as_ref(), page.as_slice());
    assert!(!more);
}

#[tokio::test]
async fn turned_off_the_bytes_come_as_they_are() {
    let wire = asset("page.gz");
    let server = serve(Some("gzip"), wire.clone(), Framing::Length).await;
    let response = Client::new()
        .send(local(&server.url("/")).decompress(false))
        .await
        .unwrap();
    assert_eq!(response.headers[CONTENT_ENCODING], "gzip");
    assert_eq!(
        response.headers[CONTENT_LENGTH],
        HeaderValue::from_str(&wire.len().to_string()).unwrap()
    );
    assert_eq!(response.body.collect().await.unwrap().as_ref(), wire);

    // Even a coding nobody here could undo passes through untouched.
    let server = serve(Some("br"), b"opaque".to_vec(), Framing::Length).await;
    let got = collected(local(&server.url("/")).decompress(false)).await;
    assert_eq!(got.unwrap(), b"opaque");
}

#[tokio::test]
async fn each_redirect_hop_is_decoded_on_its_own() {
    let page = asset("page.txt");
    let wire = asset("page.gz");
    let server = Server::plain(handler(move |mut conn| {
        let wire = wire.clone();
        async move {
            if conn.request.target == "/start" {
                conn.redirect(302, "/end").await;
            } else {
                answer(&mut conn, Some("gzip"), &wire, Framing::Length).await;
            }
        }
    }))
    .await;
    let got = collected(local(&server.url("/start")).max_redirects(2)).await;
    assert_eq!(got.unwrap(), page);
}

/// What a host sends for `Accept-Encoding: gzip`, seen on the wire by a
/// plain TLS connection and not by the client under test: the length of the
/// body as sent, and the coding it names.
async fn wire_view(host: &str, path: &str) -> (usize, Option<String>) {
    use tokio::io::AsyncReadExt;
    let roots = stellarr_outbound::tls::system_roots().unwrap();
    let connector = tokio_rustls::TlsConnector::from(stellarr_outbound::tls::client_config(roots));
    let tcp = tokio::net::TcpStream::connect((host, 443)).await.unwrap();
    let name = rustls::pki_types::ServerName::try_from(host.to_owned()).unwrap();
    let mut tls = connector.connect(name, tcp).await.unwrap();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: Mozilla/5.0 (compatible; link preview check)\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n"
    );
    tls.write_all(request.as_bytes()).await.unwrap();
    let mut all = Vec::new();
    let _ = tls.read_to_end(&mut all).await;
    let split = all.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&all[..split]).to_lowercase();
    let coding = head
        .lines()
        .find_map(|line| line.strip_prefix("content-encoding:"))
        .map(|value| value.trim().to_owned());
    (all.len() - split - 4, coding)
}

/// Run once, by hand: `acargo test -p stellarr-outbound --test encoding --
/// --ignored --nocapture`. A real host that compresses, over TLS and the
/// system's roots.
#[tokio::test]
#[ignore = "needs the network"]
async fn a_real_site_that_compresses() {
    let host = "www.wikipedia.org";
    let request = || {
        Request::get(&format!("https://{host}/"))
            .unwrap()
            .header(
                http::header::USER_AGENT,
                HeaderValue::from_static("Mozilla/5.0 (compatible; link preview check)"),
            )
            .timeout(Duration::from_secs(30))
            .max_response_bytes(8 << 20)
    };
    let client = Client::new();
    let decoded = client.send_collected(request()).await.unwrap();
    let plain = client
        .send_collected(request().decompress(false))
        .await
        .unwrap();
    let (on_the_wire, coding) = wire_view(host, "/").await;
    eprintln!(
        "decoded {} bytes; identity {} bytes; gzip on the wire {} bytes, Content-Encoding {:?}",
        decoded.body.len(),
        plain.body.len(),
        on_the_wire,
        coding
    );
    assert!(decoded.status.is_success());
    // The host really did compress, and this client undid it.
    assert_eq!(coding.as_deref(), Some("gzip"));
    assert!(on_the_wire * 2 < decoded.body.len());
    assert!(decoded.headers.get(CONTENT_ENCODING).is_none());
    assert_eq!(decoded.body.len(), plain.body.len());
    let text = String::from_utf8_lossy(&decoded.body).to_lowercase();
    assert!(text.contains("<html") && text.contains("wikipedia"));
}
