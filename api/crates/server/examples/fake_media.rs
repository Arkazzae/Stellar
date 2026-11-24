//! The fake media server of the tests, on an address of its own, for a script
//! to put behind a server and to drive over `/_fake/...`.
//!
//!   fake_media 0.0.0.0:5231
//!
//! It is the control API of the media server as its contract describes it
//! (see `tests/api/support/media.rs`), recording every request it is asked. It
//! is a tool for comparing this server with the first one, not a server to run:
//! anybody who can reach it can say what the media server says.

#[allow(dead_code)]
#[path = "../tests/api/support/media.rs"]
mod media;

#[tokio::main]
async fn main() {
    let address = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:5231".to_string())
        .parse()
        .expect("an address such as 0.0.0.0:5231");
    let fake = media::FakeMedia::start_on(address).await;
    println!("fake media server on {}", fake.address());
    std::future::pending::<()>().await;
}
