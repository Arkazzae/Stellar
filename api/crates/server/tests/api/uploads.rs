//! Uploading files: what is accepted and stored, what is refused and with
//! what, the limits and the quotas, and what a form that is broken does.
//!
//! The wire is compared with the first server's by `tests/uploads-probe.py`;
//! these tests keep the rules in place.

use std::path::Path;

use http::Method;
use rusqlite::{Connection, params};
use serde_json::Value;
use stellarr_api::values::{Id, Timestamp};

use crate::support::{Person, Request, Response, Server};

pub const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];
pub const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0];
pub const WEBP: &[u8] = b"RIFF\0\0\0\0WEBP\0\0\0\0";
pub const WAVE: &[u8] = b"RIFF\0\0\0\0WAVE\0\0\0\0";
pub const MP4: &[u8] = &[0, 0, 0, 16, b'f', b't', b'y', b'p', 1, 2, 3, 4];
pub const PDF: &[u8] = b"%PDF-1.7\n%%EOF\n";

/// A form being written, part by part.
pub struct Form {
    boundary: String,
    body: Vec<u8>,
}

impl Form {
    pub fn new() -> Form {
        Form {
            boundary: "XXB".to_string(),
            body: Vec::new(),
        }
    }

    pub fn file(self, field: &str, name: &str, content_type: &str, bytes: &[u8]) -> Form {
        let head = format!(
            "Content-Disposition: form-data; name=\"{field}\"; filename=\"{name}\"\r\nContent-Type: {content_type}"
        );
        self.part(head.as_bytes(), bytes)
    }

    pub fn text(self, field: &str, value: &str) -> Form {
        let head = format!("Content-Disposition: form-data; name=\"{field}\"");
        self.part(head.as_bytes(), value.as_bytes())
    }

    /// A part with the header lines given, whatever they say.
    pub fn part(mut self, head: &[u8], bytes: &[u8]) -> Form {
        self.body.extend_from_slice(b"--");
        self.body.extend_from_slice(self.boundary.as_bytes());
        self.body.extend_from_slice(b"\r\n");
        self.body.extend_from_slice(head);
        self.body.extend_from_slice(b"\r\n\r\n");
        self.body.extend_from_slice(bytes);
        self.body.extend_from_slice(b"\r\n");
        self
    }

    pub fn boundary(mut self, boundary: &str) -> Form {
        self.boundary = boundary.to_string();
        self
    }

    /// The closing boundary, and the request that carries the form.
    pub fn post(self, path: &str) -> Request {
        let content_type = format!("multipart/form-data; boundary={}", self.boundary);
        let mut body = self.body;
        body.extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        Request::new(Method::POST, path).bytes(&content_type, body)
    }

    /// The request, without the closing boundary.
    pub fn post_unfinished(self, path: &str) -> Request {
        let content_type = format!("multipart/form-data; boundary={}", self.boundary);
        Request::new(Method::POST, path).bytes(&content_type, self.body)
    }
}

/// A server that wants no free disk space kept back, so that the disk of the
/// machine that runs the tests does not decide anything.
pub async fn started(extra: &[(&str, &str)]) -> Server {
    let mut variables = vec![("Uploads__MinimumFreeDataRootBytes", "0")];
    variables.extend_from_slice(extra);
    Server::start_with(&variables).await
}

pub async fn upload(person: &Person, form: Form) -> Response {
    person.client.send(form.post("/api/uploads")).await
}

pub fn image(name: &str) -> Form {
    Form::new().file("file", name, "image/png", PNG)
}

/// Every object in the store, by key.
pub fn objects(server: &Server) -> Vec<String> {
    fn walk(base: &Path, folder: &Path, found: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(folder) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, found);
            } else if let Ok(relative) = path.strip_prefix(base) {
                found.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let base = server.folder().join("files");
    let mut found = Vec::new();
    walk(&base, &base, &mut found);
    found.retain(|key| !key.starts_with(".incoming"));
    found.sort();
    found
}

/// What is left in the staging folder.
pub fn staged(server: &Server) -> usize {
    std::fs::read_dir(server.folder().join("incoming")).map_or(0, Iterator::count)
}

pub fn id_of(response: &Response) -> String {
    response.json::<Value>()["id"].as_str().unwrap().to_string()
}

pub fn first_server(db: &Connection) -> Id {
    db.query_row(
        "SELECT \"ServerId\" FROM \"ServerMembers\" WHERE \"OwnerSlot\" = 1 LIMIT 1",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

/// A text channel of the server, the first one it has.
pub fn channel_of(db: &Connection, server: Id) -> Id {
    db.query_row(
        "SELECT \"Id\" FROM \"Channels\" WHERE \"ServerId\" = ?1 AND \"Kind\" = 0 ORDER BY \"Position\" LIMIT 1",
        [server],
        |row| row.get(0),
    )
    .unwrap()
}

/// Posts a message carrying the attachment, as the person.
pub fn post_message(db: &Connection, channel: Id, author: &str, attachment: &str) -> Id {
    let message = Id::random();
    db.execute(
        "INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\") VALUES (?1, ?2, ?3, 0, 'file', ?4)",
        params![message, channel, Id::parse(author).unwrap(), Timestamp::now()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO \"MessageAttachments\" (\"MessageId\", \"AttachmentId\") VALUES (?1, ?2)",
        params![message, Id::parse(attachment).unwrap()],
    )
    .unwrap();
    message
}

#[tokio::test]
async fn a_png_is_answered_with_its_asset_address_and_stored_as_an_image() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    let response = upload(&maja, image("tiny.png")).await;

    assert_eq!(response.status, 201, "{}", response.text());
    let attachment: Value = response.json();
    let id = attachment["id"].as_str().unwrap();
    assert_eq!(response.header("location"), Some(format!("/assets/{id}").as_str()));
    assert_eq!(attachment["url"], format!("/assets/{id}"));
    assert_eq!(attachment["name"], "tiny.png");
    assert_eq!(attachment["contentType"], "image/png");
    assert_eq!(attachment["size"], PNG.len());
    assert_eq!(attachment["isVoiceMessage"], false);
    assert_eq!(attachment["posterUrl"], Value::Null);
    let text = response.text();
    let places: Vec<usize> = ["id", "name", "contentType", "size", "width", "height", "url", "durationSeconds", "posterUrl", "isVoiceMessage", "previewUrl", "placeholder"]
        .iter()
        .map(|member| text.find(&format!("\"{member}\":")).unwrap())
        .collect();
    assert!(places.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
    let key: String = server
        .database()
        .query_row(
            "SELECT \"ObjectKey\" FROM \"ImageAttachments\" WHERE \"Id\" = ?1",
            [Id::parse(id).unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    let compact = id.replace('-', "");
    let pattern = key.strip_prefix("images/").and_then(|rest| rest.split_once('/')).unwrap();
    assert_eq!(pattern.0.len(), 4, "{key}");
    assert!(pattern.1.ends_with(&format!("/{compact}.png")), "{key}");
    assert_eq!(std::fs::read(server.folder().join("files").join(&key)).unwrap(), PNG);
    assert_eq!(staged(&server), 0);
}

#[tokio::test]
async fn the_older_image_address_still_accepts_uploads() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    let response = maja.client.send(image("legacy.png").post("/api/uploads/images")).await;

    assert_eq!(response.status, 201);
}

#[tokio::test]
async fn uploads_need_a_signed_in_person() {
    let server = started(&[]).await;
    server.found("maja").await;

    for path in ["/api/uploads", "/api/uploads/images", "/api/uploads/encrypted"] {
        let response = server.client().send(image("tiny.png").post(path)).await;
        assert_eq!(response.status, 401, "{path}");
    }
}

#[tokio::test]
async fn each_kind_is_stored_under_its_own_folder() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    for (content_type, bytes, folder, extension) in [
        ("audio/wav", WAVE, "audio/", ".wav"),
        ("video/mp4", MP4, "videos/", ".mp4"),
        ("application/pdf", PDF, "files/", ".pdf"),
        ("image/webp", WEBP, "images/", ".webp"),
    ] {
        let response = upload(&maja, Form::new().file("file", "sample", content_type, bytes)).await;

        assert_eq!(response.status, 201, "{content_type}");
        let key = objects(&server).into_iter().find(|key| key.starts_with(folder)).expect(folder);
        assert!(key.ends_with(extension), "{key}");
    }
}

#[tokio::test]
async fn every_type_is_stored_when_its_bytes_fit_and_refused_when_they_do_not() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let zip: &[u8] = b"PK\x03\x04....";
    let samples: [(&str, &[u8]); 21] = [
        ("image/jpeg", JPEG),
        ("image/png", PNG),
        ("image/gif", b"GIF89a...."),
        ("image/webp", WEBP),
        ("audio/mpeg", b"ID3\x04...."),
        ("audio/ogg", b"OggS...."),
        ("audio/wav", WAVE),
        ("audio/x-wav", WAVE),
        ("audio/webm", b"\x1a\x45\xdf\xa3...."),
        ("audio/mp4", MP4),
        ("audio/x-m4a", MP4),
        ("audio/aac", b"\xff\xf1...."),
        ("video/mp4", MP4),
        ("video/webm", b"\x1a\x45\xdf\xa3...."),
        ("video/ogg", b"OggS...."),
        ("video/quicktime", MP4),
        ("application/pdf", PDF),
        ("application/zip", zip),
        ("text/plain", b"hello"),
        ("application/json", b" {\"a\":1}"),
        ("application/vnd.openxmlformats-officedocument.wordprocessingml.document", zip),
    ];
    for (content_type, bytes) in samples {
        let fits = upload(&maja, Form::new().file("file", "x", content_type, bytes)).await;
        assert_eq!(fits.status, 201, "{content_type}: {}", fits.text());
        let before = objects(&server).len();

        let forged = upload(&maja, Form::new().file("file", "x", content_type, b"\x00\x01\x02\x03wrong\x00")).await;

        assert_eq!(forged.status, 415, "{content_type}");
        assert_eq!(forged.json::<Value>()["title"], "Invalid attachment format");
        assert_eq!(forged.json::<Value>()["detail"], "The content of the file does not match its declared type.");
        assert_eq!(objects(&server).len(), before, "{content_type} stored something");
    }
    assert_eq!(staged(&server), 0);
}

#[tokio::test]
async fn the_download_name_keeps_the_stem_and_takes_its_extension_from_the_verified_type() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    for (submitted, expected) in [
        ("release notes.exe", "release notes.pdf"),
        ("my:file*name?.png", "myfilename.pdf"),
        ("../../etc/passwd.png", "passwd.pdf"),
        ("////", "attachment.pdf"),
        ("   ...   .png", "attachment.pdf"),
        ("zażółć gęślą.png", "zażółć gęślą.pdf"),
        ("x\0y.png", "xy.pdf"),
    ] {
        let response = upload(&maja, Form::new().file("file", submitted, "application/pdf", PDF)).await;

        assert_eq!(response.json::<Value>()["name"], expected, "{submitted:?}");
    }
}

#[tokio::test]
async fn an_unsupported_type_is_refused_and_nothing_is_stored() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    for content_type in ["text/html", "image/svg+xml", "application/x-msdownload", ""] {
        let form = if content_type.is_empty() {
            Form::new().part(b"Content-Disposition: form-data; name=\"file\"; filename=\"page\"", b"<html>")
        } else {
            Form::new().file("file", "page", content_type, b"<html>")
        };

        let response = upload(&maja, form).await;

        assert_eq!(response.status, 415, "{content_type}");
        assert_eq!(response.json::<Value>()["title"], "Unsupported attachment type");
    }
    assert!(objects(&server).is_empty());
}

#[tokio::test]
async fn an_empty_file_is_a_validation_problem_on_the_file() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    let response = upload(&maja, Form::new().file("file", "empty.png", "image/png", b"")).await;

    assert_eq!(response.status, 400);
    assert!(response.json::<Value>()["errors"]["file"].is_array());
}

#[tokio::test]
async fn a_form_without_the_file_is_a_bad_request_with_no_body() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    for form in [Form::new().text("x", "1"), Form::new().text("file", "not a file")] {
        let response = upload(&maja, form).await;

        assert_eq!(response.status, 400);
        assert!(response.body.is_empty());
    }
}

#[tokio::test]
async fn a_file_over_the_limit_of_its_kind_is_refused_and_one_at_the_limit_is_not() {
    let server = started(&[("Uploads__MaxImageBytes", "12")]).await;
    let maja = server.found("maja").await;
    let longer = [PNG, &[0]].concat();

    let at = upload(&maja, image("fits.png")).await;
    let over = upload(&maja, Form::new().file("file", "large.png", "image/png", &longer)).await;
    let other = upload(&maja, Form::new().file("file", "other.pdf", "application/pdf", PDF)).await;

    assert_eq!(at.status, 201);
    assert_eq!(over.status, 413);
    assert_eq!(over.json::<Value>()["title"], "The attachment is too large");
    assert_eq!(over.json::<Value>()["detail"], "The maximum image size is 10 MB.");
    assert_eq!(other.status, 201);
    assert_eq!(objects(&server).len(), 2);
    assert_eq!(staged(&server), 0);
}

#[tokio::test]
async fn the_first_file_of_a_name_counts_and_the_others_are_ignored() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    let response = upload(
        &maja,
        Form::new()
            .file("file", "first.png", "image/png", PNG)
            .file("file", "second.pdf", "application/pdf", PDF),
    )
    .await;

    assert_eq!(response.json::<Value>()["name"], "first.png");
    assert_eq!(objects(&server).len(), 1);
}

#[tokio::test]
async fn a_video_may_carry_a_poster_that_is_served_from_its_own_address() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let form = Form::new()
        .file("file", "clip.mp4", "video/mp4", MP4)
        .text("durationSeconds", "42.1254")
        .text("width", "1920")
        .text("height", "1080")
        .file("poster", "poster.jpg", "image/jpeg", JPEG);

    let response = upload(&maja, form).await;

    assert_eq!(response.status, 201, "{}", response.text());
    let attachment: Value = response.json();
    let id = attachment["id"].as_str().unwrap();
    assert_eq!(attachment["posterUrl"], format!("/assets/{id}/poster"));
    assert_eq!(attachment["durationSeconds"], 42.125);
    assert_eq!(attachment["width"], 1920);
    assert_eq!(attachment["height"], 1080);
    let poster = maja.client.get(&format!("/assets/{id}/poster")).await;
    assert_eq!(poster.status, 200);
    assert_eq!(poster.header("content-type"), Some("image/jpeg"));
    assert_eq!(&poster.body[..], JPEG);
    let keys = objects(&server);
    assert!(keys.iter().any(|key| key.starts_with("posters/") && key.ends_with(".jpg")), "{keys:?}");
}

#[tokio::test]
async fn a_whole_number_of_seconds_is_written_without_a_point() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let form = Form::new().file("file", "a.wav", "audio/wav", WAVE).text("durationSeconds", "5");

    let response = upload(&maja, form).await;

    assert!(response.text().contains("\"durationSeconds\":5,"), "{}", response.text());
}

#[tokio::test]
async fn a_poster_is_refused_for_every_kind_but_video_and_when_it_is_not_a_small_image() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let large = [JPEG, &vec![b'x'; 2 * 1024 * 1024][..]].concat();
    let at_the_limit = [JPEG, &vec![b'x'; 2 * 1024 * 1024 - JPEG.len()][..]].concat();
    let video = || Form::new().file("file", "c.mp4", "video/mp4", MP4);

    let on_an_image = upload(&maja, image("a.png").file("poster", "p.jpg", "image/jpeg", JPEG)).await;
    let forged = upload(&maja, video().file("poster", "p.jpg", "image/jpeg", b"nope")).await;
    let empty = upload(&maja, video().file("poster", "p.jpg", "image/jpeg", b"")).await;
    let too_large = upload(&maja, video().file("poster", "p.jpg", "image/jpeg", &large)).await;
    let a_pdf = upload(&maja, video().file("poster", "p.pdf", "application/pdf", PDF)).await;
    let at_limit = upload(&maja, video().file("poster", "p.jpg", "image/jpeg", &at_the_limit)).await;

    for refused in [&on_an_image, &forged, &empty, &too_large, &a_pdf] {
        assert_eq!(refused.status, 400);
        assert!(refused.json::<Value>()["errors"]["poster"].is_array(), "{}", refused.text());
    }
    assert_eq!(at_limit.status, 201);
    assert_eq!(objects(&server).len(), 2, "only the video and its poster");
}

#[tokio::test]
async fn a_recorded_voice_message_is_audio_with_a_measured_duration_up_to_ten_minutes() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let voice = |duration: Option<&str>, flag: &str| {
        let form = Form::new().file("file", "v.wav", "audio/wav", WAVE).text("voiceMessage", flag);
        match duration {
            Some(duration) => form.text("durationSeconds", duration),
            None => form,
        }
    };

    let good = upload(&maja, voice(Some("5"), "true")).await;
    let at_limit = upload(&maja, voice(Some("600"), "TRUE")).await;
    let no_duration = upload(&maja, voice(None, "true")).await;
    let too_long = upload(&maja, voice(Some("601"), "true")).await;
    let not_a_length = upload(&maja, voice(Some("NaN"), "true")).await;
    let on_an_image = upload(&maja, image("a.png").text("durationSeconds", "5").text("voiceMessage", "true")).await;
    let not_a_flag = upload(&maja, voice(Some("5"), "yes")).await;

    assert_eq!(good.status, 201);
    assert_eq!(good.json::<Value>()["isVoiceMessage"], true);
    assert_eq!(at_limit.status, 201);
    for refused in [&no_duration, &too_long, &not_a_length, &on_an_image] {
        assert_eq!(refused.status, 400);
        assert_eq!(
            refused.json::<Value>()["errors"]["voiceMessage"][0],
            "A voice message must be an audio recording of up to 10 minutes."
        );
    }
    // A value that is no flag at all is not a form this server can read.
    assert_eq!(not_a_flag.status, 400);
    assert!(not_a_flag.body.is_empty());
}

#[tokio::test]
async fn a_duration_is_refused_for_kinds_without_one_and_when_it_is_not_a_positive_length() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let audio = |duration: &str| Form::new().file("file", "a.wav", "audio/wav", WAVE).text("durationSeconds", duration);

    let on_an_image = upload(&maja, image("a.png").text("durationSeconds", "5")).await;
    let on_a_pdf = upload(&maja, Form::new().file("file", "a.pdf", "application/pdf", PDF).text("durationSeconds", "5")).await;
    assert_eq!((on_an_image.status.as_u16(), on_a_pdf.status.as_u16()), (400, 400));
    for refused in ["0", "-1", "Infinity", "1e999", "2592001"] {
        let response = upload(&maja, audio(refused)).await;

        assert_eq!(response.status, 400, "{refused}");
        assert_eq!(
            response.json::<Value>()["errors"]["durationSeconds"][0],
            "The duration of the file is invalid.",
            "{refused}"
        );
    }
    for read in ["1,2", " 3 ", "2592000", "0.0004"] {
        assert_eq!(upload(&maja, audio(read)).await.status, 201, "{read}");
    }
    for unreadable in ["abc", "", "1 2"] {
        let response = upload(&maja, audio(unreadable)).await;
        assert_eq!((response.status.as_u16(), response.body.is_empty()), (400, true), "{unreadable}");
    }
}

#[tokio::test]
async fn dimensions_come_in_pairs_within_bounds_and_only_for_images_and_video() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let sized = |width: &str, height: &str| image("a.png").text("width", width).text("height", height);

    for refused in [sized("0", "5"), sized("5", "40000"), sized("-1", "-1"), image("a.png").text("width", "5")] {
        let response = upload(&maja, refused).await;

        assert_eq!(response.status, 400);
        assert_eq!(response.json::<Value>()["errors"]["dimensions"][0], "The dimensions of the file are invalid.");
    }
    let pdf = upload(&maja, Form::new().file("file", "a.pdf", "application/pdf", PDF).text("width", "5").text("height", "5")).await;
    let audio = upload(&maja, Form::new().file("file", "a.wav", "audio/wav", WAVE).text("width", "5").text("height", "5")).await;
    assert_eq!((pdf.status.as_u16(), audio.status.as_u16()), (400, 400));
    let at_the_bound = upload(&maja, sized("32768", "32768")).await;
    assert_eq!(at_the_bound.status, 201);
    assert_eq!(at_the_bound.json::<Value>()["width"], 32768);
    assert_eq!(upload(&maja, sized("x", "1")).await.status, 400);
}

#[tokio::test]
async fn an_image_may_carry_a_preview_and_a_placeholder() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let with = |preview_type: &str, bytes: &[u8]| {
        image("a.png")
            .file("preview", "p", preview_type, bytes)
            .text("placeholder", "AAAA")
    };

    let webp = upload(&maja, with("image/webp", WEBP)).await;
    let jpeg = upload(&maja, with("image/jpeg", JPEG)).await;

    assert_eq!(webp.status, 201, "{}", webp.text());
    let attachment: Value = webp.json();
    let id = attachment["id"].as_str().unwrap();
    assert_eq!(attachment["previewUrl"], format!("/assets/{id}/preview"));
    assert_eq!(attachment["placeholder"], "AAAA");
    assert_eq!(jpeg.status, 201);
    let preview = maja.client.get(&format!("/assets/{id}/preview")).await;
    assert_eq!(preview.header("content-type"), Some("image/webp"));
    assert_eq!(&preview.body[..], WEBP);
    assert!(objects(&server).iter().any(|key| key.starts_with("previews/") && key.ends_with(".webp")));
}

#[tokio::test]
async fn a_preview_that_is_not_a_good_small_webp_or_jpeg_is_refused_and_nothing_is_stored() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let large = [JPEG, &vec![b'x'; 1024 * 1024][..]].concat();
    let at_the_limit = [JPEG, &vec![b'x'; 1024 * 1024 - JPEG.len()][..]].concat();

    for (content_type, bytes) in [("image/png", PNG), ("image/jpeg", b"nope".as_slice()), ("image/jpeg", &large[..]), ("application/pdf", PDF)] {
        let response = upload(&maja, image("a.png").file("preview", "p", content_type, bytes)).await;

        assert_eq!(response.status, 400, "{content_type}");
        assert!(response.json::<Value>()["errors"]["preview"].is_array());
    }
    let on_video = upload(&maja, Form::new().file("file", "c.mp4", "video/mp4", MP4).file("preview", "p", "image/jpeg", JPEG)).await;
    assert_eq!(on_video.status, 400);
    assert!(objects(&server).is_empty());
    let at = upload(&maja, image("a.png").file("preview", "p", "image/jpeg", &at_the_limit)).await;
    assert_eq!(at.status, 201);
}

#[tokio::test]
async fn a_placeholder_is_padded_base64_of_one_to_thirty_two_bytes_and_for_images_only() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let thirty_two = format!("{}A=", "A".repeat(42));

    for good in ["AAAA", "AA==", thirty_two.as_str()] {
        let response = upload(&maja, image("a.png").text("placeholder", good)).await;

        assert_eq!(response.status, 201, "{good}");
        assert_eq!(response.json::<Value>()["placeholder"], good);
    }
    for bad in ["", "AAA", "A===", "AAAA AAAA", &"A".repeat(44), &"A".repeat(48), "????"] {
        let response = upload(&maja, image("a.png").text("placeholder", bad)).await;

        assert_eq!(response.status, 400, "{bad}");
        assert!(response.json::<Value>()["errors"]["placeholder"].is_array());
    }
    let on_a_pdf = upload(&maja, Form::new().file("file", "a.pdf", "application/pdf", PDF).text("placeholder", "AAAA")).await;
    assert_eq!(on_a_pdf.status, 400);
}

#[tokio::test]
async fn an_account_over_its_quota_is_refused_with_507_before_anything_reaches_the_store() {
    let server = started(&[("Uploads__MaxStoredBytesPerUser", "20")]).await;
    let maja = server.found("maja").await;

    let first = upload(&maja, image("a.png")).await;
    let second = upload(&maja, image("b.png")).await;

    assert_eq!(first.status, 201);
    assert_eq!(second.status, 507);
    let problem: Value = second.json();
    assert_eq!(problem["title"], "No room for the attachment");
    assert_eq!(problem["detail"], "The storage limit of this account has been used up.");
    assert_eq!(objects(&server).len(), 1);
}

#[tokio::test]
async fn the_quotas_count_posters_previews_and_other_peoples_files_as_the_instance_quota() {
    let server = started(&[("Uploads__MaxStoredBytes", "30")]).await;
    let maja = server.found("maja").await;
    let sam = server.register("sam").await;

    // 12 + 8 bytes of the image and its preview.
    let first = upload(&maja, image("a.png").file("preview", "p", "image/jpeg", JPEG)).await;
    let by_sam = upload(&sam, image("b.png")).await;
    let small = upload(&sam, Form::new().file("file", "c.txt", "text/plain", b"hello")).await;

    assert_eq!(first.status, 201);
    assert_eq!(by_sam.status, 507, "{}", by_sam.text());
    assert_eq!(by_sam.json::<Value>()["detail"], "The instance has used up the configured storage limit for attachments.");
    assert_eq!(small.status, 201);
}

#[tokio::test]
async fn a_quota_of_zero_means_no_quota() {
    let server = started(&[("Uploads__MaxStoredBytesPerUser", "0"), ("Uploads__MaxStoredBytes", "-1")]).await;
    let maja = server.found("maja").await;

    for _ in 0..3 {
        assert_eq!(upload(&maja, image("a.png")).await.status, 201);
    }
}

#[tokio::test]
async fn an_upload_that_would_leave_the_data_volume_below_its_reserve_is_refused() {
    let server = Server::start_with(&[("Uploads__MinimumFreeDataRootBytes", "9000000000000000")]).await;
    let maja = server.found("maja").await;

    let response = upload(&maja, image("a.png")).await;

    assert_eq!(response.status, 507, "{}", response.text());
    assert_eq!(
        response.json::<Value>()["detail"],
        "The server keeps a reserve of free space for the database and the logs."
    );
    assert!(objects(&server).is_empty());
}

#[tokio::test]
async fn stored_emoji_count_towards_the_quota_of_whoever_uploaded_them() {
    let server = started(&[("Uploads__MaxStoredBytesPerUser", "30")]).await;
    let maja = server.found("maja").await;
    let db = server.database();
    db.execute(
        "INSERT INTO \"ServerEmoji\" (\"Id\", \"ServerId\", \"Shortcode\", \"ObjectKey\", \"ContentType\", \"SizeBytes\", \"UploadedById\", \"CreatedAt\")
         VALUES (?1, ?2, 'party', 'emoji/x.png', 'image/png', 25, ?3, ?4)",
        params![Id::random(), first_server(&db), Id::parse(&maja.id).unwrap(), Timestamp::now()],
    )
    .unwrap();

    let response = upload(&maja, image("a.png")).await;

    assert_eq!(response.status, 507);
}

#[tokio::test]
async fn a_write_that_fails_takes_what_was_already_stored_out_again() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    // A file where the folder of the posters should be: the video is stored,
    // the poster cannot be.
    std::fs::write(server.folder().join("files").join("posters"), b"in the way").unwrap();
    let form = Form::new()
        .file("file", "c.mp4", "video/mp4", MP4)
        .file("poster", "p.jpg", "image/jpeg", JPEG);

    let response = upload(&maja, form).await;

    assert_eq!(response.status, 500);
    assert_eq!(objects(&server), ["posters"], "only the planted file is left");
    let rows: i64 = server
        .database()
        .query_row("SELECT COUNT(*) FROM \"ImageAttachments\"", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
    assert_eq!(staged(&server), 0);
}

#[tokio::test]
async fn an_encrypted_file_is_stored_as_opaque_bytes_whatever_it_says_it_is() {
    let server = started(&[("Uploads__MaxImageBytes", "10")]).await;
    let maja = server.found("maja").await;

    let response = maja
        .client
        .send(Form::new().file("file", "whatever.html", "text/html", b"\x01\x02ciphertext").post("/api/uploads/encrypted"))
        .await;

    assert_eq!(response.status, 201, "{}", response.text());
    let attachment: Value = response.json();
    assert_eq!(attachment["name"], "encrypted.bin");
    assert_eq!(attachment["contentType"], "application/octet-stream");
    assert_eq!(attachment["size"], 12);
    let key = objects(&server).remove(0);
    assert!(key.starts_with("encrypted/") && key.ends_with(".bin"), "{key}");
    let encrypted: bool = server
        .database()
        .query_row("SELECT \"IsEncrypted\" FROM \"ImageAttachments\"", [], |row| row.get(0))
        .unwrap();
    assert!(encrypted);
}

#[tokio::test]
async fn an_encrypted_file_may_be_as_large_as_the_largest_upload_plus_the_tag() {
    let server = started(&[
        ("Uploads__MaxImageBytes", "10"),
        ("Uploads__MaxAudioBytes", "10"),
        ("Uploads__MaxVideoBytes", "100"),
        ("Uploads__MaxFileBytes", "10"),
    ])
    .await;
    let maja = server.found("maja").await;
    let send = |size: usize| {
        let client = &maja.client;
        let form = Form::new().file("file", "x", "application/octet-stream", &vec![7u8; size]);
        async move { client.send(form.post("/api/uploads/encrypted")).await }
    };

    assert_eq!(send(116).await.status, 201);
    let over = send(117).await;
    assert_eq!(over.status, 413);
    assert_eq!(over.json::<Value>()["detail"], "The attachment exceeds the limit.");
    let empty = send(0).await;
    assert_eq!(empty.status, 400);
    assert!(empty.json::<Value>()["errors"]["file"].is_array());
}

#[tokio::test]
async fn a_form_that_breaks_the_format_is_refused_and_leaves_nothing_behind() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let ok = || image("a.png");
    let with_boundary = |length: usize| Form::new().boundary(&"B".repeat(length)).file("file", "a.png", "image/png", PNG);

    let unfinished = maja.client.send(ok().post_unfinished("/api/uploads")).await;
    let lying = maja
        .client
        .send(
            Request::new(Method::POST, "/api/uploads").bytes(
                "multipart/form-data; boundary=OTHER",
                b"--XXB\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.png\"\r\nContent-Type: image/png\r\n\r\nx\r\n--XXB--\r\n".to_vec(),
            ),
        )
        .await;
    let no_boundary = maja
        .client
        .send(Request::new(Method::POST, "/api/uploads").bytes("multipart/form-data", b"--XXB--\r\n".to_vec()))
        .await;
    let long_boundary = maja.client.send(with_boundary(129).post("/api/uploads")).await;
    let not_a_form = maja
        .client
        .send(Request::new(Method::POST, "/api/uploads").bytes("application/json", b"{}".to_vec()))
        .await;
    let urlencoded = maja
        .client
        .send(Request::new(Method::POST, "/api/uploads").bytes("application/x-www-form-urlencoded", b"a=b".to_vec()))
        .await;
    let no_disposition = maja
        .client
        .send(Form::new().part(b"Content-Type: image/png", PNG).post("/api/uploads"))
        .await;

    for response in [&unfinished, &lying, &no_boundary, &long_boundary, &no_disposition] {
        assert_eq!((response.status.as_u16(), response.body.is_empty()), (400, true));
    }
    assert_eq!(not_a_form.status, 415);
    assert_eq!(urlencoded.status, 415);
    assert!(objects(&server).is_empty());
    assert_eq!(staged(&server), 0);
    let at_the_longest = maja.client.send(with_boundary(128).post("/api/uploads")).await;
    assert_eq!(at_the_longest.status, 201);
}

#[tokio::test]
async fn a_part_may_have_sixteen_headers_and_its_header_lines_under_sixteen_kib() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let head = |extra: String| {
        format!("Content-Disposition: form-data; name=\"file\"; filename=\"a.png\"\r\nContent-Type: image/png{extra}")
    };
    let headers = |count: usize| (0..count).map(|n| format!("\r\nX-H{n}: 1")).collect::<String>();
    let padded = |pad: usize| format!("\r\nX-P: {}", "a".repeat(pad));

    let sixteen = upload(&maja, Form::new().part(head(headers(14)).as_bytes(), PNG)).await;
    let seventeen = upload(&maja, Form::new().part(head(headers(15)).as_bytes(), PNG)).await;
    let longest = upload(&maja, Form::new().part(head(padded(16294)).as_bytes(), PNG)).await;
    let one_more = upload(&maja, Form::new().part(head(padded(16295)).as_bytes(), PNG)).await;
    let endless = upload(&maja, Form::new().part(head(padded(200_000)).as_bytes(), PNG)).await;

    assert_eq!(sixteen.status, 201);
    assert_eq!(longest.status, 201);
    for refused in [&seventeen, &one_more, &endless] {
        assert_eq!(refused.status, 400);
    }
}

#[tokio::test]
async fn a_form_may_have_a_thousand_and_twenty_four_parts() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let with = |extra: usize| {
        let mut form = Form::new();
        for n in 0..extra {
            form = form.text(&format!("f{n}"), "1");
        }
        form.file("file", "a.png", "image/png", PNG)
    };

    assert_eq!(upload(&maja, with(1023)).await.status, 201);
    assert_eq!(upload(&maja, with(1024)).await.status, 400);
}

#[tokio::test]
async fn a_name_that_differs_only_in_case_or_a_long_s_names_the_same_field() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let form = Form::new()
        .part(b"Content-Disposition: form-data; NAME=\"FILE\"; FILENAME=\"a.png\"\r\nContent-Type: image/png", PNG)
        .text("DURATIONSECONDS", "1");

    let on_an_image = upload(&maja, form).await;

    // The duration was read, and an image has none.
    assert_eq!(on_an_image.status, 400);
    assert!(on_an_image.json::<Value>()["errors"]["durationSeconds"].is_array());
}

#[tokio::test]
async fn a_request_that_declares_more_than_the_server_takes_is_refused_unread() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let mut request = image("a.png").post("/api/uploads");
    request = request.header("Content-Length", "134217729");

    let response = maja.client.try_send(request).await;

    // The server answers before the body is sent; a client that cannot send
    // what it declared sees the connection close.
    if let Ok(response) = response {
        assert_eq!(response.status, 413);
    }
    assert!(objects(&server).is_empty());
}

#[tokio::test]
async fn a_file_over_its_limit_is_never_written_to_disk() {
    let server = started(&[("Uploads__MaxFileBytes", "1000")]).await;
    let maja = server.found("maja").await;
    let big = [PDF, &vec![b'x'; 3 * 1024 * 1024][..]].concat();

    let response = upload(&maja, Form::new().file("file", "a.pdf", "application/pdf", &big)).await;

    assert_eq!(response.status, 413);
    assert_eq!(staged(&server), 0);
    assert!(objects(&server).is_empty());
}

/// What bounds the disk the forms being read can fill: the pipeline lets
/// `Uploads:MaxConcurrentUploads` of them run, queues four times as many
/// without reading them, and answers 429 to the rest.
#[tokio::test]
async fn only_as_many_forms_are_read_at_once_as_uploads_may_run() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let server = Server::start_with(&[
        ("ASPNETCORE_ENVIRONMENT", "Production"),
        ("Rtc__TurnSharedSecret", "a-secret-for-the-relay-that-is-long-enough"),
        ("Uploads__MinimumFreeDataRootBytes", "0"),
        ("Uploads__MaxConcurrentUploads", "1"),
    ])
    .await;
    let maja = server.found("maja").await;
    let name = stellarr_api::auth::session::cookie_name(&stellarr_api::settings::Environment::new("Production"));
    let cookie = format!("{name}={}", maja.client.cookie(name).expect("a session"));
    let head = format!(
        "POST /api/uploads HTTP/1.1\r\nHost: x\r\nCookie: {cookie}\r\nX-Stellarr-Client: web\r\nX-Stellarr-Client-Version: 2.0.0\r\nContent-Type: multipart/form-data; boundary=XXB\r\nContent-Length: 100000\r\nConnection: close\r\n\r\n--XXB\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\n%PDF-1 and some more bytes"
    );
    // Seven forms that start and stall: one runs, four wait, two are turned away.
    let mut streams = Vec::new();
    for _ in 0..7 {
        let mut stream = tokio::net::TcpStream::connect(server.address()).await.unwrap();
        stream.write_all(head.as_bytes()).await.unwrap();
        streams.push(stream);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let mut turned_away = 0;
    for stream in &mut streams {
        let mut seen = [0u8; 64];
        if let Ok(Ok(read)) = tokio::time::timeout(std::time::Duration::from_millis(300), stream.read(&mut seen)).await
            && String::from_utf8_lossy(&seen[..read]).starts_with("HTTP/1.1 429")
        {
            turned_away += 1;
        }
    }

    assert_eq!(turned_away, 2);
    assert!(staged(&server) <= 1, "{} files staged", staged(&server));
}

fn resident_mib(pid: u32) -> f64 {
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &pid.to_string()]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().unwrap_or(0.0) / 1024.0
}

/// A form of a thousand fields the route does not read, each nearly 64 KiB,
/// is read and let go: it is not held in memory as it goes by.
#[tokio::test]
async fn the_fields_a_route_does_not_read_are_not_kept() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let pid = server.pid().unwrap();
    let before = resident_mib(pid);
    let mut form = Form::new();
    let value = "v".repeat(60 * 1024);
    for n in 0..1000 {
        form = form.text(&format!("unknown{n}"), &value);
    }
    let form = form.file("file", "a.png", "image/png", PNG);
    let response = upload(&maja, form).await;
    // Memory that was held is mostly still the process's when it is let go.
    let peak = resident_mib(pid);

    assert_eq!(response.status, 201, "{}", response.text());
    assert!(peak < before + 40.0, "the server grew from {before} to {peak} MiB");
}

#[tokio::test]
async fn where_df_cannot_be_run_the_upload_goes_through_and_the_log_says_the_reserve_is_off() {
    let server = Server::start_with(&[
        ("PATH", "/nonexistent"),
        ("Uploads__MinimumFreeDataRootBytes", "9000000000000000"),
    ])
    .await;
    let maja = server.found("maja").await;

    let response = upload(&maja, image("a.png")).await;

    assert_eq!(response.status, 201, "{}", response.text());
    let said = server.log().iter().any(|line| line.contains("free space reserve is not enforced"));
    assert!(said, "{:?}", server.log());
}

/// Sends a form over a socket and hangs up: after all of it (`all`), or
/// after the head of the file part alone.
async fn hang_up(server: &Server, person: &Person, bytes: usize, all: bool) {
    use tokio::io::AsyncWriteExt;
    let name = stellarr_api::auth::session::cookie_name(&stellarr_api::settings::Environment::new("Testing"));
    let cookie = format!("{name}={}", person.client.cookie(name).unwrap());
    let mut file = PDF.to_vec();
    file.resize(bytes, b'x');
    let body = {
        let mut body = b"--XXB\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\n".to_vec();
        body.extend_from_slice(&file);
        body.extend_from_slice(b"\r\n--XXB--\r\n");
        body
    };
    let head = format!(
        "POST /api/uploads HTTP/1.1\r\nHost: x\r\nCookie: {cookie}\r\nX-Stellarr-Client: web\r\nX-Stellarr-Client-Version: 2.0.0\r\nContent-Type: multipart/form-data; boundary=XXB\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    let mut stream = tokio::net::TcpStream::connect(server.address()).await.unwrap();
    stream.write_all(head.as_bytes()).await.unwrap();
    let sent = if all { body.len() } else { body.len() / 2 };
    stream.write_all(&body[..sent]).await.unwrap();
    // Gone before any answer.
    drop(stream);
}

async fn settle(server: &Server) -> (i64, Vec<String>) {
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let rows = server.database().query_row("SELECT COUNT(*) FROM \"ImageAttachments\"", [], |row| row.get(0)).unwrap();
    (rows, objects(server))
}

#[tokio::test]
async fn a_client_that_hangs_up_after_its_whole_form_still_gets_its_file_stored_with_its_row() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    hang_up(&server, &maja, 20 * 1024 * 1024, true).await;
    let (rows, stored) = settle(&server).await;

    // The handler runs to its end without its client: object and row, or neither.
    assert_eq!((rows, stored.len()), (1, 1), "{stored:?}");
    assert_eq!(staged(&server), 0);
}

#[tokio::test]
async fn a_client_that_hangs_up_in_the_middle_of_its_form_leaves_nothing() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    hang_up(&server, &maja, 20 * 1024 * 1024, false).await;
    let (rows, stored) = settle(&server).await;

    assert_eq!((rows, stored.len()), (0, 0), "{stored:?}");
    assert_eq!(staged(&server), 0);
}

#[tokio::test]
async fn the_bytes_of_a_file_past_its_limit_never_reach_the_staging_folder() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let server = started(&[("Uploads__MaxFileBytes", "1000")]).await;
    let maja = server.found("maja").await;
    let name = stellarr_api::auth::session::cookie_name(&stellarr_api::settings::Environment::new("Testing"));
    let cookie = format!("{name}={}", maja.client.cookie(name).unwrap());
    let opening = b"--XXB\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\n";
    let closing = b"\r\n--XXB--\r\n";
    let chunk = vec![b'x'; 64 * 1024];
    let total = opening.len() + 900 + 40 * chunk.len() + closing.len();
    let head = format!(
        "POST /api/uploads HTTP/1.1\r\nHost: x\r\nCookie: {cookie}\r\nX-Stellarr-Client: web\r\nX-Stellarr-Client-Version: 2.0.0\r\nContent-Type: multipart/form-data; boundary=XXB\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
    );
    let largest = |server: &Server| {
        std::fs::read_dir(server.folder().join("incoming"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| entry.metadata().ok())
            .map(|metadata| metadata.len())
            .max()
            .unwrap_or(0)
    };
    let mut stream = tokio::net::TcpStream::connect(server.address()).await.unwrap();
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(opening).await.unwrap();
    stream.write_all(b"%PDF-1.7").await.unwrap();
    stream.write_all(&vec![b'x'; 892]).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let mut seen = largest(&server);
    assert!(seen > 0, "the file under its limit is being staged");

    for _ in 0..40 {
        stream.write_all(&chunk).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        seen = seen.max(largest(&server));
    }
    stream.write_all(closing).await.unwrap();
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer).await;

    assert!(seen <= 1000, "{seen} bytes of a file over its limit were staged");
    assert!(String::from_utf8_lossy(&answer).starts_with("HTTP/1.1 413"), "{}", String::from_utf8_lossy(&answer[..60.min(answer.len())]));
    assert_eq!(staged(&server), 0);
}
