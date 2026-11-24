use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

use super::*;
use crate::frame::{OP_BINARY, OP_CLOSE, OP_CONTINUATION, OP_PING, OP_PONG, OP_TEXT, encode_frame};

const KEY: [u8; 4] = [0x12, 0x34, 0x56, 0x78];

struct Client {
    io: DuplexStream,
}

impl Client {
    async fn send(&mut self, fin: bool, opcode: u8, payload: &[u8]) {
        self.io
            .write_all(&encode_frame(fin, opcode, payload, Some(KEY)))
            .await
            .unwrap();
    }

    async fn send_raw(&mut self, bytes: &[u8]) {
        self.io.write_all(bytes).await.unwrap();
    }

    /// One frame the server wrote: opcode, fin, payload.
    async fn read(&mut self) -> (u8, bool, Vec<u8>) {
        let mut head = [0u8; 2];
        self.io.read_exact(&mut head).await.unwrap();
        assert_eq!(head[1] & 0x80, 0, "a server never masks");
        let len = match head[1] & 0x7f {
            126 => {
                let mut b = [0u8; 2];
                self.io.read_exact(&mut b).await.unwrap();
                usize::from(u16::from_be_bytes(b))
            }
            127 => {
                let mut b = [0u8; 8];
                self.io.read_exact(&mut b).await.unwrap();
                u64::from_be_bytes(b) as usize
            }
            n => usize::from(n),
        };
        let mut payload = vec![0u8; len];
        self.io.read_exact(&mut payload).await.unwrap();
        (head[0] & 0x0f, head[0] & 0x80 != 0, payload)
    }

    async fn read_close_code(&mut self) -> u16 {
        let (opcode, _, payload) = self.read().await;
        assert_eq!(opcode, OP_CLOSE);
        u16::from_be_bytes([payload[0], payload[1]])
    }
}

fn pair(config: Config) -> (Client, Sender<DuplexStream>, Receiver<DuplexStream>) {
    let (client, server) = tokio::io::duplex(1 << 20);
    let (sender, receiver) = split(server, config);
    (Client { io: client }, sender, receiver)
}

async fn message(receiver: &mut Receiver<DuplexStream>) -> (MessageKind, Vec<u8>) {
    let mut out = Vec::new();
    loop {
        match receiver.next().await.unwrap() {
            Event::Data { kind, bytes, end } => {
                out.extend_from_slice(&bytes);
                if end {
                    return (kind, out);
                }
            }
            Event::Ping | Event::Pong => {}
            Event::Close(frame) => panic!("closed: {frame:?}"),
        }
    }
}

#[tokio::test]
async fn a_masked_text_message_arrives_whole() {
    let (mut client, _sender, mut receiver) = pair(Config::default());
    client.send(true, OP_TEXT, b"Hello").await;
    assert_eq!(
        message(&mut receiver).await,
        (MessageKind::Text, b"Hello".to_vec())
    );
}

#[tokio::test]
async fn a_fragmented_message_with_a_ping_in_the_middle() {
    let (mut client, _sender, mut receiver) = pair(Config::default());
    client.send(false, OP_TEXT, b"Hel").await;
    client.send(true, OP_PING, b"are you there").await;
    client.send(false, OP_CONTINUATION, b"lo, ").await;
    client.send(true, OP_CONTINUATION, b"world").await;
    assert_eq!(
        message(&mut receiver).await,
        (MessageKind::Text, b"Hello, world".to_vec())
    );
    let (opcode, fin, payload) = client.read().await;
    assert_eq!(
        (opcode, fin, payload.as_slice()),
        (OP_PONG, true, b"are you there".as_slice())
    );
}

#[tokio::test]
async fn a_ping_is_reported_and_answered_with_its_own_bytes() {
    let (mut client, _sender, mut receiver) = pair(Config::default());
    client.send(true, OP_PING, b"").await;
    assert_eq!(receiver.next().await.unwrap(), Event::Ping);
    let (opcode, _, payload) = client.read().await;
    assert_eq!((opcode, payload.len()), (OP_PONG, 0));
}

#[tokio::test]
async fn an_unsolicited_pong_is_reported_and_nothing_is_sent() {
    let (mut client, sender, mut receiver) = pair(Config::default());
    client.send(true, OP_PONG, b"x").await;
    assert_eq!(receiver.next().await.unwrap(), Event::Pong);
    sender.send_text(b"after").await.unwrap();
    let (opcode, _, payload) = client.read().await;
    assert_eq!((opcode, payload.as_slice()), (OP_TEXT, b"after".as_slice()));
}

#[tokio::test]
async fn a_message_dribbled_one_byte_at_a_time_is_the_same_message() {
    let (mut client, _sender, mut receiver) = pair(Config::default());
    let wire = encode_frame(true, OP_BINARY, &[9u8; 300], Some(KEY));
    let writer = tokio::spawn(async move {
        for byte in wire {
            client.io.write_all(&[byte]).await.unwrap();
            tokio::task::yield_now().await;
        }
        client
    });
    assert_eq!(
        message(&mut receiver).await,
        (MessageKind::Binary, vec![9u8; 300])
    );
    drop(writer.await.unwrap());
}

#[tokio::test]
async fn a_large_frame_comes_out_in_pieces_that_add_up() {
    let (mut client, _sender, mut receiver) = pair(Config {
        read_buffer: 1024,
        ..Config::default()
    });
    let payload: Vec<u8> = (0..200_000u32).map(|n| (n % 251) as u8).collect();
    client.send(true, OP_BINARY, &payload).await;
    let mut pieces = 0;
    let mut got = Vec::new();
    loop {
        match receiver.next().await.unwrap() {
            Event::Data { bytes, end, .. } => {
                pieces += 1;
                got.extend_from_slice(&bytes);
                if end {
                    break;
                }
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(got, payload);
    assert!(pieces > 1, "the frame was not buffered whole");
}

#[tokio::test]
async fn an_empty_message_and_empty_fragments() {
    let (mut client, _sender, mut receiver) = pair(Config::default());
    client.send(true, OP_TEXT, b"").await;
    assert_eq!(message(&mut receiver).await, (MessageKind::Text, vec![]));
    client.send(false, OP_BINARY, b"").await;
    client.send(false, OP_CONTINUATION, b"a").await;
    client.send(true, OP_CONTINUATION, b"").await;
    assert_eq!(
        message(&mut receiver).await,
        (MessageKind::Binary, b"a".to_vec())
    );
}

#[tokio::test]
async fn a_character_split_between_fragments_is_valid_text() {
    let (mut client, _sender, mut receiver) = pair(Config::default());
    let euro = "\u{20ac}".as_bytes();
    client.send(false, OP_TEXT, &euro[..1]).await;
    client.send(false, OP_CONTINUATION, &euro[1..2]).await;
    client.send(true, OP_CONTINUATION, &euro[2..]).await;
    assert_eq!(
        message(&mut receiver).await,
        (MessageKind::Text, euro.to_vec())
    );
}

#[tokio::test]
async fn invalid_utf8_closes_with_1007() {
    for bad in [&[0xffu8][..], &[0xc0, 0x80], &[0xed, 0xa0, 0x80]] {
        let (mut client, _sender, mut receiver) = pair(Config::default());
        client.send(true, OP_TEXT, bad).await;
        assert!(matches!(receiver.next().await, Err(Error::InvalidUtf8)));
        assert_eq!(client.read_close_code().await, 1007);
    }
}

#[tokio::test]
async fn a_text_message_that_stops_inside_a_character_closes_with_1007() {
    let (mut client, _sender, mut receiver) = pair(Config::default());
    client
        .send(true, OP_TEXT, &"\u{20ac}".as_bytes()[..2])
        .await;
    assert!(matches!(receiver.next().await, Err(Error::InvalidUtf8)));
    assert_eq!(client.read_close_code().await, 1007);
}

#[tokio::test]
async fn binary_messages_are_not_checked_for_utf8() {
    let (mut client, _sender, mut receiver) = pair(Config::default());
    client.send(true, OP_BINARY, &[0xff, 0xfe]).await;
    assert_eq!(
        message(&mut receiver).await,
        (MessageKind::Binary, vec![0xff, 0xfe])
    );
}

#[tokio::test]
async fn an_unmasked_frame_closes_with_1002() {
    let (mut client, _sender, mut receiver) = pair(Config::default());
    client
        .send_raw(&encode_frame(true, OP_TEXT, b"hi", None))
        .await;
    assert!(matches!(receiver.next().await, Err(Error::Protocol(_))));
    assert_eq!(client.read_close_code().await, 1002);
}

#[tokio::test]
async fn frames_the_standard_forbids_close_with_1002() {
    let cases: [Vec<u8>; 6] = [
        encode_frame(true, OP_TEXT, b"x", Some(KEY))
            .into_iter()
            .enumerate()
            .map(|(i, b)| if i == 0 { b | 0x40 } else { b })
            .collect(),
        encode_frame(true, 3, b"x", Some(KEY)),
        encode_frame(true, 11, b"x", Some(KEY)),
        encode_frame(true, OP_CONTINUATION, b"x", Some(KEY)),
        encode_frame(false, OP_PING, b"x", Some(KEY)),
        encode_frame(true, OP_PING, &[0u8; 126], Some(KEY)),
    ];
    for wire in cases {
        let (mut client, _sender, mut receiver) = pair(Config::default());
        client.send_raw(&wire).await;
        assert!(
            matches!(receiver.next().await, Err(Error::Protocol(_))),
            "{:02x?}",
            &wire[..2]
        );
        assert_eq!(client.read_close_code().await, 1002);
    }
}

#[tokio::test]
async fn a_new_message_inside_a_fragmented_one_closes_with_1002() {
    let (mut client, _sender, mut receiver) = pair(Config::default());
    client.send(false, OP_TEXT, b"abc").await;
    client.send(true, OP_TEXT, b"def").await;
    loop {
        match receiver.next().await {
            Ok(Event::Data { .. }) => {}
            Err(Error::Protocol(_)) => break,
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(client.read_close_code().await, 1002);
}

#[tokio::test]
async fn a_message_over_the_limit_closes_with_1009() {
    let (mut client, _sender, mut receiver) = pair(Config {
        max_message_size: 10,
        ..Config::default()
    });
    client.send(true, OP_BINARY, &[0u8; 10]).await;
    assert_eq!(message(&mut receiver).await.1.len(), 10);
    client.send(true, OP_BINARY, &[0u8; 11]).await;
    assert!(matches!(receiver.next().await, Err(Error::MessageTooBig)));
    assert_eq!(client.read_close_code().await, 1009);
}

#[tokio::test]
async fn fragments_count_towards_the_limit_together() {
    let (mut client, _sender, mut receiver) = pair(Config {
        max_message_size: 10,
        ..Config::default()
    });
    client.send(false, OP_BINARY, &[0u8; 6]).await;
    client.send(true, OP_CONTINUATION, &[0u8; 6]).await;
    loop {
        match receiver.next().await {
            Ok(Event::Data { .. }) => {}
            Err(Error::MessageTooBig) => break,
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(client.read_close_code().await, 1009);
}

#[tokio::test]
async fn a_frame_that_claims_an_enormous_length_is_refused_before_it_is_read() {
    let (mut client, _sender, mut receiver) = pair(Config {
        max_message_size: 1000,
        ..Config::default()
    });
    let mut wire = vec![0x82, 0xff];
    wire.extend_from_slice(&(1u64 << 40).to_be_bytes());
    wire.extend_from_slice(&KEY);
    client.send_raw(&wire).await;
    assert!(matches!(receiver.next().await, Err(Error::MessageTooBig)));
    assert_eq!(client.read_close_code().await, 1009);
}

#[tokio::test]
async fn the_peers_close_is_answered_with_a_normal_closure() {
    let (mut client, sender, mut receiver) = pair(Config::default());
    let mut payload = 1001u16.to_be_bytes().to_vec();
    payload.extend_from_slice(b"going away");
    client.send(true, OP_CLOSE, &payload).await;
    assert_eq!(
        receiver.next().await.unwrap(),
        Event::Close(CloseFrame {
            code: Some(1001),
            reason: "going away".into()
        })
    );
    assert_eq!(client.read_close_code().await, 1000);
    assert!(matches!(receiver.next().await, Err(Error::Closed)));
    assert!(matches!(
        sender.send_text(b"late").await,
        Err(Error::Closed)
    ));
}

#[tokio::test]
async fn a_close_without_a_status_is_allowed() {
    let (mut client, _sender, mut receiver) = pair(Config::default());
    client.send(true, OP_CLOSE, b"").await;
    assert_eq!(
        receiver.next().await.unwrap(),
        Event::Close(CloseFrame {
            code: None,
            reason: String::new()
        })
    );
    assert_eq!(client.read_close_code().await, 1000);
}

#[tokio::test]
async fn close_frames_with_bad_content_close_with_1002() {
    let cases: [&[u8]; 6] = [
        &[0x03],
        &[0x03, 0xed],       // 1005 may not be sent
        &[0x03, 0xe7],       // 999
        &[0x03, 0xec],       // 1004
        &[0x0b, 0xb7],       // 2999
        &[0x03, 0xe8, 0xff], // reason is not UTF-8
    ];
    for payload in cases {
        let (mut client, _sender, mut receiver) = pair(Config::default());
        client.send(true, OP_CLOSE, payload).await;
        assert!(
            matches!(receiver.next().await, Err(Error::Protocol(_))),
            "{payload:02x?}"
        );
        assert_eq!(client.read_close_code().await, 1002);
    }
}

#[tokio::test]
async fn registered_and_application_close_codes_are_accepted() {
    for code in [
        1000u16, 1001, 1002, 1003, 1007, 1008, 1009, 1010, 1011, 1012, 3000, 4999,
    ] {
        let (mut client, _sender, mut receiver) = pair(Config::default());
        client.send(true, OP_CLOSE, &code.to_be_bytes()).await;
        assert!(
            matches!(receiver.next().await, Ok(Event::Close(_))),
            "{code}"
        );
    }
}

#[tokio::test]
async fn our_close_is_sent_once_and_the_peers_answer_ends_the_connection() {
    let (mut client, sender, mut receiver) = pair(Config::default());
    sender.close(1000, "bye").await.unwrap();
    assert!(sender.is_closing());
    assert!(matches!(
        sender.close(1001, "again").await,
        Err(Error::Closed)
    ));
    let (opcode, _, payload) = client.read().await;
    assert_eq!((opcode, &payload[2..]), (OP_CLOSE, b"bye".as_slice()));
    client.send(true, OP_CLOSE, &1000u16.to_be_bytes()).await;
    assert!(matches!(receiver.next().await, Ok(Event::Close(_))));
    // The answer to our close is not answered again: nothing else arrives.
    drop(sender);
    drop(receiver);
    let mut rest = Vec::new();
    client.io.read_to_end(&mut rest).await.unwrap();
    assert!(rest.is_empty());
}

#[tokio::test]
async fn a_long_close_reason_is_cut_on_a_character_boundary() {
    let (mut client, sender, _receiver) = pair(Config::default());
    let reason = "\u{142}".repeat(100);
    sender.close(1000, &reason).await.unwrap();
    let (_, _, payload) = client.read().await;
    assert!(payload.len() <= 125);
    assert!(std::str::from_utf8(&payload[2..]).is_ok());
}

#[tokio::test]
async fn data_sent_after_a_close_is_refused() {
    let (_client, sender, _receiver) = pair(Config::default());
    sender.close(1000, "").await.unwrap();
    assert!(matches!(sender.send_text(b"x").await, Err(Error::Closed)));
    assert!(matches!(sender.send_pong(b"").await, Err(Error::Closed)));
}

#[tokio::test]
async fn a_dropped_connection_is_an_io_error_not_a_close() {
    let (client, _sender, mut receiver) = pair(Config::default());
    drop(client);
    match receiver.next().await {
        Err(Error::Io(error)) => assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof),
        other => panic!("{other:?}"),
    }
    assert!(matches!(receiver.next().await, Err(Error::Closed)));
}

#[tokio::test]
async fn a_connection_that_stops_in_the_middle_of_a_frame_is_an_io_error() {
    let (mut client, _sender, mut receiver) = pair(Config::default());
    let wire = encode_frame(true, OP_TEXT, b"truncated", Some(KEY));
    client.send_raw(&wire[..wire.len() - 3]).await;
    drop(client);
    // The first part of the payload is handed out before the end is missed.
    loop {
        match receiver.next().await {
            Ok(Event::Data { end, .. }) => assert!(!end),
            Err(Error::Io(_)) => break,
            other => panic!("{other:?}"),
        }
    }
}

#[tokio::test]
async fn frames_written_by_the_server_are_whole_and_unmasked() {
    let (mut client, sender, _receiver) = pair(Config::default());
    sender.send_text(b"one").await.unwrap();
    sender
        .send(MessageKind::Binary, &vec![5u8; 70_000])
        .await
        .unwrap();
    sender.send_ping(b"p").await.unwrap();
    sender.send_pong(b"").await.unwrap();
    assert_eq!(client.read().await, (OP_TEXT, true, b"one".to_vec()));
    assert_eq!(client.read().await, (OP_BINARY, true, vec![5u8; 70_000]));
    assert_eq!(client.read().await, (OP_PING, true, b"p".to_vec()));
    assert_eq!(client.read().await, (OP_PONG, true, vec![]));
    assert!(sender.send_ping(&[0u8; 126]).await.is_err());
}

#[tokio::test]
async fn concurrent_senders_never_interleave_frames() {
    let (mut client, sender, _receiver) = pair(Config::default());
    let mut tasks = Vec::new();
    for n in 0..8u8 {
        let sender = sender.clone();
        tasks.push(tokio::spawn(async move {
            for _ in 0..20 {
                sender
                    .send(MessageKind::Binary, &vec![n; 40_000])
                    .await
                    .unwrap();
            }
        }));
    }
    for _ in 0..160 {
        let (_, _, payload) = tokio::time::timeout(Duration::from_secs(5), client.read())
            .await
            .unwrap();
        assert_eq!(payload.len(), 40_000);
        assert!(payload.iter().all(|b| *b == payload[0]));
    }
    for task in tasks {
        task.await.unwrap();
    }
}
