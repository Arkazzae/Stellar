//! The avatar, banner and wallpaper of an account, and the icon of a server:
//! setting, replacing and removing them, and what that leaves behind.

use http::Method;
use rusqlite::params;
use serde_json::Value;

use crate::support::{Request, Server};
use crate::uploads::{Form, JPEG, PDF, PNG, id_of, image, objects, started, upload};

fn named(server: &Server, name: &str) -> Vec<Value> {
    server
        .events()
        .into_iter()
        .filter(|event| event["name"] == name)
        .collect()
}

fn count(server: &Server, sql: &str) -> i64 {
    server.database().query_row(sql, [], |row| row.get(0)).unwrap()
}

#[tokio::test]
async fn a_picture_is_set_replaced_and_removed_on_the_own_account() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    let first = maja.client.send(image("avatar.png").post("/api/me/images/avatar")).await;

    assert_eq!(first.status, 201, "{}", first.text());
    let account: Value = first.json();
    let url = account["avatarUrl"].as_str().unwrap().to_string();
    assert!(url.starts_with("/assets/"));
    assert_eq!(first.header("location"), Some(url.as_str()));
    assert_eq!(account["username"], "maja");
    assert_eq!(maja.client.get(&url).await.status, 200);
    let key = objects(&server).remove(0);
    let user = maja.id.replace('-', "");
    assert!(key.starts_with(&format!("profiles/{user}/avatar/")) && key.ends_with(".png"), "{key}");
    let name: String = server
        .database()
        .query_row("SELECT \"FileName\" FROM \"ImageAttachments\"", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "avatar.png", "the name as sent, not the sanitized one");

    // A second picture takes the place of the first, and the first goes.
    let second = maja
        .client
        .send(Form::new().file("file", "../odd name.jpg", "image/jpeg", JPEG).post("/api/me/images/avatar"))
        .await;
    let replaced = second.json::<Value>()["avatarUrl"].as_str().unwrap().to_string();
    assert_ne!(replaced, url);
    assert_eq!(maja.client.get(&url).await.status, 404);
    assert_eq!(maja.client.get(&replaced).await.status, 200);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ImageAttachments\""), 1);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ObjectDeletions\""), 1);
    let name: String = server
        .database()
        .query_row("SELECT \"FileName\" FROM \"ImageAttachments\"", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "odd name.jpg");

    let removed = maja.client.send(Request::new(Method::DELETE, "/api/me/images/avatar")).await;
    assert_eq!(removed.status, 200);
    assert_eq!(removed.json::<Value>()["avatarUrl"], Value::Null);
    assert_eq!(maja.client.get(&replaced).await.status, 404);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ImageAttachments\""), 0);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ObjectDeletions\""), 2);
    let again = maja.client.send(Request::new(Method::DELETE, "/api/me/images/avatar")).await;
    assert_eq!(again.status, 200);
}

#[tokio::test]
async fn the_kind_is_trimmed_and_not_case_sensitive_and_an_unknown_one_does_not_exist() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    for kind in ["BANNER", "%20banner%20", "Wallpaper"] {
        let response = maja.client.send(image("a.png").post(&format!("/api/me/images/{kind}"))).await;
        assert_eq!(response.status, 201, "{kind}");
    }
    let unknown = maja.client.send(image("a.png").post("/api/me/images/cover")).await;
    let unknown_delete = maja.client.send(Request::new(Method::DELETE, "/api/me/images/cover")).await;
    let account = maja.client.get("/api/me").await.json::<Value>();

    assert_eq!((unknown.status.as_u16(), unknown_delete.status.as_u16()), (404, 404));
    assert!(account["bannerUrl"].is_string() && account["wallpaperUrl"].is_string());
    assert_eq!(account["avatarUrl"], Value::Null);
    // The form is read before the kind is looked at.
    let no_file = maja.client.send(Form::new().text("x", "1").post("/api/me/images/cover")).await;
    assert_eq!(no_file.status, 400);
}

#[tokio::test]
async fn a_picture_is_an_image_of_a_known_type_within_the_limit_of_its_kind() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let big = |size: usize| [PNG, &vec![b'x'; size - PNG.len()][..]].concat();
    let send = |kind: &'static str, content_type: &'static str, bytes: Vec<u8>| {
        let client = &maja.client;
        async move { client.send(Form::new().file("file", "a", content_type, &bytes).post(&format!("/api/me/images/{kind}"))).await }
    };

    let at_avatar = send("avatar", "image/png", big(4 * 1024 * 1024)).await;
    let over_avatar = send("avatar", "image/png", big(4 * 1024 * 1024 + 1)).await;
    let at_banner = send("banner", "image/png", big(8 * 1024 * 1024)).await;
    let over_banner = send("banner", "image/png", big(8 * 1024 * 1024 + 1)).await;
    let over_wallpaper = send("wallpaper", "image/png", big(8 * 1024 * 1024 + 1)).await;
    let pdf = send("avatar", "application/pdf", PDF.to_vec()).await;
    let forged = send("banner", "image/png", b"not a png".to_vec()).await;
    let empty = send("avatar", "image/png", Vec::new()).await;

    assert_eq!((at_avatar.status.as_u16(), at_banner.status.as_u16()), (201, 201));
    for (refused, status, title, detail) in [
        (&over_avatar, 413, "The image is too large", "An avatar can be at most 4 MB."),
        (&over_banner, 413, "The image is too large", "A banner can be at most 8 MB."),
        (&over_wallpaper, 413, "The image is too large", ""),
        (&pdf, 415, "", "Only JPEG, PNG, GIF and WebP images are allowed."),
        (&forged, 415, "", "The content of the file does not match its declared type."),
    ] {
        assert_eq!(refused.status.as_u16(), status);
        let problem: Value = refused.json();
        if !title.is_empty() {
            assert_eq!(problem["title"], title);
        }
        if !detail.is_empty() {
            assert_eq!(problem["detail"], detail);
        }
    }
    assert_eq!(empty.status, 400);
    assert!(empty.json::<Value>()["errors"]["file"].is_array());
}

#[tokio::test]
async fn a_picture_that_something_still_uses_is_kept_when_it_is_replaced() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let first = maja.client.send(image("a.png").post("/api/me/images/avatar")).await;
    let url = first.json::<Value>()["avatarUrl"].as_str().unwrap().to_string();
    // The same picture is also the banner: something still points at it.
    server
        .database()
        .execute("UPDATE \"Users\" SET \"BannerUrl\" = ?1", params![url])
        .unwrap();

    maja.client.send(image("b.png").post("/api/me/images/avatar")).await;

    assert_eq!(maja.client.get(&url).await.status, 200);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ImageAttachments\""), 2);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ObjectDeletions\""), 0);
}

#[tokio::test]
async fn only_what_the_account_uploaded_itself_is_retired() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let sam = server.register("sam").await;
    let theirs = id_of(&upload(&sam, image("a.png")).await);
    // An address that is somebody else's file stays where it is.
    server
        .database()
        .execute("UPDATE \"Users\" SET \"AvatarUrl\" = ?1 WHERE \"Username\" = 'maja'", params![format!("/assets/{theirs}")])
        .unwrap();

    maja.client.send(image("b.png").post("/api/me/images/avatar")).await;

    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ImageAttachments\""), 2);
    assert_eq!(sam.client.get(&format!("/assets/{theirs}")).await.status, 200);
}

#[tokio::test]
async fn an_avatar_and_a_banner_are_announced_to_everybody_and_a_wallpaper_to_nobody() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    maja.client.send(image("w.png").post("/api/me/images/wallpaper")).await;
    maja.client.send(Request::new(Method::DELETE, "/api/me/images/wallpaper")).await;
    assert!(named(&server, "UserUpdated").is_empty());

    maja.client.send(image("a.png").post("/api/me/images/avatar")).await;
    maja.client.send(Request::new(Method::DELETE, "/api/me/images/avatar")).await;
    maja.client.send(image("b.png").post("/api/me/images/banner")).await;

    let announced = named(&server, "UserUpdated");
    // Each change is told twice: to everybody with the status they may see,
    // and to the person with their own.
    assert_eq!(announced.len(), 6);
    assert_eq!(announced[0]["audience"], "everyone");
    assert_eq!(announced[1]["audience"]["users"][0], maja.id);
    assert!(announced[0]["payload"]["avatarUrl"].is_string());
    assert_eq!(announced[2]["payload"]["avatarUrl"], Value::Null);
    assert!(announced[4]["payload"]["bannerUrl"].is_string());
    assert!(announced[0]["payload"].get("wallpaperUrl").is_none());
}

#[tokio::test]
async fn a_picture_counts_against_the_storage_limits_of_ordinary_uploads() {
    let server = started(&[("Uploads__MaxStoredBytesPerUser", "20")]).await;
    let maja = server.found("maja").await;

    let fits = maja.client.send(image("a.png").post("/api/me/images/avatar")).await;
    let over = maja.client.send(image("b.png").post("/api/me/images/banner")).await;

    assert_eq!(fits.status, 201);
    assert_eq!(over.status, 507);
    assert_eq!(over.json::<Value>()["detail"], "The storage limit of this account has been used up.");
    assert_eq!(objects(&server).len(), 1);
}

#[tokio::test]
async fn pictures_need_a_signed_in_person() {
    let server = started(&[]).await;
    server.found("maja").await;

    let set = server.client().send(image("a.png").post("/api/me/images/avatar")).await;
    let remove = server.client().send(Request::new(Method::DELETE, "/api/me/images/avatar")).await;

    assert_eq!((set.status.as_u16(), remove.status.as_u16()), (401, 401));
}

#[tokio::test]
async fn the_owner_of_a_server_sets_replaces_and_removes_its_icon() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    let first = maja.client.send(image("icon.png").post("/api/admin/server/icon")).await;

    assert_eq!(first.status, 201, "{}", first.text());
    let url = first.json::<Value>()["iconUrl"].as_str().unwrap().to_string();
    assert_eq!(first.header("location"), Some(url.as_str()));
    assert_eq!(maja.client.get(&url).await.status, 200);
    let key = objects(&server).remove(0);
    assert!(key.starts_with("servers/") && key.contains("/icon/") && key.ends_with(".png"), "{key}");
    assert_eq!(audit(&server), ["server.icon"]);
    let told = named(&server, "ServerUpdated");
    assert_eq!(told.len(), 1);
    assert_eq!(told[0]["payload"]["iconUrl"], url);
    assert_eq!(told[0]["audience"]["server"], first_server_id(&server));
    let members = ["id", "name", "description", "iconUrl", "createdAt", "isPublic"];
    assert_eq!(
        told[0]["payload"].as_object().unwrap().len(),
        members.len(),
        "{}",
        told[0]["payload"]
    );

    let second = maja.client.send(image("other.png").post("/api/admin/server/icon")).await;
    let replaced = second.json::<Value>()["iconUrl"].as_str().unwrap().to_string();
    assert_eq!(maja.client.get(&url).await.status, 404, "the old icon is retired");
    assert_eq!(maja.client.get(&replaced).await.status, 200);

    let removed = maja.client.send(Request::new(Method::DELETE, "/api/admin/server/icon")).await;
    assert_eq!(removed.status, 200);
    assert_eq!(removed.json::<Value>()["iconUrl"], Value::Null);
    assert_eq!(maja.client.get(&replaced).await.status, 404);
    let again = maja.client.send(Request::new(Method::DELETE, "/api/admin/server/icon")).await;
    assert_eq!(again.status, 200);
    assert_eq!(audit(&server), ["server.icon", "server.icon", "server.icon.remove"]);
    assert_eq!(named(&server, "ServerUpdated").len(), 4, "a removal that removes nothing is announced too");
}

fn audit(server: &Server) -> Vec<String> {
    let db = server.database();
    let mut statement = db
        .prepare("SELECT \"Action\" FROM \"ServerAuditEntries\" ORDER BY \"CreatedAt\"")
        .unwrap();
    statement.query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect()
}

fn first_server_id(server: &Server) -> String {
    crate::uploads::first_server(&server.database()).to_string()
}

#[tokio::test]
async fn an_icon_is_a_small_image_and_is_the_business_of_those_who_manage_the_server() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let sam = server.register("sam").await;
    let big = [PNG, &vec![b'x'; 4 * 1024 * 1024 - PNG.len() + 1][..]].concat();

    let over = maja.client.send(Form::new().file("file", "i.png", "image/png", &big).post("/api/admin/server/icon")).await;
    let pdf = maja.client.send(Form::new().file("file", "i.pdf", "application/pdf", PDF).post("/api/admin/server/icon")).await;
    let forged = maja.client.send(Form::new().file("file", "i.png", "image/png", b"nope").post("/api/admin/server/icon")).await;
    let by_a_member = sam.client.send(image("i.png").post("/api/admin/server/icon")).await;
    let remove_by_a_member = sam.client.send(Request::new(Method::DELETE, "/api/admin/server/icon")).await;
    let anonymous = server.client().send(image("i.png").post("/api/admin/server/icon")).await;

    assert_eq!(over.status, 413);
    assert_eq!(over.json::<Value>()["detail"], "A server icon can be at most 4 MB.");
    assert_eq!(pdf.status, 415);
    assert_eq!(forged.status, 415);
    assert_eq!(
        (by_a_member.status.as_u16(), remove_by_a_member.status.as_u16(), anonymous.status.as_u16()),
        (403, 403, 401)
    );
    assert!(objects(&server).is_empty());
    assert!(audit(&server).is_empty());
    assert!(named(&server, "ServerUpdated").is_empty());
}
