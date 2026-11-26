//! The document a native client reads before it signs in, to learn which
//! instance it is talking to and whether the two are compatible.

use std::fs;
use std::io;
use std::path::Path;

use axum::extract::State;
use axum::http::HeaderMap;
use serde::Serialize;

use super::catalogue;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, ClientInfo, Json};
use crate::state::AppState;
use crate::values::Id;

const FILE_NAME: &str = "instance-id";
const API_PROTOCOL: u32 = 1;
const RTC_PROTOCOL: u32 = 1;
const FALLBACK_INSTANCE_NAME: &str = "Stellarr";
const SIGN_IN_METHODS: [&str; 2] = ["password", "invite"];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    product: &'static str,
    instance_id: Id,
    instance_name: String,
    canonical_origin: String,
    server_version: &'static str,
    build_sha: Option<String>,
    api_protocol: u32,
    api_protocol_min: u32,
    api_protocol_max: u32,
    rtc_protocol: u32,
    auth: [&'static str; 2],
    desktop: DesktopVersions,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopVersions {
    minimum_version: String,
    recommended_version: String,
}

/// Public installation identity for this origin, shared by all its servers.
pub async fn manifest(State(state): State<AppState>) -> ApiResult<axum::response::Response> {
    use axum::http::{HeaderValue, StatusCode, header};
    use rusqlite::OptionalExtension;

    let name = state.database().read(|db| -> Result<Option<String>, DbError> {
        Ok(db.query_row(r#"SELECT "Name" FROM "Servers" ORDER BY "CreatedAt", "Id" LIMIT 1"#,
            [], |row| row.get(0)).optional()?)
    }).await?;
    let name = match name.as_deref() {
        None | Some("") | Some("Stellarr") => "Stellarr".to_string(),
        Some(instance) => format!("Stellarr · {instance}"),
    };
    let (inbox, messages) = match state.messages().language() {
        stellarr_messages::Language::Polish => ("Powiadomienia", "Wiadomości"),
        _ => ("Notifications", "Messages"),
    };
    let manifest = serde_json::json!({
        "id": "/", "name": name, "short_name": name, "start_url": "/", "scope": "/",
        "display": "standalone", "background_color": "#111111", "theme_color": "#111111",
        "shortcuts": [
            {"name": messages, "url": "/?view=messages"},
            {"name": inbox, "url": "/?view=inbox"}
        ],
        "icons": [
            {"src": "/pwa/icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any"},
            {"src": "/pwa/icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any"},
            {"src": "/pwa/maskable-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable"}
        ]
    });
    let mut response = crate::http::json_response(StatusCode::OK, &manifest);
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/manifest+json"));
    Ok(response)
}

pub async fn describe(
    State(state): State<AppState>,
    headers: HeaderMap,
    client: Option<axum::Extension<ClientInfo>>,
) -> ApiResult<Json<Document>> {
    let settings = state.settings();
    let (found, saved) = state
        .database()
        .read(|db| -> Result<_, DbError> {
            let first = db
                .prepare_cached(
                    "SELECT \"Id\", \"Name\" FROM \"Servers\" ORDER BY \"CreatedAt\", \"Id\" LIMIT 1",
                )?
                .query_map([], |row| Ok((row.get::<_, Id>(0)?, row.get::<_, String>(1)?)))?
                .next()
                .transpose()?;
            let saved = match &first {
                None => None,
                Some((server, _)) => db
                    .prepare_cached(
                        "SELECT \"DesktopMinimumVersion\", \"DesktopRecommendedVersion\" \
                         FROM \"OperationsSettings\" WHERE \"ServerId\" = ?1",
                    )?
                    .query_map([server], |row| {
                        Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?))
                    })?
                    .next()
                    .transpose()?,
            };
            Ok((first, saved))
        })
        .await?;
    let instance_id = match settings.discovery.instance_id {
        Some(id) => id,
        None => {
            let data_root = settings.data_root.clone();
            tokio::task::spawn_blocking(move || instance_id(&data_root))
                .await
                .map_err(ApiError::internal)?
                .map_err(ApiError::internal)?
        }
    };
    let (saved_minimum, saved_recommended) = saved.unwrap_or((None, None));
    let desktop = &settings.desktop;
    Ok(Json(Document {
        product: "stellarr",
        instance_id,
        instance_name: found
            .map_or_else(|| FALLBACK_INSTANCE_NAME.to_string(), |(_, name)| name),
        canonical_origin: if desktop.canonical_origin.is_empty() {
            let (scheme, host) = super::requested_origin(&headers, client.as_ref().map(|c| &c.0));
            format!("{scheme}://{host}")
        } else {
            desktop.canonical_origin.clone()
        },
        server_version: crate::version::server(),
        build_sha: settings.discovery.build_sha.clone(),
        api_protocol: API_PROTOCOL,
        api_protocol_min: API_PROTOCOL,
        api_protocol_max: API_PROTOCOL,
        rtc_protocol: RTC_PROTOCOL,
        auth: SIGN_IN_METHODS,
        desktop: DesktopVersions {
            minimum_version: catalogue::desktop_version(
                saved_minimum.as_deref(),
                desktop.minimum_version.as_deref(),
            ),
            recommended_version: catalogue::desktop_version(
                saved_recommended.as_deref(),
                desktop.recommended_version.as_deref(),
            ),
        },
    }))
}

/// The id of this instance, kept in the data folder so that it survives
/// restarts and new containers. Made on the first call, and when two
/// processes start together the one that arrives second adopts the id of the
/// first. It reads the file each time: it is a few bytes, and nothing has to
/// be remembered between requests.
pub fn instance_id(data_root: &Path) -> io::Result<Id> {
    let path = data_root.join(FILE_NAME);
    match fs::read_to_string(&path) {
        Ok(text) => return stored(&path, &text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::create_dir_all(data_root)?;
    let id = Id::random();
    // The file appears complete or not at all: written beside, then linked
    // into place, which fails when another process was first.
    let temporary = data_root.join(format!("{FILE_NAME}.{}.tmp", id.to_string().replace('-', "")));
    fs::write(&temporary, id.to_string())?;
    let linked = fs::hard_link(&temporary, &path);
    let _ = fs::remove_file(&temporary);
    match linked {
        Ok(()) => Ok(id),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            stored(&path, &fs::read_to_string(&path)?)
        }
        Err(error) => Err(error),
    }
}

/// The id a file holds. A file an operator wrote ends in a line break, which
/// is not part of the id.
fn stored(path: &Path, text: &str) -> io::Result<Id> {
    Id::parse(text.trim_end()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} does not contain a valid instance UUID.", path.display()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn the_id_is_made_once_and_kept() {
        let folder = TempDir::new();
        let root = folder.path().join("not").join("there").join("yet");

        let first = instance_id(&root).unwrap();
        let again = instance_id(&root).unwrap();

        assert_eq!(first, again);
        assert_eq!(fs::read_to_string(root.join("instance-id")).unwrap(), first.to_string());
        let left: Vec<_> = fs::read_dir(&root).unwrap().collect();
        assert_eq!(left.len(), 1, "no temporary file stays behind");
    }

    #[test]
    fn an_id_already_on_the_volume_is_the_one_used() {
        let folder = TempDir::new();
        fs::write(
            folder.path().join("instance-id"),
            "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE\n",
        )
        .unwrap();

        assert_eq!(
            instance_id(folder.path()).unwrap().to_string(),
            "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"
        );
    }

    #[test]
    fn a_file_that_holds_no_uuid_is_an_error_and_is_left_alone() {
        let folder = TempDir::new();
        fs::write(folder.path().join("instance-id"), "no id here").unwrap();

        let error = instance_id(folder.path()).unwrap_err();

        assert!(error.to_string().ends_with("does not contain a valid instance UUID."), "{error}");
        assert_eq!(fs::read_to_string(folder.path().join("instance-id")).unwrap(), "no id here");
    }

    #[test]
    fn processes_that_start_together_agree_on_one_id() {
        let folder = TempDir::new();
        let root = folder.path().to_path_buf();

        let ids: Vec<Id> = (0..16)
            .map(|_| {
                let root = root.clone();
                std::thread::spawn(move || instance_id(&root).unwrap())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();

        assert!(ids.iter().all(|id| *id == ids[0]));
    }
}
