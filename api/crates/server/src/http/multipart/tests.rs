use crate::http::stream;

use super::*;

fn headers(content_type: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, content_type.parse().unwrap());
    headers
}

/// A body delivered in pieces of `piece` bytes.
fn body_in_pieces(data: Vec<u8>, piece: usize) -> Body {
    let (sender, body) = stream::channel(4);
    tokio::spawn(async move {
        for chunk in data.chunks(piece.max(1)) {
            if sender
                .send(Ok(Bytes::copy_from_slice(chunk)))
                .await
                .is_err()
            {
                return;
            }
        }
    });
    body
}

fn form(boundary: &str, parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (head, body) in parts {
        out.extend_from_slice(format!("--{boundary}\r\n{head}\r\n\r\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    out
}

struct Read {
    heads: Vec<PartHead>,
    bodies: Vec<Vec<u8>>,
}

async fn read_all(data: Vec<u8>, piece: usize, content_type: &str) -> Result<Read, FormError> {
    let mut reader = Multipart::open(&headers(content_type), body_in_pieces(data, piece), 1 << 20)?;
    let mut read = Read {
        heads: Vec::new(),
        bodies: Vec::new(),
    };
    while let Some(head) = reader.next_part().await? {
        let mut body = Vec::new();
        while let Some(chunk) = reader.chunk().await? {
            body.extend_from_slice(&chunk);
        }
        read.heads.push(head);
        read.bodies.push(body);
    }
    Ok(read)
}

const TYPE: &str = "multipart/form-data; boundary=XXB";

#[tokio::test]
async fn parts_are_read_whatever_the_size_of_the_pieces_the_body_comes_in() {
    let data = form(
        "XXB",
        &[
            ("Content-Disposition: form-data; name=\"a\"", b"one"),
            (
                "Content-Disposition: form-data; name=\"file\"; filename=\"x.png\"\r\nContent-Type: image/png",
                b"\x89PNG\r\n--XX not it\r\n-",
            ),
            ("Content-Disposition: form-data; name=\"empty\"", b""),
        ],
    );
    for piece in [1, 2, 3, 5, 7, 16, 1000] {
        let read = read_all(data.clone(), piece, TYPE).await.unwrap();

        assert_eq!(
            read.bodies,
            [
                b"one".to_vec(),
                b"\x89PNG\r\n--XX not it\r\n-".to_vec(),
                Vec::new()
            ],
            "{piece}"
        );
        assert_eq!(
            read.heads[1].header("content-type").as_deref(),
            Some("image/png")
        );
    }
}

#[tokio::test]
async fn a_preamble_an_epilogue_and_the_rest_of_a_boundary_line_are_ignored() {
    let mut data = b"preamble --X\r\n".to_vec();
    data.extend_from_slice(b"--XXB   junk\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\nv\r\n--XXB--trailing\r\nepilogue");

    let read = read_all(data, 3, TYPE).await.unwrap();

    assert_eq!(read.bodies, [b"v".to_vec()]);
}

#[tokio::test]
async fn the_closing_boundary_may_end_the_body() {
    let data = b"--XXB\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\nv\r\n--XXB--".to_vec();

    assert_eq!(
        read_all(data, 4, TYPE).await.unwrap().bodies,
        [b"v".to_vec()]
    );
}

#[tokio::test]
async fn a_body_that_stops_before_the_closing_boundary_is_refused() {
    let whole = form(
        "XXB",
        &[("Content-Disposition: form-data; name=\"a\"", b"v")],
    );
    for cut in 0..whole.len() - 4 {
        let result = read_all(whole[..cut].to_vec(), 5, TYPE).await;

        assert_eq!(result.err(), Some(FormError::Malformed), "cut at {cut}");
    }
}

#[tokio::test]
async fn bare_line_feeds_do_not_end_a_line() {
    let data = b"--XXB\nContent-Disposition: form-data; name=\"a\"\n\nv\n--XXB--\n".to_vec();

    assert!(read_all(data, 8, TYPE).await.is_err());
}

#[tokio::test]
async fn a_header_needs_a_name_and_a_colon() {
    for head in ["no colon here", ": value", " continued without a colon"] {
        let data = form("XXB", &[(head, b"v")]);

        assert_eq!(
            read_all(data, 8, TYPE).await.err(),
            Some(FormError::Malformed),
            "{head}"
        );
    }
}

#[tokio::test]
async fn sixteen_headers_are_read_and_seventeen_are_refused() {
    let lines = |count: usize| -> String {
        (0..count)
            .map(|n| format!("X-H{n}: 1"))
            .collect::<Vec<_>>()
            .join("\r\n")
    };

    let sixteen = read_all(form("XXB", &[(&lines(16), b"v")]), 100, TYPE)
        .await
        .unwrap();
    let seventeen = read_all(form("XXB", &[(&lines(17), b"v")]), 100, TYPE).await;

    assert_eq!(sixteen.heads[0].headers.len(), 16);
    assert_eq!(seventeen.err(), Some(FormError::Malformed));
}

#[tokio::test]
async fn the_header_lines_of_a_part_may_add_up_to_just_under_sixteen_kib() {
    // The two header lines total 16,325 bytes.
    let lines = |pad: usize| {
        format!(
            "X-P: {}\r\nContent-Disposition: form-data; name=\"a\"",
            "a".repeat(pad)
        )
    };
    let longest =
        MAX_HEADER_BYTES - "Content-Disposition: form-data; name=\"a\"".len() - "X-P: ".len() - 1;

    assert!(
        read_all(form("XXB", &[(&lines(longest), b"v")]), 4096, TYPE)
            .await
            .is_ok()
    );
    assert_eq!(
        read_all(form("XXB", &[(&lines(longest + 1), b"v")]), 4096, TYPE)
            .await
            .err(),
        Some(FormError::Malformed)
    );
}

#[tokio::test]
async fn a_header_that_never_ends_is_refused_without_reading_it_all() {
    let mut data = b"--XXB\r\nX-P: ".to_vec();
    data.extend(std::iter::repeat_n(b'a', 100_000));

    assert_eq!(
        read_all(data, 4096, TYPE).await.err(),
        Some(FormError::Malformed)
    );
}

#[tokio::test]
async fn a_thousand_and_twenty_four_parts_are_read_and_one_more_is_refused() {
    let part = ("Content-Disposition: form-data; name=\"a\"", b"v" as &[u8]);

    let most = read_all(form("XXB", &vec![part; 1024]), 1000, TYPE)
        .await
        .unwrap();
    let over = read_all(form("XXB", &vec![part; 1025]), 1000, TYPE).await;

    assert_eq!(most.heads.len(), 1024);
    assert_eq!(over.err(), Some(FormError::Malformed));
}

#[tokio::test]
async fn the_boundary_is_taken_from_the_content_type() {
    let data = form(
        "b-1",
        &[("Content-Disposition: form-data; name=\"a\"", b"v")],
    );
    for content_type in [
        "multipart/form-data; boundary=b-1",
        "multipart/form-data; boundary=\"b-1\"",
        "MULTIPART/Form-Data; charset=utf-8; BOUNDARY=b-1",
        "multipart/form-data;boundary=b-1",
    ] {
        assert_eq!(
            read_all(data.clone(), 9, content_type)
                .await
                .unwrap()
                .bodies
                .len(),
            1,
            "{content_type}"
        );
    }
}

#[tokio::test]
async fn a_content_type_that_names_no_usable_boundary_is_refused() {
    let data = form("b", &[("Content-Disposition: form-data; name=\"a\"", b"v")]);
    let long = format!("multipart/form-data; boundary={}", "b".repeat(129));
    let longest = format!("multipart/form-data; boundary={}", "b".repeat(128));

    for content_type in [
        "multipart/form-data",
        "multipart/form-data; boundary=",
        "multipart/form-data; boundary=\"\"",
        &long,
    ] {
        assert_eq!(
            read_all(data.clone(), 9, content_type).await.err(),
            Some(FormError::Malformed),
            "{content_type}"
        );
    }
    assert!(Multipart::open(&headers(&longest), Body::empty(), 10).is_ok());
}

#[tokio::test]
async fn a_request_that_is_no_form_is_refused_as_such() {
    for content_type in [
        "application/json",
        "multipart/mixed; boundary=b",
        "text/plain",
    ] {
        assert_eq!(
            Multipart::open(&headers(content_type), Body::empty(), 10).err(),
            Some(FormError::NotAForm),
            "{content_type}"
        );
    }
    // A request that names no type at all is a bad request, not another kind.
    assert_eq!(
        Multipart::open(&HeaderMap::new(), Body::empty(), 10).err(),
        Some(FormError::Malformed)
    );
    assert_eq!(
        Multipart::open(
            &headers("application/x-www-form-urlencoded"),
            Body::empty(),
            10
        )
        .err(),
        Some(FormError::NotAForm)
    );
}

#[tokio::test]
async fn a_declared_length_over_the_ceiling_is_refused_before_anything_is_read() {
    let mut declared = headers(TYPE);
    declared.insert(header::CONTENT_LENGTH, "11".parse().unwrap());

    assert_eq!(
        Multipart::open(&declared, Body::empty(), 10).err(),
        Some(FormError::TooLarge)
    );
    declared.insert(header::CONTENT_LENGTH, "10".parse().unwrap());
    assert!(Multipart::open(&declared, Body::empty(), 10).is_ok());
}

#[tokio::test]
async fn a_body_that_outgrows_the_ceiling_is_cut_off() {
    let data = form(
        "XXB",
        &[(
            "Content-Disposition: form-data; name=\"a\"",
            &vec![b'x'; 5000],
        )],
    );
    let mut reader = Multipart::open(&headers(TYPE), body_in_pieces(data, 700), 2000).unwrap();

    let mut outcome = Ok(());
    loop {
        match reader.next_part().await {
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(error) => {
                outcome = Err(error);
                break;
            }
        }
        loop {
            match reader.chunk().await {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(error) => {
                    outcome = Err(error);
                    break;
                }
            }
        }
        if outcome.is_err() {
            break;
        }
    }

    assert_eq!(outcome, Err(FormError::TooLarge));
}

#[tokio::test]
async fn a_body_that_is_cut_off_by_the_client_is_refused() {
    let (sender, body) = stream::channel(1);
    tokio::spawn(async move {
        let _ = sender
            .send(Ok(Bytes::from_static(b"--XXB\r\nContent-Disp")))
            .await;
        let _ = sender
            .send(Err(std::io::Error::other("the client went away")))
            .await;
    });
    let mut reader = Multipart::open(&headers(TYPE), body, 1 << 20).unwrap();

    assert_eq!(reader.next_part().await.err(), Some(FormError::Malformed));
}

#[tokio::test(start_paused = true)]
async fn a_client_that_sends_nothing_is_cut_off() {
    let (sender, body) = stream::channel(1);
    let held = tokio::spawn(async move {
        let _ = sender.send(Ok(Bytes::from_static(b"--XXB\r\n"))).await;
        tokio::time::sleep(Duration::from_secs(3600)).await;
        drop(sender);
    });
    let mut reader = Multipart::open(&headers(TYPE), body, 1 << 20).unwrap();

    assert_eq!(reader.next_part().await.err(), Some(FormError::TooSlow));
    held.abort();
}

#[tokio::test(start_paused = true)]
async fn a_client_that_trickles_is_cut_off_and_one_that_is_fast_is_not() {
    let trickle = |gap: Duration, bytes: usize| async move {
        let (sender, body) = stream::channel(1);
        let task = tokio::spawn(async move {
            let _ = sender
                .send(Ok(Bytes::from_static(
                    b"--XXB\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n",
                )))
                .await;
            for _ in 0..1000 {
                tokio::time::sleep(gap).await;
                if sender
                    .send(Ok(Bytes::from(vec![b'x'; bytes])))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        });
        let mut reader = Multipart::open(&headers(TYPE), body, 1 << 30).unwrap();
        let outcome = async {
            reader.next_part().await?;
            while reader.chunk().await?.is_some() {}
            Ok::<(), FormError>(())
        }
        .await;
        task.abort();
        outcome
    };

    // One byte every four seconds is under any rate; a megabyte every four is not.
    assert_eq!(
        trickle(Duration::from_secs(4), 1).await,
        Err(FormError::TooSlow)
    );
    assert_eq!(
        trickle(Duration::from_secs(4), 1 << 20).await,
        Err(FormError::Malformed)
    );
}

mod disposition {
    use super::super::*;

    fn parse(text: &str) -> Disposition {
        Disposition::parse(text).unwrap_or_else(|| panic!("{text} should be read"))
    }

    #[test]
    fn a_file_has_a_name_and_a_file_name() {
        let d = parse("form-data; name=\"file\"; filename=\"a b.png\"");

        assert_eq!(
            (d.kind, d.name.as_str(), d.file_name.as_deref()),
            (Kind::File, "file", Some("a b.png"))
        );
    }

    #[test]
    fn a_value_has_no_file_name() {
        for text in [
            "form-data; name=\"x\"",
            "form-data; name=x",
            "form-data;name=\"x\";filename=\"\"",
            "form-data",
        ] {
            assert_eq!(parse(text).kind, Kind::Field, "{text}");
        }
    }

    #[test]
    fn the_extended_file_name_wins_and_is_decoded() {
        let d = parse(
            "form-data; name=\"f\"; filename=\"plain.png\"; filename*=UTF-8''z%C3%B3%C5%82w.png",
        );
        assert_eq!(d.file_name.as_deref(), Some("zółw.png"));
        let latin = parse("form-data; name=\"f\"; filename*=iso-8859-1''caf%E9.png");
        assert_eq!(latin.file_name.as_deref(), Some("café.png"));
        let unknown = parse("form-data; name=\"f\"; filename*=klingon''x");
        assert_eq!(unknown.file_name.as_deref(), Some("klingon''x"));
    }

    #[test]
    fn parameter_names_are_not_case_sensitive_and_the_type_is() {
        assert_eq!(
            parse("form-data; NAME=\"f\"; FileName=\"a\"").kind,
            Kind::File
        );
        assert_eq!(
            parse("FORM-DATA; name=\"f\"; filename=\"a\"").kind,
            Kind::Other
        );
        assert_eq!(
            parse("attachment; name=\"f\"; filename=\"a\"").kind,
            Kind::Other
        );
    }

    #[test]
    fn a_quoted_value_may_hold_semicolons_and_escaped_quotes() {
        let d = parse("form-data; name=\"f\"; filename=\"a;b\\\"c.png\"");

        assert_eq!(d.file_name.as_deref(), Some("a;b\\\"c.png"));
    }

    #[test]
    fn the_first_of_two_names_counts() {
        assert_eq!(parse("form-data; name=\"x\"; name=\"file\"").name, "x");
    }

    #[test]
    fn what_is_not_a_disposition_is_refused() {
        for text in [
            "",
            "   ",
            "form data",
            "form-data; name=\"unterminated",
            "form-data; =x",
            "; name=x",
        ] {
            assert!(Disposition::parse(text).is_none(), "{text:?}");
        }
    }

    #[test]
    fn a_parameter_without_a_value_is_read_as_empty() {
        let d = parse("form-data; name; filename=\"a.png\"");

        assert_eq!((d.kind, d.name.as_str()), (Kind::File, ""));
    }
}
