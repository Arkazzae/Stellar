use super::{Installed, LaunchMode, authority::Authority, hostname, invalid, library};
use crate::{
    auth::{Caller, session},
    encoding::hex_lower,
    http::{ApiError, ApiResult, Json, Path},
    instance::users::User,
    state::AppState,
    values::{Id, Timestamp},
};
use aws_lc_rs::digest::{SHA256, digest};
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use bytes::Bytes;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub(super) enum Scope {
    Voice(Id),
    Personal(Id),
}
impl Scope {
    pub(super) fn from_path(value: &str, user: Id) -> ApiResult<Self> {
        if value == "personal" {
            Ok(Self::Personal(user))
        } else {
            Id::parse(value)
                .map(Self::Voice)
                .ok_or_else(ApiError::not_found)
        }
    }
    pub(super) fn mode(self) -> LaunchMode {
        match self {
            Self::Voice(_) => LaunchMode::Voice,
            Self::Personal(_) => LaunchMode::Personal,
        }
    }
    fn segment(self) -> String {
        match self {
            Self::Voice(channel) => channel.to_string(),
            Self::Personal(_) => "personal".into(),
        }
    }
    fn place(self) -> Value {
        match self {
            Self::Voice(channel) => json!({"id":channel}),
            Self::Personal(_) => Value::Null,
        }
    }
}
type Place = (Id, Scope);
#[derive(Default)]
pub(crate) struct Sessions {
    pub(super) changes: tokio::sync::RwLock<()>,
    rooms: Mutex<HashMap<Place, Live>>,
}
struct Lease {
    user: Id,
    account_session: Id,
    account_version: i64,
    token: Id,
    ticket: Option<Id>,
    cookie: Id,
    touched: Instant,
}
/// A single wildcard-TLS label identifies the ZIP and its installation. Keeping
/// the version in the origin lets absolute and relative URLs share the ZIP root.
fn asset_label(installed: &Installed) -> String {
    let identity = format!("{}/{}", installed.install, installed.digest);
    let hash = digest(&SHA256, identity.as_bytes());
    hex_lower(&hash.as_ref()[..24])
}

struct Live {
    asset_label: String,
    installed: Installed,
    authority: Authority,
    leases: HashMap<Id, Lease>,
    pictures: HashMap<Id, (Instant, String)>,
    used: Instant,
}
impl Sessions {
    pub(super) fn pinned_digest(&self, id: Id, channel: Scope) -> Option<String> {
        self.rooms
            .lock()
            .unwrap()
            .get(&(id, channel))
            .map(|live| live.installed.digest.clone())
    }
    pub fn remove(&self, id: Id) {
        self.rooms
            .lock()
            .unwrap()
            .retain(|(installed, _), _| *installed != id);
    }
}
pub(super) fn config(state: &AppState) -> ApiResult<(String, String)> {
    let domain = state.settings().activity_domain.clone().unwrap_or_default();
    let origin = state
        .settings()
        .activity_origin
        .clone()
        .unwrap_or_else(|| state.settings().desktop.canonical_origin.clone());
    let url = stellarr_outbound::Url::parse(&origin).map_err(|_| {
        invalid("Configure the HTTPS instance origin and a separate activity domain.")
    })?;
    if !hostname(&domain)
        || !url.is_https()
        || origin != format!("https://{}", url.host_header())
        || url.host() == domain
        || url.host().ends_with(&format!(".{domain}"))
    {
        return Err(invalid(
            "Configure independent HTTPS instance and activity hostnames.",
        ));
    }
    Ok((origin, domain))
}
fn seated(state: &AppState, scope: Scope, user: Id) -> bool {
    let Scope::Voice(channel) = scope else {
        return scope == Scope::Personal(user);
    };
    state.realtime().is_some_and(|hub| {
        !hub.presence()
            .voice_connections_of(user, channel)
            .is_empty()
    })
}
pub(super) fn check(state: &AppState, channel: Scope, user: Id) -> ApiResult<()> {
    if seated(state, channel, user) {
        Ok(())
    } else {
        Err(ApiError::forbidden())
    }
}
fn approved_origin(headers: &HeaderMap, origin: &str) -> ApiResult<()> {
    if headers.get(header::ORIGIN).and_then(|h| h.to_str().ok()) == Some(origin) {
        Ok(())
    } else {
        Err(ApiError::forbidden())
    }
}
fn prune(rooms: &mut HashMap<Place, Live>, state: &AppState) {
    rooms.retain(|(_, channel), live| {
        let departed: Vec<Id> = live
            .leases
            .iter()
            .filter_map(|(id, lease)| {
                (lease.touched.elapsed() > Duration::from_secs(60) || !seated(state, *channel, *id))
                    .then_some(*id)
            })
            .collect();
        for id in departed {
            live.leases.remove(&id);
            live.authority.revoke(id);
            live.pictures.remove(&id);
        }
        live.used.elapsed() < Duration::from_secs(180) && !live.authority.expired()
    })
}
#[derive(Deserialize)]
pub(super) struct Device {
    device: String,
}
pub(super) async fn available(
    State(state): State<AppState>,
    caller: Caller,
    Path(path): Path<String>,
    headers: HeaderMap,
    Json(input): Json<Device>,
) -> ApiResult<Json<Value>> {
    let channel = Scope::from_path(&path, caller.user)?;
    let Ok((origin, _)) = config(&state) else {
        return Ok(Json(json!([])));
    };
    approved_origin(&headers, &origin)?;
    check(&state, channel, caller.user)?;
    let _gate = state.activities().changes.read().await;
    let all = library(&state).await?;
    let mut rooms = state.activities().rooms.lock().unwrap();
    prune(&mut rooms, &state);
    let list:Vec<_>=all.into_iter().filter(|entry|entry.enabled).filter_map(|entry|{
        let live=rooms.get(&(entry.install,channel));
        let current=live.map_or(&entry,|room|&room.installed);
        if !current.manifest.launch_modes.contains(&channel.mode()){ return None; }
        let artwork=super::artwork::urls(current,&format!("/api/activities/{}/art/{}",channel.segment(),entry.install));
        Some(json!({"install":entry.install,"name":current.manifest.name,"version":current.manifest.version,"running":live.is_some(),
               "description":current.manifest.description,"artwork":artwork,
               "supported":current.manifest.supports(&input.device),"devices":current.manifest.devices}))
    }).collect();
    Ok(Json(json!(list)))
}
#[derive(Deserialize)]
pub(super) struct Launch {
    install: Id,
    device: String,
}
pub(super) async fn launch(
    State(state): State<AppState>,
    caller: Caller,
    Path(path): Path<String>,
    headers: HeaderMap,
    Json(input): Json<Launch>,
) -> ApiResult<Json<Value>> {
    let channel = Scope::from_path(&path, caller.user)?;
    let (origin, domain) = config(&state)?;
    approved_origin(&headers, &origin)?;
    let _gate = state.activities().changes.read().await;
    let current = library(&state)
        .await?
        .into_iter()
        .find(|a| a.install == input.install && a.enabled)
        .ok_or_else(ApiError::not_found)?;
    check(&state, channel, caller.user)?;
    let user = state
        .database()
        .read(move |db| User::find(db, caller.user))
        .await?
        .ok_or_else(ApiError::forbidden)?;
    let mut rooms = state.activities().rooms.lock().unwrap();
    prune(&mut rooms, &state);
    let key = (input.install, channel);
    let pin = rooms.get(&key).map_or(&current, |room| &room.installed);
    if !pin.manifest.launch_modes.contains(&channel.mode()) {
        return Err(ApiError::forbidden());
    }
    if !rooms.contains_key(&key) {
        if !current.manifest.supports(&input.device) {
            return Err(invalid("This release does not support this device."));
        }
        if rooms.len() >= 64 {
            return Err(ApiError::status(StatusCode::TOO_MANY_REQUESTS));
        }
        rooms.insert(
            key,
            Live {
                authority: Authority::new(&current.manifest)?,
                asset_label: asset_label(&current),
                installed: current,
                leases: HashMap::new(),
                pictures: HashMap::new(),
                used: Instant::now(),
            },
        );
    }
    let live = rooms.get_mut(&key).expect("joined room");
    if !live.installed.manifest.supports(&input.device) {
        return Err(invalid(
            "This session's release does not support this device.",
        ));
    }
    if !live.leases.contains_key(&caller.user) && live.leases.len() >= 128 {
        return Err(invalid("This activity is full."));
    }
    let token = Id::random();
    let ticket = Id::random();
    let port = stellarr_outbound::Url::parse(&origin)
        .map_err(ApiError::internal)?
        .port();
    let suffix = if port == 443 {
        String::new()
    } else {
        format!(":{port}")
    };
    live.leases.insert(
        caller.user,
        Lease {
            user: caller.user,
            account_session: caller.session,
            account_version: user.session_version,
            token,
            ticket: Some(ticket),
            cookie: Id::random(),
            touched: Instant::now(),
        },
    );
    live.used = Instant::now();
    Ok(Json(
        json!({"src":format!("https://{}.{domain}{suffix}/_stellarr/start/{ticket}",live.asset_label),"access":token,"title":live.installed.manifest.name,"permissions":live.installed.manifest.permissions}),
    ))
}
#[derive(Deserialize)]
pub(super) struct Action {
    install: Id,
    access: Id,
    action: Value,
}
fn in_session<T>(
    state: &AppState,
    channel: Scope,
    caller: Caller,
    input: &Action,
    headers: &HeaderMap,
    op: impl FnOnce(&mut Live) -> ApiResult<T>,
) -> ApiResult<T> {
    let (origin, _) = config(state)?;
    approved_origin(headers, &origin)?;
    let mut rooms = state.activities().rooms.lock().unwrap();
    let live = rooms
        .get_mut(&(input.install, channel))
        .ok_or_else(|| ApiError::status(StatusCode::GONE))?;
    if !seated(state, channel, caller.user) {
        live.authority.revoke(caller.user);
        live.leases.remove(&caller.user);
        live.pictures.remove(&caller.user);
        return Err(ApiError::forbidden());
    }
    let lease = live
        .leases
        .get_mut(&caller.user)
        .filter(|l| {
            l.account_session == caller.session
                && l.token == input.access
                && l.touched.elapsed() < Duration::from_secs(60)
        })
        .ok_or_else(ApiError::forbidden)?;
    lease.touched = Instant::now();
    live.used = Instant::now();
    op(live)
}

pub(super) async fn room(
    State(state): State<AppState>,
    caller: Caller,
    Path(path): Path<String>,
    headers: HeaderMap,
    Json(input): Json<Action>,
) -> ApiResult<Response> {
    let channel = Scope::from_path(&path, caller.user)?;
    let _gate = state.activities().changes.read().await;
    let mut person = json!({"id": caller.user});
    if input.action["op"] == "open" {
        let (origin, _) = config(&state)?;
        let user = state
            .database()
            .read(move |db| User::find(db, caller.user))
            .await?
            .ok_or_else(ApiError::forbidden)?;
        person["name"] = json!(user.display_name);
        person["avatar"] = json!(
            user.avatar_url
                .filter(|path| path.starts_with("/assets/"))
                .map(|path| format!("{origin}{path}"))
        );
    }
    in_session(&state, channel, caller, &input, &headers, |live| {
        let stale: Vec<Id> = live
            .leases
            .keys()
            .copied()
            .filter(|id| !seated(&state, channel, *id))
            .collect();
        for id in stale {
            live.authority.revoke(id);
            live.leases.remove(&id);
            live.pictures.remove(&id);
        }
        let output = live.authority.run(&person, &input.action)?;
        if let Some(error) = output.get("error") {
            let status = output["status"]
                .as_u64()
                .and_then(|s| StatusCode::from_u16(s as u16).ok())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            return Ok(Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(
                    error
                        .as_str()
                        .unwrap_or("Activity rejected the operation.")
                        .to_owned(),
                ))
                .unwrap());
        }
        let mut value = output["value"].clone();
        if input.action["op"] == "open" {
            value["place"] = channel.place();
            value["launchMode"] = json!(channel.mode());
            value["standalone"] = json!(false);
        }
        if input.action["op"] == "close" {
            live.leases.remove(&caller.user);
            live.pictures.remove(&caller.user);
        }
        Ok(Response::builder()
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CACHE_CONTROL, "no-store")
            .body(Body::from(value.to_string()))
            .unwrap())
    })
}
pub(super) async fn tile(
    State(state): State<AppState>,
    caller: Caller,
    Path(path): Path<String>,
    headers: HeaderMap,
    Json(input): Json<Action>,
) -> ApiResult<Json<Value>> {
    let channel = Scope::from_path(&path, caller.user)?;
    let _gate = state.activities().changes.read().await;
    in_session(&state, channel, caller, &input, &headers, |live| {
        if !live
            .installed
            .manifest
            .permissions
            .iter()
            .any(|s| s == "media.cameraTile")
        {
            return Err(ApiError::forbidden());
        }
        live.pictures.retain(|id, (at, _)| {
            at.elapsed() < Duration::from_millis(1800) && seated(&state, channel, *id)
        });
        let val = match input.action["op"].as_str() {
            Some("clear") => {
                live.pictures.remove(&caller.user);
                json!({})
            }
            Some("put") => {
                let image = input.action["image"]
                    .as_str()
                    .filter(|s| {
                        s.len() <= 96 * 1024
                            && s.strip_prefix("data:image/jpeg;base64,").is_some_and(|v| {
                                !v.is_empty()
                                    && v.bytes()
                                        .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
                            })
                    })
                    .ok_or_else(ApiError::bad_request)?;
                if live
                    .pictures
                    .get(&caller.user)
                    .is_some_and(|(at, _)| at.elapsed() < Duration::from_millis(100))
                {
                    return Err(ApiError::status(StatusCode::TOO_MANY_REQUESTS));
                }
                live.pictures
                    .insert(caller.user, (Instant::now(), image.into()));
                json!({})
            }
            Some("get") => {
                let ids = input.action["ids"]
                    .as_array()
                    .filter(|ids| ids.len() <= 16)
                    .ok_or_else(ApiError::bad_request)?;
                let mut result = json!({});
                for id in ids {
                    let id = id
                        .as_str()
                        .and_then(Id::parse)
                        .ok_or_else(ApiError::bad_request)?;
                    result[id.to_string()] = json!(live.pictures.get(&id).map(|(_, s)| s));
                }
                result
            }
            _ => return Err(ApiError::bad_request()),
        };
        Ok(Json(val))
    })
}

const SDK: &str = include_str!("../../../../../sdk/client.js");
const MEDIA: &str = include_str!("../../../../../sdk/media.js");
fn media_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "json" => "application/json",
        "css" => "text/css",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "avif" => "image/avif",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        _ => "application/octet-stream",
    }
}

fn asset_content(
    mut builder: axum::http::response::Builder,
    headers: &HeaderMap,
    content_type: &str,
    bytes: Bytes,
) -> Response {
    use crate::http::file::{Range, range_for};

    let length = bytes.len();
    let media = content_type.starts_with("video/") || content_type.starts_with("audio/");
    let range = if media && !headers.contains_key(header::IF_RANGE) {
        range_for(headers, length as u64)
    } else {
        Range::Whole
    };
    let body = match range {
        Range::Whole => bytes,
        Range::Part(first, last) => {
            builder = builder.status(StatusCode::PARTIAL_CONTENT).header(
                header::CONTENT_RANGE,
                format!("bytes {first}-{last}/{length}"),
            );
            bytes.slice(first as usize..=last as usize)
        }
        Range::Unsatisfiable => {
            builder = builder
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{length}"));
            Bytes::new()
        }
    };
    if media {
        builder = builder.header(header::ACCEPT_RANGES, "bytes");
    }
    builder
        .header(header::CONTENT_LENGTH, body.len().to_string())
        .body(Body::from(body))
        .expect("valid activity response")
}
pub(super) async fn asset_root(state: State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    asset(state, Path("index.html".to_owned()), headers).await
}

pub(super) async fn asset(
    State(state): State<AppState>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let _gate = state.activities().changes.read().await;
    let (origin, domain) = config(&state)?;
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(ApiError::forbidden)?;
    let label = host
        .strip_suffix(&format!(".{domain}"))
        .ok_or_else(ApiError::not_found)?;
    let (cookie, install, digest, manifest, credential) = {
        let mut rooms = state.activities().rooms.lock().unwrap();
        let received: HashSet<_> = headers
            .get(header::COOKIE)
            .and_then(|s| s.to_str().ok())
            .unwrap_or("")
            .split(';')
            .filter_map(|p| p.trim().split_once('='))
            .filter(|(key, _)| key.starts_with("__Host-stellarr-act-"))
            .filter_map(|(_, s)| Id::parse(s))
            .collect();
        let ticket = path.strip_prefix("_stellarr/start/").and_then(Id::parse);
        let mut found = None;
        for ((installation, channel), live) in rooms.iter_mut() {
            if live.asset_label != label {
                continue;
            }
            for lease in live.leases.values_mut() {
                if lease.touched.elapsed() >= Duration::from_secs(60)
                    || !seated(&state, *channel, lease.user)
                {
                    continue;
                }
                let cookie = match ticket {
                    Some(value) if lease.ticket == Some(value) => {
                        lease.ticket = None;
                        Some((lease.cookie, *channel))
                    }
                    None if received.contains(&lease.cookie) => None,
                    _ => continue,
                };
                found = Some((
                    cookie,
                    *installation,
                    live.installed.digest.clone(),
                    live.installed.manifest.clone(),
                    (lease.user, lease.account_version, lease.account_session),
                ));
                break;
            }
            if found.is_some() {
                break;
            }
        }
        found.ok_or_else(ApiError::forbidden)?
    };
    let current = state.clone();
    let active = state
        .database()
        .read(move |db| {
            let (user, version, session_id) = credential;
            let checked = session::authenticate(
                db,
                current.sealer(),
                &current.settings().environment,
                true,
                session::Ticket::of_connection(user, version, session_id),
                Timestamp::now(),
            )?;
            Ok::<_, crate::database::DbError>(matches!(
                checked,
                session::Authentication::SignedIn { .. }
            ))
        })
        .await?;
    if !active {
        return Err(ApiError::forbidden());
    }
    if let Some((cookie, scope)) = cookie {
        let key = match scope {
            Scope::Personal(id) => format!("personal-{id}"),
            Scope::Voice(id) => format!("voice-{id}"),
        };

        return Ok(Response::builder().status(StatusCode::FOUND).header(header::LOCATION,"/index.html")
            .header(header::SET_COOKIE,format!("__Host-stellarr-act-{key}={cookie}; Path=/; Secure; HttpOnly; SameSite=None; Partitioned"))
            .header(header::CACHE_CONTROL,"no-store").header(header::REFERRER_POLICY,"no-referrer").body(Body::empty()).unwrap());
    }
    let (content_type, bytes) = match path.as_str() {
        "_stellarr/sdk/v1.js" => ("text/javascript", Bytes::from_static(SDK.as_bytes())),
        "_stellarr/sdk/media.js" => ("text/javascript", Bytes::from_static(MEDIA.as_bytes())),
        _ => {
            let file = path;
            let mime = media_type(&file);
            let bytes=state.database().read(move|db|->ApiResult<Vec<u8>>{
                use rusqlite::OptionalExtension;
                db.query_row("SELECT f.Content FROM ActivityFiles f JOIN ActivityInstalls i ON i.Id=f.Install WHERE i.Enabled=1 AND f.Install=?1 AND f.Digest=?2 AND f.Path=?3",rusqlite::params![install,digest,file],|r|r.get(0)).optional()?.ok_or_else(ApiError::not_found)
            }).await?;
            (mime, Bytes::from(bytes))
        }
    };
    let external = manifest
        .connects
        .iter()
        .map(|c| format!("https://{}", c.host))
        .collect::<Vec<_>>()
        .join(" ");
    let csp = format!(
        "default-src 'self' {external}; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval' {external}; style-src 'self' 'unsafe-inline' {external}; img-src 'self' data: blob: {origin}/assets/ {external}; media-src 'self' blob: {external}; worker-src 'self' blob:; frame-ancestors {origin}; object-src 'none'; base-uri 'none'; form-action 'none'"
    );
    let builder = Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .header(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_str(&csp).map_err(ApiError::internal)?,
        )
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::REFERRER_POLICY, "strict-origin-when-cross-origin")
        .header(
            "Permissions-Policy",
            "camera=(), microphone=(), display-capture=()",
        );
    Ok(asset_content(builder, &headers, content_type, bytes))
}

#[cfg(test)]
mod asset_response_tests {
    use super::*;

    #[tokio::test]
    async fn installed_media_serves_browser_ranges() {
        let mut headers = HeaderMap::new();
        let content = Bytes::from_static(b"0123456789");
        for (request, value, bytes) in [
            ("bytes=0-1", "bytes 0-1/10", &b"01"[..]),
            ("bytes=7-", "bytes 7-9/10", &b"789"[..]),
            ("bytes=-3", "bytes 7-9/10", &b"789"[..]),
        ] {
            headers.insert(header::RANGE, HeaderValue::from_static(request));
            let response =
                asset_content(Response::builder(), &headers, "video/mp4", content.clone());
            assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
            assert_eq!(response.headers()[header::ACCEPT_RANGES], "bytes");
            assert_eq!(response.headers()[header::CONTENT_RANGE], value);
            assert_eq!(
                axum::body::to_bytes(response.into_body(), 10)
                    .await
                    .unwrap(),
                bytes
            );
        }
        headers.insert(header::RANGE, HeaderValue::from_static("bytes=20-"));
        let response = asset_content(Response::builder(), &headers, "video/mp4", content.clone());
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes */10");
        let nonmedia = asset_content(Response::builder(), &headers, "text/html", content);
        assert_eq!(nonmedia.status(), StatusCode::OK);
        assert_eq!(nonmedia.headers()[header::CONTENT_LENGTH], "10");
    }
}
