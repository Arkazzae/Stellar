//! The client discovery document and the desktop and Android releases: what
//! an operator publishes on the data volume, and what the clients are told.
//!
//! The routes are not behind the client gate, so every test here also holds
//! that a client of any kind, or none, is answered.

use std::path::Path;

use aws_lc_rs::digest;
use http::Method;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::support::{Request, Response, Server, TempDir, raw_get};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The present moment as the manifests of the deploy scripts write it.
fn now_text() -> String {
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    stellarr_api::values::Timestamp::from_unix_seconds(at.as_secs() as i64)
        .unwrap()
        .to_wire_text()
}

const PACKAGE: &[u8] = b"stellarr-package-bytes";
const CACHE: &str = "public, max-age=60";

fn sha256(bytes: &[u8]) -> String {
    hex(digest::digest(&digest::SHA256, bytes).as_ref())
}

struct Platform {
    folder: &'static str,
    manifest_file: &'static str,
    package_file: fn(&str) -> String,
    name: &'static str,
    architecture: &'static str,
    describe: &'static str,
    download: &'static str,
    content_type: &'static str,
}

const DESKTOP: Platform = Platform {
    folder: "desktop",
    manifest_file: "desktop-release.json",
    package_file: |version| format!("stellarr-desktop-{version}-1-x86_64.pkg.tar.zst"),
    name: "arch",
    architecture: "x86_64",
    describe: "/api/desktop/release",
    download: "/api/desktop/release/arch",
    content_type: "application/zstd",
};

const ANDROID: Platform = Platform {
    folder: "android",
    manifest_file: "android-release.json",
    package_file: |version| format!("stellarr-android-{version}-arm64-v8a.apk"),
    name: "android",
    architecture: "arm64-v8a",
    describe: "/api/android/release",
    download: "/api/android/release/apk",
    content_type: "application/vnd.android.package-archive",
};

const PLATFORMS: [&Platform; 2] = [&DESKTOP, &ANDROID];

fn manifest_path(root: &Path, platform: &Platform) -> std::path::PathBuf {
    root.join("releases")
        .join(platform.folder)
        .join(platform.manifest_file)
}

/// Writes the package and a manifest for it the way the deploy script does.
/// `alter` changes the manifest before it is written.
fn publish(root: &Path, platform: &Platform, version: &str, alter: impl FnOnce(&mut Value)) {
    let directory = root.join("releases").join(platform.folder);
    std::fs::create_dir_all(&directory).unwrap();
    let file_name = (platform.package_file)(version);
    std::fs::write(directory.join(&file_name), PACKAGE).unwrap();
    let mut manifest = json!({
        "schemaVersion": 1,
        "version": version,
        "publishedAt": now_text(),
        "notes": format!("Release {version}"),
        "artifact": {
            "platform": platform.name,
            "architecture": platform.architecture,
            "fileName": file_name,
            "sizeBytes": PACKAGE.len(),
            "sha256": sha256(PACKAGE),
        },
    });
    alter(&mut manifest);
    std::fs::write(manifest_path(root, platform), manifest.to_string()).unwrap();
}

fn plain(_: &mut Value) {}

/// The names of the members of a JSON object in the order they were written,
/// which a parsed value does not keep; none for what is not an object.
fn member_order(text: &str) -> Vec<String> {
    use serde::de::{Deserializer, IgnoredAny, MapAccess, Visitor};

    struct Names(Vec<String>);
    impl<'de> Visitor<'de> for Names {
        type Value = Vec<String>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an object")
        }

        fn visit_map<A: MapAccess<'de>>(mut self, mut map: A) -> Result<Vec<String>, A::Error> {
            while let Some(name) = map.next_key::<String>()? {
                map.next_value::<IgnoredAny>()?;
                self.0.push(name);
            }
            Ok(self.0)
        }
    }
    serde_json::Deserializer::from_str(text)
        .deserialize_map(Names(Vec::new()))
        .unwrap_or_default()
}

fn bad_request_line(response: &Response) -> String {
    format!("{} {}", response.status, response.text())
}

// ---------------------------------------------------------------------------
// The packages an operator publishes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nothing_is_served_before_the_first_release() {
    for platform in PLATFORMS {
        let server = Server::start().await;
        let client = server.client();

        let describe = client.get(platform.describe).await;
        let download = client.get(platform.download).await;
        let listing = client.get("/api/client/releases").await;

        assert_eq!(describe.status, 404, "{}", platform.name);
        assert!(describe.body.is_empty() && describe.header("content-type").is_none());
        assert_eq!(download.status, 404);
        assert_eq!(listing.status, 200);
        assert_eq!(listing.json::<Value>(), json!([]));
        assert_eq!(listing.header("cache-control"), Some(CACHE));
    }
}

#[tokio::test]
async fn a_published_release_is_described_and_its_package_streamed() {
    for platform in PLATFORMS {
        let server = Server::start().await;
        publish(server.folder(), platform, "0.4.2", |manifest| {
            manifest["version"] = json!(" 0.4.2 ");
            manifest["notes"] = json!("  Faster start  ");
        });
        let client = server.client();
        let file_name = (platform.package_file)("0.4.2");

        let describe = client.get(platform.describe).await;
        let metadata: Value = describe.json();

        assert_eq!(describe.status, 200);
        assert_eq!(describe.header("cache-control"), Some(CACHE));
        assert_eq!(
            describe.header("content-type"),
            Some("application/json; charset=utf-8")
        );
        assert_eq!(metadata["version"], "0.4.2");
        assert_eq!(metadata["notes"], "Faster start");
        assert_eq!(metadata["platform"], platform.name);
        assert_eq!(metadata["architecture"], platform.architecture);
        assert_eq!(metadata["fileName"], file_name);
        assert_eq!(metadata["sizeBytes"], PACKAGE.len());
        assert_eq!(metadata["sha256"], sha256(PACKAGE));
        assert_eq!(metadata["downloadUrl"], platform.download);
        assert!(metadata["publishedAt"].as_str().unwrap().ends_with('Z'));
        assert_eq!(
            member_order(&describe.text()),
            [
                "version", "publishedAt", "notes", "platform", "architecture", "fileName",
                "sizeBytes", "sha256", "downloadUrl"
            ],
            "the members come in the order the clients have always seen"
        );

        let download = client.get(metadata["downloadUrl"].as_str().unwrap()).await;
        assert_eq!(download.status, 200);
        assert_eq!(download.header("content-type"), Some(platform.content_type));
        assert_eq!(
            download.header("content-disposition"),
            Some(format!("attachment; filename={file_name}; filename*=UTF-8''{file_name}").as_str())
        );
        assert_eq!(download.header("cache-control"), Some(CACHE));
        assert_eq!(download.header("accept-ranges"), Some("bytes"));
        assert_eq!(download.header("content-length"), Some(PACKAGE.len().to_string().as_str()));
        assert!(download.header("last-modified").unwrap().ends_with(" GMT"));
        assert_eq!(download.body, PACKAGE);
    }
}

#[tokio::test]
async fn an_interrupted_download_can_resume_with_a_range_request() {
    for platform in PLATFORMS {
        let server = Server::start().await;
        publish(server.folder(), platform, "0.4.2", plain);

        let response = server
            .client()
            .send(Request::new(Method::GET, platform.download).header("Range", "bytes=8-14"))
            .await;

        assert_eq!(response.status, 206);
        assert_eq!(response.body, PACKAGE[8..15]);
        assert_eq!(
            response.header("content-range"),
            Some(format!("bytes 8-14/{}", PACKAGE.len()).as_str())
        );
        assert_eq!(response.header("content-length"), Some("7"));
    }
}

#[tokio::test]
async fn a_new_release_is_served_without_restarting_the_server() {
    for platform in PLATFORMS {
        let server = Server::start().await;
        let client = server.client();
        publish(server.folder(), platform, "0.1.0", plain);
        let first: Value = client.get(platform.describe).await.json();

        publish(server.folder(), platform, "0.2.0", plain);
        let second: Value = client.get(platform.describe).await.json();
        std::fs::remove_file(manifest_path(server.folder(), platform)).unwrap();
        let withdrawn = client.get(platform.describe).await;

        assert_eq!(first["version"], "0.1.0");
        assert_eq!(second["version"], "0.2.0");
        assert_eq!(withdrawn.status, 404);
    }
}

async fn assert_unavailable(server: &Server, platform: &Platform) {
    let client = server.client();
    for path in [platform.describe, platform.download, "/api/client/releases"] {
        let response = client.get(path).await;
        assert_eq!(response.status, 503, "{path}: {}", response.text());
        assert_eq!(
            response.header("content-type"),
            Some("application/problem+json")
        );
        assert!(response.header("cache-control").is_none());
        assert_eq!(response.json::<Value>()["status"], 503);
    }
}

#[tokio::test]
async fn a_manifest_that_is_not_json_is_unavailable_rather_than_missing() {
    for platform in PLATFORMS {
        let server = Server::start().await;
        publish(server.folder(), platform, "0.4.2", plain);
        std::fs::write(manifest_path(server.folder(), platform), "{ this is not json").unwrap();

        assert_unavailable(&server, platform).await;
    }
}

#[tokio::test]
async fn a_manifest_with_no_usable_content_is_unavailable() {
    for (platform, content) in [(&DESKTOP, "null"), (&ANDROID, "null"), (&DESKTOP, "[]"), (&ANDROID, "{}")] {
        let server = Server::start().await;
        publish(server.folder(), platform, "0.4.2", plain);
        std::fs::write(manifest_path(server.folder(), platform), content).unwrap();

        assert_unavailable(&server, platform).await;
    }
}

type Corruption = (&'static str, fn(&mut Value));

const CORRUPTIONS: [Corruption; 15] = [
    ("unknown schema version", |m| m["schemaVersion"] = json!(2)),
    ("pre-release version", |m| m["version"] = json!("1.2.3-beta")),
    ("no publication time", |m| {
        m.as_object_mut().unwrap().remove("publishedAt");
    }),
    ("notes that are too long", |m| m["notes"] = json!("x".repeat(2_001))),
    ("no artifact", |m| {
        m.as_object_mut().unwrap().remove("artifact");
    }),
    ("another platform", |m| m["artifact"]["platform"] = json!("windows")),
    ("another architecture", |m| m["artifact"]["architecture"] = json!("riscv64")),
    ("malformed digest", |m| m["artifact"]["sha256"] = json!("ABC")),
    ("no declared size", |m| m["artifact"]["sizeBytes"] = json!(0)),
    ("size the file does not have", |m| {
        m["artifact"]["sizeBytes"] = json!(PACKAGE.len() + 1)
    }),
    ("file that is not there", |m| m["artifact"]["fileName"] = json!("missing.bin")),
    ("file name that climbs out of the directory", |m| m["artifact"]["fileName"] = json!("..")),
    ("blank file name", |m| m["artifact"]["fileName"] = json!(" ")),
    // Each of these resolves to the real package, so only the spelling gives them away.
    ("file name with a current-directory prefix", |m| prefix_file_name(m, "./")),
    ("file name that detours through a subdirectory", |m| prefix_file_name(m, "sub/../")),
];

fn prefix_file_name(manifest: &mut Value, prefix: &str) {
    let name = manifest["artifact"]["fileName"].as_str().unwrap().to_string();
    manifest["artifact"]["fileName"] = json!(format!("{prefix}{name}"));
}

#[tokio::test]
async fn a_manifest_that_cannot_be_trusted_is_unavailable() {
    for platform in PLATFORMS {
        for (name, corrupt) in CORRUPTIONS {
            let server = Server::start().await;
            publish(server.folder(), platform, "0.4.2", corrupt);

            let response = server.client().get(platform.describe).await;
            let download = server.client().get(platform.download).await;
            let listing = server.client().get("/api/client/releases").await;

            assert_eq!(
                (response.status.as_u16(), download.status.as_u16(), listing.status.as_u16()),
                (503, 503, 503),
                "{} {name}",
                platform.name
            );
        }
    }
}

#[tokio::test]
async fn a_package_outside_the_release_directory_is_never_served() {
    for platform in PLATFORMS {
        for absolute in [false, true] {
            let server = Server::start().await;
            // The same size as declared, so that only its location gives it away.
            let outside = server.folder().join("releases").join("outside.bin");
            publish(server.folder(), platform, "0.4.2", |manifest| {
                manifest["artifact"]["fileName"] = json!(if absolute {
                    outside.display().to_string()
                } else {
                    "../outside.bin".to_string()
                });
            });
            std::fs::write(&outside, PACKAGE).unwrap();

            assert_unavailable(&server, platform).await;
        }
    }
}

#[tokio::test]
async fn the_listing_names_the_android_release_first_and_then_the_desktop_one() {
    let server = Server::start().await;
    publish(server.folder(), &DESKTOP, "0.1.3", plain);
    publish(server.folder(), &ANDROID, "0.1.0", plain);

    let response = server.client().get("/api/client/releases").await;
    let releases: Value = response.json();

    assert_eq!(response.status, 200);
    assert_eq!(response.header("cache-control"), Some(CACHE));
    assert_eq!(releases.as_array().unwrap().len(), 2);
    assert_eq!(releases[0]["platform"], "android");
    assert_eq!(releases[0]["downloadUrl"], ANDROID.download);
    assert_eq!(releases[1]["platform"], "arch");
    assert_eq!(releases[1]["downloadUrl"], DESKTOP.download);
}

#[tokio::test]
async fn the_listing_still_offers_the_other_platform_when_one_manifest_is_broken() {
    for (broken, healthy) in [(&DESKTOP, &ANDROID), (&ANDROID, &DESKTOP)] {
        let server = Server::start().await;
        publish(server.folder(), healthy, "0.1.3", plain);
        publish(server.folder(), broken, "0.1.3", plain);
        std::fs::write(manifest_path(server.folder(), broken), "{}").unwrap();
        let client = server.client();

        let releases: Value = client.get("/api/client/releases").await.json();

        assert_eq!(releases.as_array().unwrap().len(), 1);
        assert_eq!(releases[0]["platform"], healthy.name);
        assert_eq!(client.get(broken.describe).await.status, 503);
        assert_eq!(client.get(healthy.describe).await.status, 200);
    }
}

#[tokio::test]
async fn the_problem_of_a_broken_manifest_is_told_in_the_language_of_the_instance() {
    let server = Server::start_with(&[("STELLARR_LANGUAGE", "pl")]).await;
    publish(server.folder(), &DESKTOP, "0.1.3", plain);
    std::fs::write(manifest_path(server.folder(), &DESKTOP), "{}").unwrap();
    let client = server.client();

    let desktop: Value = client.get(DESKTOP.describe).await.json();
    let listing: Value = client.get("/api/client/releases").await.json();

    assert_eq!(desktop["title"], "Wydanie desktopowe jest chwilowo niedostępne.");
    assert_eq!(listing["title"], "Lista wydań aplikacji jest chwilowo niedostępna.");
}

#[tokio::test]
async fn a_publication_that_is_a_folder_where_a_file_belongs_is_a_failure_of_the_server() {
    let server = Server::start().await;
    std::fs::create_dir_all(manifest_path(server.folder(), &DESKTOP)).unwrap();

    let response = server.client().get(DESKTOP.describe).await;

    assert_eq!(response.status, 500, "{}", bad_request_line(&response));
}

#[tokio::test]
async fn a_package_that_is_replaced_by_one_of_another_size_is_not_served() {
    let server = Server::start().await;
    publish(server.folder(), &DESKTOP, "0.4.2", plain);
    let package = server.folder().join("releases/desktop").join((DESKTOP.package_file)("0.4.2"));
    std::fs::write(&package, b"a package still being copied").unwrap();

    assert_eq!(server.client().get(DESKTOP.download).await.status, 503);
}

#[tokio::test]
async fn every_route_answers_every_kind_of_client() {
    let server = Server::start_with(&[("Desktop__MinimumVersion", "9.9.9")]).await;
    publish(server.folder(), &DESKTOP, "0.4.2", plain);
    publish(server.folder(), &ANDROID, "0.4.2", plain);
    let clients = [
        ("no headers", server.anonymous()),
        ("another product", server.client_of("voicephy", "9.9.9")),
        ("no version", server.anonymous().with_header("X-Stellarr-Client", "web")),
        ("a desktop below the floor", server.client_of("desktop-linux", "0.0.1")),
        ("the web", server.client()),
    ];
    for (who, client) in clients {
        for path in [
            "/.well-known/stellarr/client",
            "/api/desktop/release",
            "/api/desktop/release/arch",
            "/api/android/release",
            "/api/android/release/apk",
            "/api/client/releases",
            "/api/desktop/update/linux/x86_64/0.1.9",
            "/api/desktop/update/download/stable/appimage",
        ] {
            let response = client.get(path).await;
            assert!(
                matches!(response.status.as_u16(), 200 | 204 | 404),
                "{who} {path}: {}",
                response.status
            );
            assert_ne!(response.status, 403);
        }
    }
}

#[tokio::test]
async fn only_get_is_served() {
    let server = Server::start().await;
    for path in [DESKTOP.describe, DESKTOP.download, "/api/client/releases", "/.well-known/stellarr/client"] {
        for method in [Method::POST, Method::PUT, Method::DELETE, Method::HEAD] {
            let response = server.client().send(Request::new(method.clone(), path)).await;
            assert_eq!(response.status, 405, "{method} {path}");
            assert_eq!(response.header("allow"), Some("GET"));
        }
    }
}

#[tokio::test]
async fn a_trailing_slash_does_not_change_the_route() {
    let server = Server::start().await;
    publish(server.folder(), &DESKTOP, "0.4.2", plain);

    for path in ["/api/desktop/release/", "/api/desktop/release/arch/", "/api/client/releases/", "/.well-known/stellarr/client/"] {
        assert_eq!(server.client().get(path).await.status, 200, "{path}");
    }
}

// ---------------------------------------------------------------------------
// The package as a download: dates, conditions and ranges
// ---------------------------------------------------------------------------

fn conditional(path: &str, headers: &[(&str, &str)]) -> Request {
    headers
        .iter()
        .fold(Request::new(Method::GET, path), |request, (name, value)| request.header(name, value))
}

#[tokio::test]
async fn the_package_is_dated_with_the_moment_of_its_publication() {
    let server = Server::start().await;
    publish(server.folder(), &DESKTOP, "0.4.2", |manifest| {
        manifest["publishedAt"] = json!("2026-11-20T10:11:12.9876543Z")
    });
    let client = server.client();

    let whole = client.get(DESKTOP.download).await;
    let same = client
        .send(conditional(DESKTOP.download, &[("If-Modified-Since", "Fri, 20 Nov 2026 10:11:12 GMT")]))
        .await;
    let older = client
        .send(conditional(DESKTOP.download, &[("If-Modified-Since", "Fri, 20 Nov 2026 10:11:11 GMT")]))
        .await;
    let failed = client
        .send(conditional(DESKTOP.download, &[("If-Unmodified-Since", "Fri, 20 Nov 2026 10:11:11 GMT")]))
        .await;

    assert_eq!(whole.header("last-modified"), Some("Fri, 20 Nov 2026 10:11:12 GMT"));
    assert_eq!(same.status, 304);
    assert!(same.body.is_empty());
    assert_eq!(same.header("last-modified"), Some("Fri, 20 Nov 2026 10:11:12 GMT"));
    assert_eq!(same.header("cache-control"), Some(CACHE));
    assert!(same.header("content-type").is_none() && same.header("accept-ranges").is_none());
    assert_eq!(older.status, 200);
    assert_eq!(older.body, PACKAGE);
    assert_eq!(failed.status, 412);
    assert!(failed.body.is_empty());
}

#[tokio::test]
async fn a_range_is_honoured_as_far_as_the_file_goes() {
    let server = Server::start().await;
    publish(server.folder(), &DESKTOP, "0.4.2", |manifest| {
        manifest["publishedAt"] = json!("2026-11-20T10:11:12Z")
    });
    let client = server.client();
    let length = PACKAGE.len();
    let ask = |range: &'static str| {
        let client = &client;
        async move {
            client
                .send(conditional(DESKTOP.download, &[("Range", range)]))
                .await
        }
    };

    let tail = ask("bytes=-5").await;
    let open = ask("bytes=20-").await;
    let clamped = ask("bytes=0-99999").await;
    let past = ask("bytes=99999-").await;
    let none = ask("bytes=-0").await;
    let two = ask("bytes=0-1,5-9").await;
    let nonsense = ask("bytes=abc").await;
    let backwards = ask("bytes=5-2").await;

    assert_eq!((tail.status.as_u16(), &tail.body[..]), (206, &PACKAGE[length - 5..]));
    assert_eq!((open.status.as_u16(), &open.body[..]), (206, &PACKAGE[20..]));
    assert_eq!(clamped.status, 206);
    assert_eq!(clamped.body, PACKAGE);
    assert_eq!(
        clamped.header("content-range"),
        Some(format!("bytes 0-{}/{length}", length - 1).as_str())
    );
    for refused in [&past, &none] {
        assert_eq!(refused.status, 416);
        assert_eq!(refused.header("content-range"), Some(format!("bytes */{length}").as_str()));
        assert!(refused.body.is_empty());
    }
    for whole in [&two, &nonsense, &backwards] {
        assert_eq!(whole.status, 200);
        assert_eq!(whole.body, PACKAGE);
        assert!(whole.header("content-range").is_none());
    }
    // A range that goes with a copy the client has: only while it is still the same.
    let still = client
        .send(conditional(DESKTOP.download, &[("Range", "bytes=0-3"), ("If-Range", "Fri, 20 Nov 2026 10:11:12 GMT")]))
        .await;
    let changed = client
        .send(conditional(DESKTOP.download, &[("Range", "bytes=0-3"), ("If-Range", "Fri, 20 Nov 2026 10:11:11 GMT")]))
        .await;
    assert_eq!((still.status.as_u16(), still.body.len()), (206, 4));
    assert_eq!((changed.status.as_u16(), changed.body.len()), (200, length));
}

#[tokio::test]
async fn the_name_a_package_is_saved_under_is_made_safe_for_every_browser() {
    let server = Server::start().await;
    for (name, expected) in [
        ("a b.bin", "attachment; filename=\"a b.bin\"; filename*=UTF-8''a%20b.bin"),
        ("żółw.bin", "attachment; filename=___w.bin; filename*=UTF-8''%C5%BC%C3%B3%C5%82w.bin"),
        ("quo\"te.bin", "attachment; filename=\"quo\\\"te.bin\"; filename*=UTF-8''quo%22te.bin"),
        ("new\nline.bin", "attachment; filename=new_line.bin; filename*=UTF-8''new%0Aline.bin"),
        ("back\\slash.bin", "attachment; filename=\"back\\\\slash.bin\"; filename*=UTF-8''back%5Cslash.bin"),
    ] {
        publish(server.folder(), &DESKTOP, "0.4.2", |manifest| {
            manifest["artifact"]["fileName"] = json!(name)
        });
        std::fs::write(server.folder().join("releases/desktop").join(name), PACKAGE).unwrap();

        let response = server.client().get(DESKTOP.download).await;

        assert_eq!(response.status, 200, "{name:?}");
        assert_eq!(response.header("content-disposition"), Some(expected), "{name:?}");
        let value = response.headers.get("content-disposition").unwrap();
        assert!(value.as_bytes().iter().all(|b| (0x20..0x7f).contains(b)));
    }
}

#[tokio::test]
async fn a_manifest_may_be_written_in_the_spellings_the_first_server_read() {
    let server = Server::start().await;
    publish(server.folder(), &DESKTOP, "0.4.2", plain);
    let mut text = std::fs::read_to_string(manifest_path(server.folder(), &DESKTOP)).unwrap();
    text = text
        .replace("schemaVersion", "SchemaVersion")
        .replace("\"sizeBytes\":22", "\"sizeBytes\":\"22\"")
        .replace("\"version\":\"0.4.2\"", "\"version\":\" 0.4.2\\t\"");
    let mut bytes = b"\xEF\xBB\xBF".to_vec();
    bytes.extend(text.into_bytes());
    std::fs::write(manifest_path(server.folder(), &DESKTOP), bytes).unwrap();

    let response = server.client().get(DESKTOP.describe).await;

    assert_eq!(response.status, 200, "{}", response.text());
    assert_eq!(response.json::<Value>()["version"], "0.4.2");
}

// ---------------------------------------------------------------------------
// The document a client reads before it signs in
// ---------------------------------------------------------------------------

const DISCOVERY: &str = "/.well-known/stellarr/client";

fn insert_server(server: &Server, name: &str, created: &str, id: &str) {
    server
        .database()
        .execute(
            "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\")
             VALUES (?1, ?2, ?3, 1, 1)",
            [id, name, created],
        )
        .unwrap();
}

fn insert_operations(connection: &Connection, server: &str, minimum: Option<&str>, recommended: Option<&str>) {
    connection
        .execute(
            "INSERT INTO \"OperationsSettings\" (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\",
                 \"InviteMailEnabled\", \"RecoveryMailEnabled\", \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\",
                 \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\", \"UpdatedAt\", \"DesktopMinimumVersion\", \"DesktopRecommendedVersion\")
             VALUES (?1, '', 'UTC', 0, '', 0, 0, 0, 7, 7, 7, 0, '2026-11-24 00:33:18', ?2, ?3)",
            rusqlite::params![server, minimum, recommended],
        )
        .unwrap();
}

#[tokio::test]
async fn an_unconfigured_instance_introduces_itself_by_its_server_and_the_request() {
    let server = Server::start().await;
    insert_server(&server, "Orbit", "2026-11-24 00:33:18", "3D82DA44-8AA5-4BDF-9EE1-2902DF99880C");

    let response = server.anonymous().get(DISCOVERY).await;
    let document: Value = response.json();

    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-type"), Some("application/json; charset=utf-8"));
    assert_eq!(document["instanceName"], "Orbit");
    assert_eq!(document["canonicalOrigin"], format!("http://{}", server.address()));
    assert_eq!(document["buildSha"], Value::Null);
    assert_eq!(document["desktop"]["minimumVersion"], "0.1.0");
    assert_eq!(document["desktop"]["recommendedVersion"], "0.1.3");
    assert!(!document["serverVersion"].as_str().unwrap().trim().is_empty());
    assert_eq!(
        member_order(&response.text()),
        [
            "product", "instanceId", "instanceName", "canonicalOrigin", "serverVersion", "buildSha",
            "apiProtocol", "apiProtocolMin", "apiProtocolMax", "rtcProtocol", "auth", "desktop"
        ]
    );
}

#[tokio::test]
async fn the_first_server_by_creation_names_the_instance() {
    let server = Server::start().await;
    insert_server(&server, "Second", "2026-11-24 00:33:19", "00000000-0000-4000-8000-000000000001");
    insert_server(&server, "Tie B", "2026-11-24 00:33:18", "FFFFFFFF-0000-4000-8000-000000000001");
    insert_server(&server, "Tie A", "2026-11-24 00:33:18", "AAAAAAAA-0000-4000-8000-000000000001");

    let document: Value = server.anonymous().get(DISCOVERY).await.json();

    assert_eq!(document["instanceName"], "Tie A");
}

#[tokio::test]
async fn an_instance_without_any_server_is_called_by_the_product_name() {
    let server = Server::start().await;

    let document: Value = server.anonymous().get(DISCOVERY).await.json();

    assert_eq!(document["instanceName"], "Stellarr");
}

#[tokio::test]
async fn the_origin_follows_the_address_the_proxy_forwarded() {
    let server = Server::start().await;
    let client = server
        .anonymous()
        .with_header("X-Forwarded-Host", "chat.example.test")
        .with_header("X-Forwarded-Proto", "https");

    let document: Value = client.get(DISCOVERY).await.json();

    assert_eq!(document["canonicalOrigin"], "https://chat.example.test");
}

#[tokio::test]
async fn a_misconfigured_canonical_origin_stops_the_start_instead_of_a_silent_guess() {
    for origin in [
        "chat.example.test",
        "ftp://chat.example.test",
        "https://operator:secret@chat.example.test",
        "https://chat.example.test/?next=1",
        "https://chat.example.test/#fragment",
    ] {
        let said = Server::refused_to_start(&[("Desktop__CanonicalOrigin", origin)]).await;

        assert!(said.contains("Desktop:CanonicalOrigin must be an absolute HTTP(S) origin"), "{origin}: {said}");
    }
}

#[tokio::test]
async fn an_instance_id_already_on_the_data_volume_is_kept() {
    let server = Server::start().await;
    std::fs::write(server.folder().join("instance-id"), "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee\n").unwrap();

    let document: Value = server.anonymous().get(DISCOVERY).await.json();

    assert_eq!(document["instanceId"], "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee");
}

#[tokio::test]
async fn a_file_with_no_instance_id_in_it_is_a_failure_of_the_server() {
    let server = Server::start().await;
    std::fs::write(server.folder().join("instance-id"), "nonsense").unwrap();

    let response = server.anonymous().get(DISCOVERY).await;

    assert_eq!(response.status, 500);
    assert_eq!(
        std::fs::read_to_string(server.folder().join("instance-id")).unwrap(),
        "nonsense"
    );
}

#[tokio::test]
async fn versions_the_operator_saved_win_and_are_the_floor_the_gate_enforces() {
    let server = Server::start_with(&[
        ("Desktop__MinimumVersion", "0.1.0"),
        ("Desktop__RecommendedVersion", "0.2.0"),
    ])
    .await;
    let id = "3D82DA44-8AA5-4BDF-9EE1-2902DF99880C";
    insert_server(&server, "Orbit", "2026-11-24 00:33:18", id);
    insert_operations(&server.database(), id, Some(" 0.3.0 "), None);
    let old_desktop = server.client_of("desktop-linux", "0.2.9");

    let document: Value = server.anonymous().get(DISCOVERY).await.json();
    let refused = old_desktop.get("/api/session").await;
    let problem: Value = refused.json();

    assert_eq!(document["desktop"]["minimumVersion"], "0.3.0");
    // An unsaved recommendation still comes from the configuration.
    assert_eq!(document["desktop"]["recommendedVersion"], "0.2.0");
    assert_eq!(refused.status, 403);
    assert_eq!(problem["minimumVersion"], document["desktop"]["minimumVersion"]);
}

#[tokio::test]
async fn an_anonymous_desktop_handshake_describes_the_compatible_instance() {
    let server = Server::start_with(&[
        ("Stellarr__InstanceId", "11111111-2222-4333-8444-555555555555"),
        ("Desktop__CanonicalOrigin", "https://chat.example.test/"),
        ("Desktop__MinimumVersion", "0.1.0"),
        ("Desktop__RecommendedVersion", "0.2.0"),
        ("Build__Sha", "abc1234"),
    ])
    .await;

    let handshake: Value = server.anonymous().get(DISCOVERY).await.json();

    assert_eq!(handshake["product"], "stellarr");
    assert_eq!(handshake["instanceId"], "11111111-2222-4333-8444-555555555555");
    assert_eq!(handshake["instanceName"], "Stellarr");
    assert_eq!(handshake["canonicalOrigin"], "https://chat.example.test");
    assert_eq!(handshake["buildSha"], "abc1234");
    assert_eq!(handshake["apiProtocol"], 1);
    assert_eq!(handshake["apiProtocolMin"], 1);
    assert_eq!(handshake["apiProtocolMax"], 1);
    assert_eq!(handshake["rtcProtocol"], 1);
    assert_eq!(handshake["auth"], json!(["password", "invite"]));
    assert_eq!(handshake["desktop"], json!({ "minimumVersion": "0.1.0", "recommendedVersion": "0.2.0" }));
}

#[tokio::test]
async fn a_generated_instance_identity_survives_an_application_restart() {
    let folder = TempDir::new();
    let first = Server::start_in(folder.path(), &[]).await;
    let made: Value = first.anonymous().get(DISCOVERY).await.json();
    first.stop().await;

    let restarted = Server::start_in(folder.path(), &[]).await;
    let kept: Value = restarted.anonymous().get(DISCOVERY).await.json();

    assert!(made["instanceId"].as_str().is_some_and(|id| id.len() == 36));
    assert_eq!(made["instanceId"], kept["instanceId"]);
}

#[tokio::test]
async fn an_instance_id_that_is_not_a_uuid_stops_the_start() {
    let said = Server::refused_to_start(&[("STELLARR_INSTANCE_ID", "not-a-uuid")]).await;

    assert!(said.contains("Stellarr:InstanceId must be a UUID."), "{said}");
}

// ---------------------------------------------------------------------------
// What an installed desktop client is told about updates
// ---------------------------------------------------------------------------

const SIGNATURE: &str = "untrusted comment: signature from minisign secret key\nRWSRGejCl7EEOExTHJ2LUMF6MF1yBuIBWKvgYWuFE0duj3HfmJq9rj4BAAAAAAAAAAA=";
const APP_IMAGE: &[u8] = b"stellarr-appimage-bytes";

fn check(installed: &str, bucket: i64) -> String {
    format!("/api/desktop/update/linux/x86_64/{installed}?channel=stable&bundle=appimage&bucket={bucket}")
}

fn update_manifest_file(channel: &str) -> &'static str {
    if channel == "beta" {
        "desktop-updater-beta.json"
    } else {
        "desktop-updater.json"
    }
}

/// An AppImage and the manifest of the updater for it.
fn publish_update(
    root: &Path,
    version: &str,
    bytes: &[u8],
    channel: &str,
    rollout: i64,
    alter: impl FnOnce(&mut Value),
) {
    let directory = root.join("releases").join("desktop");
    std::fs::create_dir_all(&directory).unwrap();
    let file_name = format!("stellarr-desktop-{version}-x86_64.AppImage");
    std::fs::write(directory.join(&file_name), bytes).unwrap();
    let mut manifest = json!({
        "schemaVersion": 1,
        "channel": channel,
        "version": version,
        "publishedAt": now_text(),
        "notes": format!("Stellarr Desktop {version}"),
        "rolloutPercent": rollout,
        "artifacts": [{
            "target": "linux",
            "architecture": "x86_64",
            "bundle": "appimage",
            "fileName": file_name,
            "sizeBytes": bytes.len(),
            "sha256": sha256(bytes),
            "signature": SIGNATURE,
        }],
    });
    alter(&mut manifest);
    std::fs::write(directory.join(update_manifest_file(channel)), manifest.to_string()).unwrap();
}

fn publish_stable(root: &Path, version: &str) {
    publish_update(root, version, APP_IMAGE, "stable", 100, plain);
}

#[tokio::test]
async fn a_client_on_an_older_version_is_offered_the_signed_appimage() {
    let server = Server::start().await;
    publish_update(server.folder(), "0.2.0", b"stellarr-appimage", "stable", 100, plain);
    let client = server.client();

    let response = client.get(&check("0.1.9", 0)).await;
    let body: Value = response.json();

    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-type"), Some("application/json; charset=utf-8"));
    assert_eq!(body["version"], "0.2.0");
    assert_eq!(body["signature"], SIGNATURE);
    assert!(body["pub_date"].as_str().is_some());
    // The address is composed from the request, so that the same manifest is
    // right whatever name an instance answers on.
    assert_eq!(
        body["url"],
        format!("http://{}/api/desktop/update/download/stable/appimage", server.address())
    );
    assert_eq!(
        member_order(&response.text()),
        ["version", "pub_date", "url", "signature", "notes"]
    );

    let download = client.get(body["url"].as_str().unwrap().split_once(&server.address().to_string()).unwrap().1).await;
    assert_eq!(download.status, 200);
    assert_eq!(download.body, b"stellarr-appimage".as_slice());
}

#[tokio::test]
async fn the_address_of_the_download_follows_the_host_and_scheme_the_proxy_forwarded() {
    let server = Server::start().await;
    publish_stable(server.folder(), "0.2.0");
    let client = server
        .client()
        .with_header("X-Forwarded-Host", "chat.example.test")
        .with_header("X-Forwarded-Proto", "https");

    let body: Value = client.get(&check("0.1.9", 0)).await.json();

    assert_eq!(body["url"], "https://chat.example.test/api/desktop/update/download/stable/appimage");
}

#[tokio::test]
async fn a_client_already_on_the_published_version_is_told_there_is_nothing() {
    let server = Server::start().await;
    publish_stable(server.folder(), "0.2.0");

    let response = server.client().get(&check("0.2.0", 0)).await;

    assert_eq!(response.status, 204);
    assert!(response.body.is_empty());
}

#[tokio::test]
async fn a_system_packaged_client_is_not_offered_an_appimage_it_cannot_install() {
    let server = Server::start().await;
    publish_stable(server.folder(), "0.2.0");

    let response = server
        .client()
        .get("/api/desktop/update/linux/x86_64/0.1.9?channel=stable&bundle=unknown&bucket=0")
        .await;

    assert_eq!(response.status, 204);
}

#[tokio::test]
async fn a_staged_rollout_only_answers_the_buckets_it_has_reached() {
    let server = Server::start().await;
    publish_update(server.folder(), "0.2.0", APP_IMAGE, "stable", 25, plain);

    let inside = server.client().get(&check("0.1.9", 24)).await;
    let outside = server.client().get(&check("0.1.9", 25)).await;

    assert_eq!(inside.status, 200);
    assert_eq!(outside.status, 204);
}

/// A client that cannot say which bucket it is in must not be treated as an
/// early adopter: an install that forgets its identity every launch would
/// otherwise walk into every partial rollout.
#[tokio::test]
async fn a_client_without_a_bucket_waits_for_the_full_rollout() {
    let server = Server::start().await;
    let path = "/api/desktop/update/linux/x86_64/0.1.9?channel=stable&bundle=appimage";
    publish_update(server.folder(), "0.2.0", APP_IMAGE, "stable", 99, plain);

    let partial = server.client().get(path).await;
    publish_update(server.folder(), "0.2.0", APP_IMAGE, "stable", 100, plain);
    let complete = server.client().get(path).await;

    assert_eq!(partial.status, 204);
    assert_eq!(complete.status, 200);
}

#[tokio::test]
async fn the_beta_channel_is_served_from_its_own_manifest() {
    let server = Server::start().await;
    publish_stable(server.folder(), "0.2.0");
    publish_update(server.folder(), "0.3.0", b"stellarr-appimage-beta", "beta", 100, plain);
    let client = server.client();

    let stable: Value = client.get("/api/desktop/update/linux/x86_64/0.1.9?channel=stable&bundle=appimage&bucket=0").await.json();
    let beta: Value = client.get("/api/desktop/update/linux/x86_64/0.1.9?channel=beta&bundle=appimage&bucket=0").await.json();
    let beta_file = client.get("/api/desktop/update/download/beta/appimage").await;

    assert_eq!(stable["version"], "0.2.0");
    assert_eq!(beta["version"], "0.3.0");
    assert_eq!(beta["url"].as_str().unwrap().rsplit_once("/api").unwrap().1, "/desktop/update/download/beta/appimage");
    assert_eq!(beta_file.body, b"stellarr-appimage-beta".as_slice());
}

#[tokio::test]
async fn an_unpublished_channel_and_a_foreign_architecture_answer_nothing_rather_than_fail() {
    let server = Server::start().await;
    publish_stable(server.folder(), "0.2.0");
    let client = server.client();

    for path in [
        "/api/desktop/update/linux/x86_64/0.1.9?channel=beta&bundle=appimage&bucket=0",
        "/api/desktop/update/linux/aarch64/0.1.9?channel=stable&bundle=appimage&bucket=0",
        "/api/desktop/update/windows/x86_64/0.1.9?channel=stable&bundle=appimage&bucket=0",
        "/api/desktop/update/linux/x86_64/0.1.9?channel=unknown&bundle=appimage&bucket=0",
        "/api/desktop/update/linux/x86_64/0.1.9?channel=&bundle=appimage&bucket=0",
        "/api/desktop/update/linux/x86_64/0.1.9?channel=stable&bundle=&bucket=0",
        "/api/desktop/update/LINUX/x86_64/0.1.9?channel=stable&bundle=appimage&bucket=0",
    ] {
        assert_eq!(client.get(path).await.status, 204, "{path}");
    }
}

/// A manifest naming a file outside the release directory is the one failure
/// that must not be answered with "nothing for you": it is a broken or
/// tampered publication, and 204 would hide it behind clients that simply
/// never update.
#[tokio::test]
async fn a_manifest_reaching_outside_the_release_directory_is_refused() {
    let server = Server::start().await;
    publish_update(server.folder(), "0.2.0", APP_IMAGE, "stable", 100, |manifest| {
        manifest["artifacts"][0]["fileName"] = json!("../../../etc/passwd")
    });

    let response = server.client().get(&check("0.1.9", 0)).await;

    assert_eq!(response.status, 503);
}

#[tokio::test]
async fn nothing_is_published_before_the_first_update() {
    let server = Server::start().await;
    let client = server.client();

    let asked = client.get(&check("0.1.9", 0)).await;
    let download = client.get("/api/desktop/update/download/stable/appimage").await;

    assert_eq!(asked.status, 204);
    assert_eq!(download.status, 404);
    assert!(download.body.is_empty());
}

#[tokio::test]
async fn an_offered_update_is_not_cached_and_names_its_publication_time() {
    let server = Server::start().await;
    publish_update(server.folder(), "0.2.0", APP_IMAGE, "stable", 100, |manifest| {
        manifest["publishedAt"] = json!("2026-11-20T10:11:12.5Z")
    });

    let response = server.client().get(&check("0.1.9", 0)).await;
    let offer: Value = response.json();

    assert_eq!(response.status, 200);
    assert_eq!(response.header("cache-control"), Some("no-store"));
    assert_eq!(offer["notes"], "Stellarr Desktop 0.2.0");
    assert_eq!(offer["pub_date"], "2026-11-20T10:11:12.5000000Z");
}

#[tokio::test]
async fn a_client_not_behind_the_published_version_or_not_saying_its_version_is_told_there_is_nothing() {
    let server = Server::start().await;
    publish_stable(server.folder(), "0.2.0");

    for installed in ["0.2.0", "0.3.0", "latest", "0.1", "0.1.9-beta", "0.2.0.1", "%20", "999999999999.0.0"] {
        let response = server.client().get(&check(installed, 0)).await;
        assert_eq!(response.status, 204, "{installed}");
    }
    for installed in ["0.1.9", "00.001.009", "%200.1.9%20"] {
        let response = server.client().get(&check(installed, 0)).await;
        assert_eq!(response.status, 200, "{installed}");
    }
}

#[tokio::test]
async fn a_bucket_above_the_valid_range_counts_as_the_last_one_and_a_negative_one_is_refused() {
    for (rollout, bucket, expected) in [
        (100, 99, 200),
        (100, 1000, 200),
        (100, -1, 400),
        (99, 1000, 204),
        (99, -1, 400),
        (50, 49, 200),
        (50, 50, 204),
        (0, 0, 204),
    ] {
        let server = Server::start().await;
        publish_update(server.folder(), "0.2.0", APP_IMAGE, "stable", rollout, plain);

        let response = server.client().get(&check("0.1.9", bucket)).await;

        assert_eq!(response.status, expected, "rollout {rollout}, bucket {bucket}");
    }
}

#[tokio::test]
async fn a_query_that_is_not_one_of_an_updater_is_a_bad_request_or_read_as_the_first_server_read_it() {
    let server = Server::start().await;
    publish_stable(server.folder(), "0.2.0");
    let client = server.client();
    let base = "/api/desktop/update/linux/x86_64/0.1.9";

    for (query, expected) in [
        ("bucket=abc", 400),
        ("bucket=", 400),
        ("bucket=1&bucket=2", 400),
        ("bucket=5.0", 400),
        ("bucket=99999999999", 400),
        ("bucket=-99999999999", 400),
        ("bucket=%2D1", 400),
        ("bucket=%2B5", 400),
        ("bucket=+5", 400),
        ("bucket=%095%0A", 400),
        ("bucket=3&bucket=", 400),
        ("bucket=&bucket=3", 400),
        ("bucket=3&bucket=4", 400),
        ("bucket=&bucket=", 400),
        ("bucket=1%00", 400),
        ("bucket=1%20%00", 400),
        ("bucket=%001", 400),
        ("bucket=1%00%20", 400),
        ("bucket=%0B1", 400),
        ("bucket=%1C1", 400),
        ("bucket=%C2%A01", 400),
        ("bucket=%EF%BC%91", 400),
        ("bucket=2147483648", 400),
        ("channel=stable&channel=&bucket=0", 400),
        ("channel=stable&channel=stable&bucket=0", 400),
        ("Bucket=5&CHANNEL=stable&Bundle=appimage", 200),
        ("channel=stable&channel=beta", 400),
        ("bucket=5&channel", 204),
        ("", 200),
    ] {
        let response = client.get(&format!("{base}?{query}")).await;
        assert_eq!(response.status, expected, "{query}");
    }
}

#[tokio::test]
async fn the_download_of_something_not_published_is_not_found() {
    let server = Server::start().await;
    publish_stable(server.folder(), "0.2.0");

    for path in [
        "unknown/appimage",
        "beta/appimage",
        "stable/deb",
        "Stable/appimage",
        "stable/AppImage",
        "x86_64/0.1.9",
        "stable",
        "stable/appimage/extra",
        "..%2f..%2fetc/passwd",
        "stable/..%2fappimage",
        "stable/%00",
    ] {
        let response = server.client().get(&format!("/api/desktop/update/download/{path}")).await;
        assert!(matches!(response.status.as_u16(), 404 | 400), "{path}: {}", response.status);
        assert!(response.body.is_empty(), "{path}");
    }
}

#[tokio::test]
async fn the_download_is_revalidated_soon_and_can_be_resumed() {
    let server = Server::start().await;
    publish_stable(server.folder(), "0.2.0");
    let client = server.client();
    let path = "/api/desktop/update/download/stable/appimage";

    let whole = client.get(path).await;
    let part = client.send(conditional(path, &[("Range", "bytes=9-16")])).await;

    assert_eq!(whole.status, 200);
    assert_eq!(whole.header("cache-control"), Some(CACHE));
    assert_eq!(whole.header("content-type"), Some("application/octet-stream"));
    assert_eq!(
        whole.header("content-disposition"),
        Some("attachment; filename=stellarr-desktop-0.2.0-x86_64.AppImage; filename*=UTF-8''stellarr-desktop-0.2.0-x86_64.AppImage")
    );
    assert_eq!(part.status, 206);
    assert_eq!(part.body, APP_IMAGE[9..17]);
}

type UpdateCorruption = (&'static str, fn(&mut Value));

const UPDATE_CORRUPTIONS: [UpdateCorruption; 12] = [
    ("a manifest of another channel", |m| m["channel"] = json!("beta")),
    ("a rollout above one hundred", |m| m["rolloutPercent"] = json!(101)),
    ("a rollout below zero", |m| m["rolloutPercent"] = json!(-1)),
    ("a pre-release version", |m| m["version"] = json!("0.2.0-rc1")),
    ("no artifacts", |m| m["artifacts"] = json!([])),
    ("an unsigned artifact", |m| {
        m["artifacts"][0].as_object_mut().unwrap().remove("signature");
    }),
    ("a signature that is too short", |m| m["artifacts"][0]["signature"] = json!("RWQ=")),
    ("a digest that is not hex", |m| m["artifacts"][0]["sha256"] = json!("z".repeat(64))),
    ("a size the file does not have", |m| {
        m["artifacts"][0]["sizeBytes"] = json!(APP_IMAGE.len() + 1)
    }),
    ("a file that is not there", |m| m["artifacts"][0]["fileName"] = json!("gone.AppImage")),
    ("a file name with a current-directory prefix", |m| prefix_update_name(m, "./")),
    ("a file name that detours through a subdirectory", |m| prefix_update_name(m, "sub/../")),
];

fn prefix_update_name(manifest: &mut Value, prefix: &str) {
    let name = manifest["artifacts"][0]["fileName"].as_str().unwrap().to_string();
    manifest["artifacts"][0]["fileName"] = json!(format!("{prefix}{name}"));
}

#[tokio::test]
async fn a_manifest_that_cannot_be_trusted_fails_the_check_and_the_download() {
    for (defect, corrupt) in UPDATE_CORRUPTIONS {
        let server = Server::start().await;
        publish_update(server.folder(), "0.2.0", APP_IMAGE, "stable", 100, corrupt);
        let client = server.client();

        let asked = client.get(&check("0.1.9", 0)).await;
        let download = client.get("/api/desktop/update/download/stable/appimage").await;

        assert_eq!((asked.status.as_u16(), download.status.as_u16()), (503, 503), "{defect}");
        assert_eq!(asked.header("content-type"), Some("application/problem+json"));
        assert_eq!(
            asked.json::<Value>()["title"],
            "The desktop update is temporarily unavailable."
        );
    }
}

#[tokio::test]
async fn a_manifest_that_is_not_json_fails_the_check() {
    let server = Server::start().await;
    publish_stable(server.folder(), "0.2.0");
    std::fs::write(server.folder().join("releases/desktop/desktop-updater.json"), "not json").unwrap();

    assert_eq!(server.client().get(&check("0.1.9", 0)).await.status, 503);
}

#[tokio::test]
async fn a_signature_that_is_almost_too_long_is_still_one() {
    for (signature, expected) in [
        ("A".repeat(4096), 200),
        (format!("{}\n", "A".repeat(4096)), 200),
        ("A".repeat(4097), 503),
        ("A".repeat(63), 503),
        ("A".repeat(64), 200),
    ] {
        let server = Server::start().await;
        publish_update(server.folder(), "0.2.0", APP_IMAGE, "stable", 100, |manifest| {
            manifest["artifacts"][0]["signature"] = json!(signature)
        });

        let response = server.client().get(&check("0.1.9", 0)).await;

        assert_eq!(response.status, expected, "{}", signature.len());
    }
}

#[tokio::test]
async fn a_broken_artifact_of_another_platform_does_not_take_updates_away_from_linux() {
    let server = Server::start().await;
    publish_update(server.folder(), "0.2.0", APP_IMAGE, "stable", 100, |manifest| {
        manifest["artifacts"].as_array_mut().unwrap().insert(
            0,
            json!({
                "target": "windows", "architecture": "x86_64", "bundle": "appimage",
                "fileName": "missing.exe", "sizeBytes": 10, "sha256": "not-a-digest", "signature": SIGNATURE,
            }),
        );
        manifest["artifacts"].as_array_mut().unwrap().push(Value::Null);
    });
    let client = server.client();

    let linux = client.get(&check("0.1.9", 0)).await;
    let windows = client
        .get("/api/desktop/update/windows/x86_64/0.1.9?bundle=appimage&bucket=0")
        .await;

    assert_eq!(linux.status, 200);
    assert_eq!(windows.status, 503);
}

// ---------------------------------------------------------------------------
// Hostile requests: names, versions and ranges that are not what they say
// ---------------------------------------------------------------------------

/// Request targets that try to leave the folder, name something that is
/// not a channel, or break the encoding.
const HOSTILE_TARGETS: &[&str] = &[
    "/api/desktop/update/download/stable/..",
    "/api/desktop/update/download/stable/../appimage",
    "/api/desktop/update/download/../stable/appimage",
    "/api/desktop/update/download/stable/%2e%2e",
    "/api/desktop/update/download/..%2f..%2f..%2fetc/passwd",
    "/api/desktop/update/download/stable%2f..%2f..%2fetc%2fpasswd/appimage",
    "/api/desktop/update/download/stable/..%5c..%5cetc",
    "/api/desktop/update/download/stable/%2fetc%2fpasswd",
    "/api/desktop/update/download/stable/appimage%00",
    "/api/desktop/update/download/stable%00/appimage",
    "/api/desktop/update/download/stable/appimage%2f",
    "/api/desktop/update/download/stable//appimage",
    "/api/desktop/update/download//stable/appimage",
    "/api/desktop/update/download/%ff%fe/appimage",
    "/api/desktop/update/download/stable/%c0%af",
    "/api/desktop/update/download/stable/appimage?x=%zz",
    "/api/desktop/update/linux/x86_64/..",
    "/api/desktop/update/linux/x86_64/../0.1.9",
    "/api/desktop/update/linux/..%2f..%2fetc/0.1.9",
    "/api/desktop/update/..%2f..%2fetc%2fpasswd/x86_64/0.1.9",
    "/api/desktop/update/linux/x86_64/0.1.9%00",
    "/api/desktop/update/linux/x86_64/%00",
    "/api/desktop/update/linux/x86_64/0.1.9?channel=..%2f..%2fetc&bundle=appimage",
    "/api/desktop/update/linux/x86_64/0.1.9?channel=stable&bundle=..%2fappimage",
    "/api/desktop/update/linux/x86_64/0.1.9?channel=desktop-updater&bundle=appimage",
    "/api/desktop/update/linux/x86_64/0.1.9?channel=stable%00&bundle=appimage",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=%00",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=1%00",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=0x10",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=1e1",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=--1",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=%E0%A4%A6",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=%EF%BC%91",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=2147483647",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=2147483648",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=-2147483648",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=%20%205%20%20",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=%0B5",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=%005",
    "/api/desktop/update/linux/x86_64/0.1.9?&&bucket=3&&",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=3&bucket=",
    "/api/desktop/update/linux/x86_64/0.1.9?BUCKET=3&bucket=4",
    "/api/desktop/update/linux/x86_64/0.1.9?bundle=appimage&bundle=appimage&bucket=0",
    "/api/desktop/update/linux/x86_64/0.1.9?bundle=appimage&bundle=&bucket=0",
    "/api/desktop/update/linux/x86_64/0.1.9?bundle=&bundle=appimage&bucket=0",
    "/api/desktop/update/linux/x86_64/0.1.9?channel=stable&channel=stable&bucket=0",
    "/api/desktop/update/linux/x86_64/0.1.9?channel=stable&channel=&bucket=0",
    "/api/desktop/update/linux/x86_64/0.1.9?channel=&channel=stable&bucket=0",
    "/api/desktop/update/linux/x86_64/0.1.9?channel=stable%00&bucket=0",
    "/api/desktop/update/linux/x86_64/0.1.9?channel=sta%62le&bucket=0",
    "/api/desktop/update/linux/x86_64/0.1.9?Channel=STABLE&bucket=0",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=-0",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=%2B0",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=0000000000005",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=1_0",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=1,2",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=%FF",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=%2B%2B1",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=+-1",
    "/api/desktop/update/linux/x86_64/0.1.9?bucket=-+1",
    "/api/desktop/update/linux/x86_64/0.1.9?channel=%73table",
    "/api/desktop/update/linux/x86_64/0.1.9?channel=stable&bundle=appimage&bucket=0&channel=beta",
    "/api/desktop/update/linux/x86_64/1.0.0",
    "/api/desktop/update/linux/x86_64/0.0.0",
    "/api/desktop/update/linux/x86_64/0.1.9.0",
    "/api/desktop/update/linux/x86_64/0.1.9-",
    "/api/desktop/update/linux/x86_64/+0.1.9",
    "/api/desktop/update/linux/x86_64/-0.1.9",
    "/api/desktop/update/linux/x86_64/0.1.%39",
    "/api/desktop/update/linux/x86_64/%30.1.9",
    "/api/desktop/update/linux/x86_64/0.1.9%20",
    "/api/desktop/update/linux/x86_64/%09%0A0.1.9%0D",
    "/api/desktop/update/linux/x86_64/%C2%A00.1.9",
    "/api/desktop/update/linux/x86_64/%E2%80%830.1.9",
    "/api/desktop/update/linux/x86_64/%EF%BC%90.1.9",
    "/api/desktop/update/linux/x86_64/0.1.%D9%A9",
    "/api/desktop/update/linux/x86_64/999999999.0.0",
    "/api/desktop/update/linux/x86_64/1000000000.0.0",
    "/api/desktop/update/linux/x86_64/0.1.9/extra",
    "/api/desktop/update/linux/x86_64",
    "/api/desktop/update/linux",
    "/api/desktop/update/",
    "/api/desktop/update",
    "/api/desktop/update/linux//0.1.9",
    "/api/desktop/update/Linux/x86_64/0.1.9",
    "/api/desktop/release/arch/extra",
    "/api/desktop/release/arch%00",
    "/api/desktop/release/..",
    "/api/desktop/release/../android/release",
    "/api/desktop/release//arch",
    "/api/desktop/release/ARCH",
    "/api/desktop/release/arch?x=1",
    "/api/desktop/release;x=1",
    "/api/desktop/release%2farch",
    "/api/android/release/apk/",
    "/api/android/release/apk%2f",
    "/api/android/release/APK",
    "/api/android/release/../desktop/release",
    "/api/client/releases?x=%zz",
    "/api/client/releases/extra",
    "/api/client/releases%00",
    "/api/Client/Releases",
    "/.well-known/stellarr/client/extra",
    "/.well-known/stellarr/Client",
    "/.well-known/stellarr",
    "/.well-known/stellarr/client?x=1",
    "/.well-known/stellarr/client%00",
    "/.well-known/stellarr/%63lient",
];

/// Range headers that are not ranges, or are ranges of a file that is not
/// that long.
const HOSTILE_RANGES: &[&str] = &[
    "bytes=0-5", "bytes=0-5,", "bytes=,0-5", "bytes=0-5;", "bytes=-5-", "bytes=00-05", "bytes=+1-5",
    "bytes=1-+5", "bytes=", "bytes=-", "-5", "0-5", "bytes", "bytes=1", "bytes=1-2-3", "x=1-2", "=1-2",
    "by tes=1-2", "bytes=0-9223372036854775807", "bytes=0-9223372036854775808",
    "bytes=9223372036854775807-", "bytes=-9223372036854775807", "bytes=-9223372036854775808",
    "bytes=0-18446744073709551615", "bytes=18446744073709551616-", "bytes=-18446744073709551616",
    "bytes=5-5", "bytes=1027-1027", "bytes=1028-1028", "bytes=1027-", "bytes=1028-", "bytes=-1028",
    "bytes=-1029", "bytes=-1", "bytes=0-0", "bytes=0-", "bytes=0-5\t", "bytes=\t0-5", "bytes=0-5 ,",
    "bytes=1-2, 3-4", "bytes=1-2 3-4", "bytes=\"0-5\"", "bytes==0-5", "bytes=0-5=", "bytes:0-5",
    "bytes 0-5", " bytes=0-5", "bytes=0-5 ", "bytes= 0-5", "bytes =0-5", "bytes=0 -5", "bytes=0- 5",
    "BYTES=0-5", "Bytes=0-5", "bytes=0x5-9", "bytes=1e1-9", "bytes=1_0-9", "bytes=0-5-", "bytes=--5",
    "bytes=-+5", "bytes=\u{0}0-5", "bytes=0-5\u{0}", "", " ", ",", "bytes=0-1027,", "bytes=1000-1,2000-",
    "bytes=0-1,0-1", "bytes=0-4,\t5-9", "bytes=-1,-2", "units=0-5", "bytes=0-5, bytes=7-9", "bytes=\u{a0}0-5",
    "bytes=٠-5", "bytes=0-٥", "bytes=0-5;q=1", "bytes=0-5 6", "bytes=0 5", "bytes=0\t-\t5",
    "bytes=-0", "bytes=-00", "bytes=0000000000000000000000000-5", "bytes=0-00000000000000000000000005",
    "bytes=1028-1027", "bytes=1027-1026", "bytes=2-1", "bytes=0--1", "bytes=-1-0",
];

/// Names that a manifest might give a file, and that must never open one.
fn hostile_file_names() -> Vec<String> {
    let mut names: Vec<String> = [
        "..", ".", "../x", "/etc/passwd", "a/b", "./stellarr", "sub/../x", "..\\x", "\\x", "C:\\x", "x/", "/", "//", "a\u{0}b", "\u{0}",
        "x\u{0}", " ", "\t", "\u{a0}", "\u{2003}", "", " x", "x ", "...", "x..", "..x", "~", "~/x", "$HOME", "x%2fy", "x%00y", "x\u{202e}", "ü",
        "con", "nul", "aux", "x:y", "x*y", "x?y", "<x>", "|x", "x\"y", "x\ny", "x\ry",
    ]
    .iter()
    .map(|name| name.to_string())
    .collect();
    names.push("x".repeat(255));
    names.push("x".repeat(256));
    names.push("x".repeat(4096));
    names.push("x".repeat(5000));
    names.push(format!("{}/x", "d/".repeat(300)));
    names
}

const SECRET: &[u8] = b"SECRET-OUTSIDE-THE-RELEASE-FOLDER";

#[tokio::test]
async fn a_name_in_a_manifest_never_reaches_a_file_outside_its_folder() {
    let server = Server::start().await;
    let outside = server.folder().join("secret.bin");
    std::fs::write(&outside, SECRET).unwrap();
    let digest = "0".repeat(64);
    let mut names = hostile_file_names();
    names.extend(
        [
            "../secret.bin",
            "../../secret.bin",
            "../../../secret.bin",
            "desktop/../../secret.bin",
            "./../secret.bin",
            "..\\secret.bin",
            "%2e%2e/secret.bin",
            "%2e%2e%2fsecret.bin",
            "..%2fsecret.bin",
            "....//secret.bin",
            "releases/../secret.bin",
        ]
        .map(String::from),
    );
    names.push(outside.display().to_string());
    names.push(format!("{}\0", outside.display()));
    names.push(format!("file://{}", outside.display()));
    names.push(format!("{}/../secret.bin", server.folder().join("releases/desktop").display()));
    for name in &names {
        for platform in PLATFORMS {
            let (describe, download) = (platform.describe, platform.download);
            publish(server.folder(), platform, "0.4.2", |manifest| {
                manifest["artifact"]["fileName"] = json!(name);
                manifest["artifact"]["sizeBytes"] = json!(SECRET.len());
                manifest["artifact"]["sha256"] = json!(digest);
            });
            for path in [describe, download, "/api/client/releases"] {
                let response = server.client().get(path).await;
                assert!(
                    !response.body.windows(SECRET.len()).any(|window| window == SECRET),
                    "{name:?} gave {path} the file outside"
                );
                assert_ne!(response.status, 200, "{name:?} {path}");
                assert!(response.status == 503 || (path == "/api/client/releases" && response.status == 200), "{name:?} {path}: {}", response.status);
            }
        }
        publish_update(server.folder(), "0.2.0", APP_IMAGE, "stable", 100, |manifest| {
            manifest["artifacts"][0]["fileName"] = json!(name);
            manifest["artifacts"][0]["sizeBytes"] = json!(SECRET.len());
            manifest["artifacts"][0]["sha256"] = json!(digest);
        });
        for path in [check("0.1.9", 0), "/api/desktop/update/download/stable/appimage".to_string()] {
            let response = server.client().get(&path).await;
            assert!(
                !response.body.windows(SECRET.len()).any(|window| window == SECRET),
                "{name:?} gave {path} the file outside"
            );
            assert_eq!(response.status, 503, "{name:?} {path}");
        }
    }
}

#[tokio::test]
async fn a_symlink_the_operator_made_is_measured_and_served_as_the_file_it_points_at() {
    let server = Server::start().await;
    publish(server.folder(), &DESKTOP, "0.4.2", plain);
    let folder = server.folder().join("releases/desktop");
    let real = folder.join((DESKTOP.package_file)("0.4.2"));
    let elsewhere = server.folder().join("kept.bin");
    std::fs::rename(&real, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &real).unwrap();
    // The first server measured the link itself, so that a symlinked package
    // had to be declared with the length of the link's text. Here the size
    // that counts is the one of the file it points at, which is the one the
    // manifest declares.

    let linked = server.client().get(DESKTOP.download).await;
    std::fs::remove_file(&elsewhere).unwrap();
    let dangling = server.client().get(DESKTOP.download).await;

    assert_eq!((linked.status.as_u16(), &linked.body[..]), (200, PACKAGE));
    assert_eq!(dangling.status, 503);
}

#[tokio::test]
async fn hostile_requests_get_an_ordinary_answer_and_leave_the_server_standing() {
    let server = Server::start().await;
    publish(server.folder(), &DESKTOP, "0.4.2", plain);
    publish(server.folder(), &ANDROID, "0.4.2", plain);
    publish_stable(server.folder(), "0.2.0");
    let address = server.address();

    for target in HOSTILE_TARGETS {
        let answer = raw_get(address, target.as_bytes(), &[]).await;
        assert!(
            matches!(answer.status, 200 | 204 | 400 | 404 | 405 | 414),
            "{target}: {}",
            answer.status
        );
    }
    for range in HOSTILE_RANGES {
        for path in [DESKTOP.download, "/api/desktop/update/download/stable/appimage"] {
            let answer = raw_get(address, path.as_bytes(), &[("Range", range)]).await;
            match answer.status {
                200 => assert_eq!(answer.body.len(), PACKAGE.len().max(APP_IMAGE.len()).min(answer.body.len())),
                206 => {
                    let held = answer.header("content-range").unwrap();
                    let (span, total) = held.strip_prefix("bytes ").unwrap().split_once('/').unwrap();
                    let (first, last) = span.split_once('-').unwrap();
                    let (first, last, total): (usize, usize, usize) = (first.parse().unwrap(), last.parse().unwrap(), total.parse().unwrap());
                    assert!(first <= last && last < total, "{range:?}: {held}");
                    assert_eq!(answer.body.len(), last - first + 1, "{range:?}");
                }
                416 | 400 => assert!(answer.body.is_empty(), "{range:?}"),
                other => panic!("{range:?} of {path}: {other}"),
            }
        }
    }
    for (name, value) in [
        ("If-Modified-Since", "x".repeat(100_000)),
        ("If-Unmodified-Since", "\u{1}".to_string()),
        ("If-Range", "\"".repeat(1000)),
        ("Range", format!("bytes={}-", "9".repeat(100_000))),
        ("Range", format!("bytes=0-{}", "1,".repeat(50_000))),
    ] {
        let answer = raw_get(address, DESKTOP.download.as_bytes(), &[(name, &value)]).await;
        assert!(matches!(answer.status, 0 | 200 | 206 | 304 | 400 | 412 | 416 | 431), "{name}: {}", answer.status);
    }
    assert_eq!(server.client().get(DESKTOP.download).await.body, PACKAGE);
}

// ---------------------------------------------------------------------------
// A big download is streamed, not held
// ---------------------------------------------------------------------------

/// What the process of the server holds in memory, in KiB, as `ps` says.
fn resident_kib(pid: u32) -> u64 {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&output.stdout).trim().parse().unwrap_or(0)
}

/// Reads an answer to the end and counts its body without keeping it.
async fn discard_body(address: std::net::SocketAddr, target: &str, headers: &[(&str, &str)]) -> (u16, u64, Vec<(String, String)>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut request = format!("GET {target} HTTP/1.1\r\nHost: big.test\r\nConnection: close\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.unwrap();
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let status = text.split(' ').nth(1).unwrap().parse().unwrap();
    let headers = text
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    let mut counted = 0u64;
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = stream.read(&mut buffer).await.unwrap();
        if read == 0 {
            return (status, counted, headers);
        }
        counted += read as u64;
    }
}

/// Moves 256 MiB through the server and watches how much of it the process
/// holds. The file is sparse, so that it takes next to nothing of the disk.
#[tokio::test]
async fn a_download_of_hundreds_of_megabytes_is_streamed_and_not_held_in_memory() {
    const LENGTH: u64 = 256 * 1024 * 1024;
    let server = Server::start().await;
    let pid = server.pid().expect("the binary");
    let directory = server.folder().join("releases/desktop");
    std::fs::create_dir_all(&directory).unwrap();
    let file_name = "stellarr-desktop-0.4.2-1-x86_64.pkg.tar.zst";
    let file = std::fs::File::create(directory.join(file_name)).unwrap();
    file.set_len(LENGTH).unwrap();
    // The last bytes are not zeros, so that a range there is told from a miss.
    {
        use std::io::{Seek, SeekFrom, Write};
        let mut file = &file;
        file.seek(SeekFrom::Start(LENGTH - 8)).unwrap();
        file.write_all(b"THE-END!").unwrap();
    }
    drop(file);
    std::fs::write(
        directory.join("desktop-release.json"),
        json!({
            "schemaVersion": 1, "version": "0.4.2", "publishedAt": now_text(), "notes": "big",
            "artifact": { "platform": "arch", "architecture": "x86_64", "fileName": file_name,
                          "sizeBytes": LENGTH, "sha256": "0".repeat(64) },
        })
        .to_string(),
    )
    .unwrap();
    // The server has started and answered once, so that what it holds is what it holds at rest.
    assert_eq!(server.client().get("/api/desktop/release").await.status, 200);
    let before = resident_kib(pid);

    let watching = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(before));
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sampler = {
        let (watching, done) = (watching.clone(), done.clone());
        tokio::task::spawn_blocking(move || {
            while !done.load(std::sync::atomic::Ordering::Relaxed) {
                watching.fetch_max(resident_kib(pid), std::sync::atomic::Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        })
    };
    let (status, counted, headers) = discard_body(server.address(), DESKTOP.download, &[]).await;
    let during = watching.load(std::sync::atomic::Ordering::Relaxed);
    let (part_status, part, part_headers) =
        discard_body(server.address(), DESKTOP.download, &[("Range", "bytes=134217728-")]).await;
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    sampler.await.unwrap();
    let peak = watching.load(std::sync::atomic::Ordering::Relaxed);

    println!("the server held {before} KiB at rest and {during} KiB at the most while {counted} bytes went out ({peak} KiB at the most with a range after)");
    assert_eq!((status, counted), (200, LENGTH));
    assert_eq!(headers.iter().find(|(name, _)| name == "content-length").map(|(_, v)| v.as_str()), Some(LENGTH.to_string().as_str()));
    assert_eq!((part_status, part), (206, LENGTH / 2));
    assert_eq!(
        part_headers.iter().find(|(name, _)| name == "content-range").map(|(_, v)| v.as_str()),
        Some(format!("bytes 134217728-{}/{LENGTH}", LENGTH - 1).as_str())
    );
    // A file held whole would add 256 MiB.
    assert!(peak < before + 48 * 1024, "{before} KiB at rest, {peak} KiB at the most");
    let tail = server
        .client()
        .send(conditional(DESKTOP.download, &[("Range", "bytes=-8")]))
        .await;
    assert_eq!((tail.status.as_u16(), &tail.body[..]), (206, b"THE-END!".as_slice()));
}

#[tokio::test]
async fn a_client_that_hangs_up_in_the_middle_of_a_download_does_not_hold_the_server() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const LENGTH: u64 = 64 * 1024 * 1024;
    let server = Server::start().await;
    let directory = server.folder().join("releases/desktop");
    std::fs::create_dir_all(&directory).unwrap();
    let file_name = "stellarr-desktop-0.4.2-1-x86_64.pkg.tar.zst";
    std::fs::File::create(directory.join(file_name)).unwrap().set_len(LENGTH).unwrap();
    std::fs::write(
        directory.join("desktop-release.json"),
        json!({
            "schemaVersion": 1, "version": "0.4.2", "publishedAt": now_text(), "notes": "big",
            "artifact": { "platform": "arch", "architecture": "x86_64", "fileName": file_name,
                          "sizeBytes": LENGTH, "sha256": "0".repeat(64) },
        })
        .to_string(),
    )
    .unwrap();

    for _ in 0..20 {
        let mut stream = tokio::net::TcpStream::connect(server.address()).await.unwrap();
        stream
            .write_all(format!("GET {} HTTP/1.1\r\nHost: x\r\n\r\n", DESKTOP.download).as_bytes())
            .await
            .unwrap();
        let mut some = [0u8; 4096];
        stream.read_exact(&mut some).await.unwrap();
        drop(stream);
    }

    // Each of them left a task that has to end by itself.
    let mut open = u64::MAX;
    for _ in 0..100 {
        let listing = std::process::Command::new("lsof")
            .args(["-p", &server.pid().unwrap().to_string(), "-a", "-d", "0-1023"])
            .output();
        open = listing.map_or(0, |out| String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.contains(file_name)).count() as u64);
        if open == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(open, 0, "files of abandoned downloads stay open");
    assert_eq!(server.client().get("/api/desktop/release").await.status, 200);
}
