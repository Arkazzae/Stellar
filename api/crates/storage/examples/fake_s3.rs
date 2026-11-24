//! The fake bucket server of the tests, as a process of its own, so that a test
//! which measures its memory does not count the server's. Prints
//! `LISTENING <endpoint>` and serves until it is killed.

#[allow(dead_code)]
#[path = "../tests/support/fake_s3.rs"]
mod fake_s3;

#[tokio::main]
async fn main() {
    let fake = fake_s3::FakeS3::start().await;
    println!("LISTENING {}", fake.endpoint);
    std::future::pending::<()>().await;
}
