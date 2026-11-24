//! Uploads and downloads with the files in a bucket instead of the data
//! volume, against the fake bucket server the storage crate tests with.

#[path = "../../../storage/tests/support/fake_s3.rs"]
#[allow(dead_code)]
pub(crate) mod fake_s3;

use http::Method;
use serde_json::Value;

use crate::support::Request;
use crate::uploads::{Form, JPEG, MP4, PDF, PNG, id_of, image, objects, started};
use fake_s3::{ACCESS_KEY, FakeS3, SECRET_KEY};

const BUCKET: &str = "uploads";

async fn with_bucket(fake: &FakeS3, extra: &[(&str, &str)]) -> crate::support::Server {
    let mut variables = vec![
        ("Storage__Provider", "S3"),
        ("Storage__Endpoint", fake.endpoint.as_str()),
        ("Storage__AccessKey", ACCESS_KEY),
        ("Storage__SecretKey", SECRET_KEY),
        ("Storage__Bucket", BUCKET),
    ];
    variables.extend_from_slice(extra);
    started(&variables).await
}

fn key_of(server: &crate::support::Server, id: &str) -> String {
    server
        .database()
        .query_row(
            "SELECT \"ObjectKey\" FROM \"ImageAttachments\" WHERE \"Id\" = ?1",
            [stellarr_api::values::Id::parse(id).unwrap()],
            |row| row.get(0),
        )
        .unwrap()
}

#[tokio::test]
async fn an_upload_lands_in_the_bucket_and_not_on_the_volume() {
    let fake = FakeS3::start().await;
    let server = with_bucket(&fake, &[]).await;
    let maja = server.found("maja").await;

    let response = maja
        .client
        .send(
            Form::new()
                .file("file", "clip.mp4", "video/mp4", MP4)
                .file("poster", "p.jpg", "image/jpeg", JPEG)
                .post("/api/uploads"),
        )
        .await;

    assert_eq!(response.status, 201, "{}", response.text());
    let id = id_of(&response);
    let key = key_of(&server, &id);
    assert_eq!(fake.stored(BUCKET, &key).unwrap().0, MP4);
    let poster = fake
        .stored(BUCKET, key.replace("videos/", "posters/").replace(".mp4", ".jpg").as_str())
        .unwrap();
    assert_eq!(poster.0, JPEG);
    assert!(objects(&server).is_empty(), "nothing is kept on the volume");
    assert_eq!(crate::uploads::staged(&server), 0);
}

#[tokio::test]
async fn a_download_is_streamed_from_the_bucket_with_its_ranges() {
    let fake = FakeS3::start().await;
    let server = with_bucket(&fake, &[]).await;
    let maja = server.found("maja").await;
    let body: Vec<u8> = (0..=255u8).cycle().take(3 * 1024 * 1024).collect();
    let big = [PDF, &body[..]].concat();
    let id = id_of(
        &maja
            .client
            .send(Form::new().file("file", "big.pdf", "application/pdf", &big).post("/api/uploads"))
            .await,
    );
    let path = format!("/assets/{id}");

    let whole = maja.client.get(&path).await;
    let part = maja
        .client
        .send(Request::new(Method::GET, &path).header("Range", "bytes=1000000-1000099"))
        .await;
    let past = maja
        .client
        .send(Request::new(Method::GET, &path).header("Range", &format!("bytes={}-", big.len())))
        .await;

    assert_eq!(whole.status, 200);
    assert_eq!(whole.body.len(), big.len());
    assert_eq!(&whole.body[..], &big[..]);
    assert_eq!(part.status, 206);
    assert_eq!(&part.body[..], &big[1_000_000..1_000_100]);
    assert_eq!(
        part.header("content-range"),
        Some(format!("bytes 1000000-1000099/{}", big.len()).as_str())
    );
    assert_eq!(past.status, 416);
}

#[tokio::test]
async fn an_object_that_is_not_in_the_bucket_is_not_found() {
    let fake = FakeS3::start().await;
    let server = with_bucket(&fake, &[]).await;
    let maja = server.found("maja").await;
    let id = id_of(&maja.client.send(image("a.png").post("/api/uploads")).await);
    fake.fail_next("GET", 404, 1);

    assert_eq!(maja.client.get(&format!("/assets/{id}")).await.status, 404);
}

#[tokio::test]
async fn a_bucket_that_refuses_the_write_fails_the_upload_and_leaves_no_row() {
    let fake = FakeS3::start().await;
    let server = with_bucket(&fake, &[]).await;
    let maja = server.found("maja").await;
    fake.fail_next("PUT", 403, 20);

    let response = maja.client.send(image("a.png").post("/api/uploads")).await;

    assert_eq!(response.status, 500);
    assert!(response.json::<Value>()["title"].is_string());
    let rows: i64 = server
        .database()
        .query_row("SELECT COUNT(*) FROM \"ImageAttachments\"", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
    assert_eq!(crate::uploads::staged(&server), 0);
}

#[tokio::test]
async fn a_picture_that_is_replaced_is_queued_for_removal_from_the_bucket() {
    let fake = FakeS3::start().await;
    let server = with_bucket(&fake, &[]).await;
    let maja = server.found("maja").await;

    maja.client.send(Form::new().file("file", "a.png", "image/png", PNG).post("/api/me/images/avatar")).await;
    maja.client.send(Form::new().file("file", "b.png", "image/png", PNG).post("/api/me/images/avatar")).await;

    let queued: Vec<String> = {
        let db = server.database();
        let mut statement = db.prepare("SELECT \"ObjectKey\" FROM \"ObjectDeletions\"").unwrap();
        statement.query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect()
    };
    assert_eq!(queued.len(), 1);
    assert!(queued[0].starts_with("profiles/"), "{queued:?}");
}

#[tokio::test]
async fn a_server_with_a_bucket_that_cannot_be_named_refuses_to_start() {
    let refused = crate::support::Server::refused_to_start(&[("Storage__Provider", "S3")]).await;

    assert!(refused.contains("Storage:Endpoint"), "{refused}");
}
