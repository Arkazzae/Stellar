//! Requests sent as bytes, so that what a hostile client writes arrives as it
//! was written: the support client builds a URI and cannot send `%00`, a
//! backslash, a bare odd byte or a request line of its own.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// An answer, read whole.
pub struct Answer {
    /// `0` when the connection ended before a whole head was read, which is
    /// what a server that refuses a request it has not read to the end may
    /// do.
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Answer {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(have, _)| have.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// `request` exactly as given (the request line, the headers, the blank line
/// and whatever follows), sent over a connection of its own that is read to
/// the end.
pub async fn raw_request(address: SocketAddr, request: &[u8]) -> Answer {
    let mut stream = TcpStream::connect(address).await.expect("a connection");
    // The server may answer and close before the last byte is written.
    let _ = stream.write_all(request).await;
    let mut everything = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), stream.read_to_end(&mut everything))
        .await
        .expect("an answer within a minute")
        .ok();
    parse_answer(&everything)
}

/// `GET target` with `headers`, `target` byte for byte. The connection is
/// closed after the answer.
pub async fn raw_get(address: SocketAddr, target: &[u8], headers: &[(&str, &str)]) -> Answer {
    raw_send(address, "GET", target, headers).await
}

/// The same with the method of your choice.
pub async fn raw_send(
    address: SocketAddr,
    method: &str,
    target: &[u8],
    headers: &[(&str, &str)],
) -> Answer {
    let mut request = format!("{method} ").into_bytes();
    request.extend_from_slice(target);
    request.extend_from_slice(b" HTTP/1.1\r\nHost: oracle.test\r\nConnection: close\r\n");
    for (name, value) in headers {
        request.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    request.extend_from_slice(b"\r\n");
    raw_request(address, &request).await
}

pub fn parse_answer(bytes: &[u8]) -> Answer {
    let Some(split) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
        return Answer {
            status: 0,
            headers: Vec::new(),
            body: Vec::new(),
        };
    };
    let head = String::from_utf8_lossy(&bytes[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse().ok())
        .expect("a status");
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();
    let rest = &bytes[split + 4..];
    let chunked = headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("transfer-encoding") && value.contains("chunked")
    });
    let body = if chunked {
        unchunk(rest)
    } else {
        rest.to_vec()
    };
    Answer {
        status,
        headers,
        body,
    }
}

fn unchunk(mut rest: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    loop {
        let Some(line_end) = rest.windows(2).position(|pair| pair == b"\r\n") else {
            return body;
        };
        let size = usize::from_str_radix(String::from_utf8_lossy(&rest[..line_end]).trim(), 16)
            .unwrap_or(0);
        rest = &rest[line_end + 2..];
        if size == 0 || rest.len() < size {
            return body;
        }
        body.extend_from_slice(&rest[..size]);
        rest = rest.get(size + 2..).unwrap_or(&[]);
    }
}
