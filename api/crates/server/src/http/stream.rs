//! A response body that a task of its own fills, chunk by chunk.
//!
//! [`channel`] gives the sender the task writes to and the `Body` the response
//! carries. A chunk is sent with `sender.send(Ok(bytes)).await`, which waits
//! while the client is behind, so the memory held is the buffer and not the
//! content. A failure is sent as `Err(error)` and ends the body abnormally (the
//! connection is closed, so that the client does not take a short body for a
//! whole one). Dropping the sender ends the body normally, after every chunk
//! that was sent. `send` fails when the client is gone: stop then.
//!
//! This is written here and not taken from `http_body_util::channel::Channel`,
//! whose `poll_frame` looks at the chunks first and at the sender being gone
//! after: a sender that sends its last chunk and ends between the two looks
//! like a body that ended with no content. A download of a small file lost its
//! only chunk that way about once in four thousand requests, and hyper, having
//! promised a length, closed the connection with nothing, or only the head,
//! sent. Reading the one channel that carries both the chunks and the end has no
//! such gap.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::Body;
use bytes::Bytes;
use hyper::body::{Body as HttpBody, Frame};
use tokio::sync::mpsc;

struct Chunks(mpsc::Receiver<io::Result<Bytes>>);

impl HttpBody for Chunks {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        self.0
            .poll_recv(cx)
            .map(|item| item.map(|chunk| chunk.map(Frame::data)))
    }
}

/// A body and the sender that fills it, `buffered` chunks ahead of the client
/// at most.
pub fn channel(buffered: usize) -> (mpsc::Sender<io::Result<Bytes>>, Body) {
    let (sender, receiver) = mpsc::channel(buffered);
    (sender, Body::new(Chunks(receiver)))
}

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt;

    use super::*;

    #[tokio::test]
    async fn every_chunk_sent_before_the_sender_ends_is_read_then_the_body_ends() {
        let (sender, body) = channel(2);
        let writer = tokio::spawn(async move {
            for part in ["a", "bc", "def"] {
                sender.send(Ok(Bytes::from(part))).await.unwrap();
            }
        });

        let read = body.collect().await.unwrap().to_bytes();

        writer.await.unwrap();
        assert_eq!(&read[..], b"abcdef");
    }

    #[tokio::test]
    async fn a_failure_ends_the_body_with_an_error_after_the_chunks_before_it() {
        let (sender, mut body) = channel(2);
        sender.send(Ok(Bytes::from("ab"))).await.unwrap();
        sender
            .send(Err(io::ErrorKind::UnexpectedEof.into()))
            .await
            .unwrap();

        let first = body.frame().await.unwrap().unwrap();
        let second = body.frame().await.unwrap();

        assert_eq!(first.into_data().unwrap(), "ab");
        assert!(second.is_err());
    }

    #[tokio::test]
    async fn a_sender_whose_client_is_gone_is_told() {
        let (sender, body) = channel(1);
        drop(body);

        assert!(sender.send(Ok(Bytes::from("a"))).await.is_err());
    }
}
