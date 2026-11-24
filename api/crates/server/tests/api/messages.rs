//! Posting, rewriting and removing messages, and reading a channel back.

use serde_json::{Value, json};

use crate::support::{Person, Server};

struct World {
    server: Server,
    owner: Person,
    general: String,
}

async fn world() -> World {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let general: String = server
        .database()
        .query_row(
            "SELECT \"Id\" FROM \"Channels\" WHERE \"Kind\" = 0 ORDER BY \"Position\" LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    World {
        server,
        owner,
        general: general.to_lowercase(),
    }
}

#[tokio::test]
async fn smoke_posting_and_reading() {
    let w = world().await;

    let sent = w
        .owner
        .client
        .post_json(&format!("/api/channels/{}/messages", w.general), &json!({"content": "hello @owner"}))
        .await;
    assert_eq!(sent.status, 201);
    let history = w.owner.client.get(&format!("/api/channels/{}/messages", w.general)).await;
    let _: Value = history.json();
    let _ = &w.server;
}

// ---- The cursor against the first server's own code

mod cursor_oracle {
    //! The history cursor, read and written by the first server's code and by
    //! this one on the same texts. Not part of the ordinary run: it needs a
    //! .NET SDK and a small program, kept outside the repository, that reads
    //! one text per line on its standard input and answers `no`, `throw ..` or
    //! `ok <scope> <snapshot ticks> <before ticks> <before id> <encoded>` per
    //! line.
    //!
    //! ```sh
    //! MESSAGES_HARNESS=/path/to/Harness.dll DOTNET=~/.dotnet/dotnet \
    //!   cargo test -p stellarr-api --test api cursor_oracle -- --ignored --nocapture
    //! ```

    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    use stellarr_api::messages::cursor::{HistoryCursor, Reading};

    struct Random(u64);

    impl Random {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    const STRANGE: &[&str] = &["=", "==", "===", " ", "\t", "\n", "\r", "+", "/", "\u{e9}", "\u{a0}", "\0", "A", "Q", "g", "-", "_"];

    fn encode(bytes: &[u8]) -> String {
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let mut word = 0u32;
            for (index, byte) in chunk.iter().enumerate() {
                word |= u32::from(*byte) << (16 - 8 * index);
            }
            for index in 0..chunk.len() + 1 {
                out.push(char::from(ALPHABET[((word >> (18 - 6 * index)) & 63) as usize]));
            }
        }
        out
    }

    fn random_cursor(random: &mut Random) -> Vec<u8> {
        let mut bytes: Vec<u8> = Vec::new();
        let legacy = random.below(4) == 0;
        let ticks = |random: &mut Random| -> i64 {
            match random.below(6) {
                0 => 0,
                1 => 3_155_378_975_999_999_999,
                2 => 3_155_378_976_000_000_000,
                3 => -1,
                4 => random.next() as i64,
                _ => 638_000_000_000_000_000 + (random.next() % 10_000_000_000_000_000) as i64,
            }
        };
        if legacy {
            bytes.extend((0..16).map(|_| random.next() as u8));
            bytes.extend(ticks(random).to_be_bytes());
            let offset = match random.below(4) {
                0 => -1i32,
                1 => i32::MIN,
                _ => random.next() as i32 & 0x7fff,
            };
            bytes.extend(offset.to_be_bytes());
        } else {
            bytes.push(if random.below(8) == 0 { random.next() as u8 } else { 1 });
            bytes.extend((0..16).map(|_| random.next() as u8));
            let snapshot = ticks(random);
            bytes.extend(snapshot.to_be_bytes());
            let before = if random.below(3) == 0 { ticks(random) } else { snapshot - random.below(1000) as i64 };
            bytes.extend(before.to_be_bytes());
            bytes.extend((0..16).map(|_| random.next() as u8));
        }
        bytes
    }

    fn mutate(random: &mut Random, text: &str) -> String {
        let mut chars: Vec<String> = text.chars().map(String::from).collect();
        for _ in 0..random.below(4) {
            let at = random.below(chars.len() + 1);
            match random.below(4) {
                0 if at < chars.len() => {
                    chars.remove(at);
                }
                1 => chars.insert(at, STRANGE[random.below(STRANGE.len())].to_string()),
                2 if at < chars.len() => chars[at] = STRANGE[random.below(STRANGE.len())].to_string(),
                _ => chars.truncate(at),
            }
        }
        chars.concat()
    }

    #[test]
    #[ignore = "needs the first server's harness"]
    fn the_cursor_is_read_and_written_as_the_first_server_does() {
        let dll = std::env::var("MESSAGES_HARNESS").expect("MESSAGES_HARNESS: the harness assembly");
        let dotnet = std::env::var("DOTNET").unwrap_or_else(|_| "dotnet".to_string());
        let mut random = Random(0x9E37_79B9_7F4A_7C15);
        let mut cases: Vec<String> = vec![String::new(), "A".into()];
        for _ in 0..40_000 {
            let text = encode(&random_cursor(&mut random));
            cases.push(text.clone());
            cases.push(mutate(&mut random, &text));
        }
        // A line holds one case: none of them may contain a line break.
        let cases: Vec<String> = cases.into_iter().filter(|text| !text.contains(['\n', '\r'])).collect();

        let mut child = Command::new(dotnet)
            .arg(dll)
            .arg("cursor")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("the harness starts");
        let mut input = child.stdin.take().unwrap();
        let feed = cases.clone();
        let writer = std::thread::spawn(move || {
            for case in feed {
                writeln!(input, "{case}").unwrap();
            }
        });
        let answers: Vec<String> = BufReader::new(child.stdout.take().unwrap()).lines().map(Result::unwrap).collect();
        writer.join().unwrap();
        assert_eq!(answers.len(), cases.len());

        let mut accepted = 0;
        for (case, answer) in cases.iter().zip(&answers) {
            let ours = match HistoryCursor::decode(case) {
                Reading::Invalid => "no".to_string(),
                Reading::Fault => "throw FormatException".to_string(),
                Reading::Cursor(cursor) => {
                    accepted += 1;
                    let epoch = 621_355_968_000_000_000i64;
                    let (before_ticks, before_id, encoded) = match cursor.before {
                        Some((at, id)) => (
                            (at.ticks() + epoch).to_string(),
                            id.to_string(),
                            cursor.encode().unwrap(),
                        ),
                        None => ("-".to_string(), "-".to_string(), "-".to_string()),
                    };
                    format!(
                        "ok {} {} {} {} {}",
                        cursor.scope,
                        cursor.snapshot.ticks() + epoch,
                        before_ticks,
                        before_id,
                        encoded
                    )
                }
            };
            assert_eq!(&ours, answer, "{case:?}");
        }
        println!("{} cases, {accepted} read as cursors", cases.len());
        assert!(accepted > 10_000);
    }
}
