use super::{Installed, LaunchMode, authority::Authority, hostname, invalid, library};
use crate::{
    auth::{Caller, session},
    database::DbError,
    encoding::hex_lower,
    events::{Audience, Event},
    http::{ApiError, ApiResult, Json, Path, Problem, Query},
    instance::users::User,
    permissions::{membership, visibility},
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
type Rooms = HashMap<Place, Live>;
#[derive(Default)]
pub(crate) struct Sessions {
    pub(crate) changes: tokio::sync::RwLock<()>,
    rooms: Mutex<Rooms>,
}
struct Lease {
    user: Id,
    /// When the person came in, counted per room: whoever has been inside
    /// longest takes over from a host who leaves.
    joined: u64,
    account_session: Id,
    account_version: i64,
    parent_origin: String,
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
    /// Who launched the activity, or took it over from whoever did. Only the
    /// host ends it for everybody or swaps it for another; in a personal room
    /// it is the account itself.
    host: Id,
    /// How many people have come in, which numbers the next one.
    arrivals: u64,
}
impl Live {
    fn new(installed: Installed, host: Id) -> ApiResult<Live> {
        Ok(Live {
            authority: Authority::new(&installed.manifest)?,
            asset_label: asset_label(&installed),
            installed,
            leases: HashMap::new(),
            pictures: HashMap::new(),
            used: Instant::now(),
            host,
            arrivals: 0,
        })
    }
    /// The people inside, in the order they came in.
    fn participants(&self) -> Vec<Id> {
        let mut inside: Vec<&Lease> = self.leases.values().collect();
        inside.sort_by_key(|lease| lease.joined);
        inside.iter().map(|lease| lease.user).collect()
    }
    /// Takes somebody out: their lease, their part in the authority and their
    /// picture. A host who leaves hands the activity to whoever has been inside
    /// longest, and it keeps running.
    fn depart(&mut self, user: Id) {
        self.leases.remove(&user);
        self.authority.revoke(user);
        self.pictures.remove(&user);
        if self.host == user
            && let Some(next) = self
                .leases
                .values()
                .min_by_key(|lease| lease.joined)
                .map(|lease| lease.user)
        {
            self.host = next;
        }
    }
    /// Takes out everybody whose lease lapsed or who no longer sits where the
    /// activity runs.
    fn sweep(&mut self, state: &AppState, scope: Scope) {
        let departed: Vec<Id> = self
            .leases
            .values()
            .filter(|lease| {
                lease.touched.elapsed() > Duration::from_secs(60)
                    || !seated(state, scope, lease.user)
            })
            .map(|lease| lease.user)
            .collect();
        for user in departed {
            self.depart(user);
        }
    }
    /// Whether the room still runs: a voice room while anybody is in it, a
    /// personal one for three minutes after its last use, and neither once its
    /// authority has ended the session.
    fn alive(&self, scope: Scope) -> bool {
        let used = match scope {
            Scope::Voice(_) => !self.leases.is_empty(),
            Scope::Personal(_) => self.used.elapsed() < Duration::from_secs(180),
        };
        used && !self.authority.expired()
    }
}
/// What the channel list shows of the activity that runs in a voice channel.
fn summary(channel: Id, live: &Live) -> Value {
    let artwork = super::artwork::urls(
        &live.installed,
        &format!("/api/activities/{channel}/art/{}", live.installed.install),
    );
    json!({
        "install": live.installed.install,
        "name": live.installed.manifest.name,
        "icon": artwork["icon"],
        "host": live.host,
        "participants": live.participants(),
        "seats": live.installed.manifest.seats,
    })
}
/// The activity that runs in a voice channel, with its installation.
fn voice_room(rooms: &Rooms, channel: Id) -> Option<(Id, &Live)> {
    rooms
        .iter()
        .find(|((_, scope), _)| *scope == Scope::Voice(channel))
        .map(|((install, _), live)| (*install, live))
}

/// What a voice channel shows of its activity: which one runs, who hosts it
/// and who is inside, in order.
type Shown = (Id, Id, Vec<Id>);
fn shown(rooms: &Rooms) -> HashMap<Id, Shown> {
    rooms
        .iter()
        .filter_map(|((install, scope), live)| match scope {
            Scope::Voice(channel) => Some((*channel, (*install, live.host, live.participants()))),
            Scope::Personal(_) => None,
        })
        .collect()
}
/// A voice channel whose activity looks other than it did.
struct Change {
    channel: Id,
    /// Another activity took the place of the one that ran: those who were in
    /// it hear that it ended before they hear of the next one.
    replaced: bool,
}
fn changes(before: &HashMap<Id, Shown>, after: &HashMap<Id, Shown>) -> Vec<Change> {
    let mut changed: Vec<Change> = Vec::new();
    for channel in before.keys().chain(after.keys()) {
        let (was, is) = (before.get(channel), after.get(channel));
        if was != is && !changed.iter().any(|change| change.channel == *channel) {
            changed.push(Change {
                channel: *channel,
                replaced: matches!((was, is), (Some(was), Some(is)) if was.0 != is.0),
            });
        }
    }
    changed
}

fn publish_voice_activity(
    state: &AppState,
    audience: Audience,
    channel: Id,
    activity: Option<Value>,
) {
    state.events().publish(Event::new(
        audience,
        "VoiceActivityChanged",
        &json!({ "channelId": channel, "activity": activity }),
    ));
}

/// Tells everybody who sees each of these voice channels, and whoever sits in
/// it, what runs there now: the channel list shows it to people outside the
/// call too. The audience is read first. What runs is read and sent under the
/// lock of the rooms, so no two announcements of a channel reach anybody in
/// the reverse order of the changes they follow.
async fn announce(state: &AppState, changed: Vec<Change>) {
    for change in changed {
        match crate::voice::rooms::audience(state, change.channel).await {
            Ok(Some(audience)) => state.activities().send_current(state, audience, &change),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, "a channel was not told which activity runs in it");
            }
        }
    }
}

fn participant(user: &User) -> Value {
    let avatar = user
        .avatar_url
        .as_deref()
        .and_then(|url| url.strip_prefix("/assets/"))
        .and_then(Id::parse)
        .map(|asset| format!("/_stellarr/avatars/{}/{asset}", user.id));
    json!({ "id": user.id, "name": user.display_name, "username": user.username, "avatar": avatar })
}

impl Sessions {
    pub(crate) fn update_profile(&self, user: &User) -> ApiResult<()> {
        let person = participant(user);
        for live in self
            .rooms
            .lock()
            .unwrap()
            .values()
            .filter(|live| live.leases.contains_key(&user.id))
        {
            live.authority.update_profile(&person)?;
        }
        Ok(())
    }

    pub(super) fn pinned_digest(&self, id: Id, channel: Scope) -> Option<String> {
        self.rooms
            .lock()
            .unwrap()
            .get(&(id, channel))
            .map(|live| live.installed.digest.clone())
    }
    /// Changes the rooms under their lock and names the voice channels whose
    /// activity looks other than before, for [`announce`] once the lock is
    /// released.
    fn update<T>(&self, op: impl FnOnce(&mut Rooms) -> T) -> (T, Vec<Change>) {
        let mut rooms = self.rooms.lock().unwrap();
        let before = shown(&rooms);
        let output = op(&mut rooms);
        let after = shown(&rooms);
        (output, changes(&before, &after))
    }
    fn send_current(&self, state: &AppState, audience: Audience, change: &Change) {
        let rooms = self.rooms.lock().unwrap();
        let current =
            voice_room(&rooms, change.channel).map(|(_, live)| summary(change.channel, live));
        if change.replaced {
            publish_voice_activity(state, audience.clone(), change.channel, None);
        }
        publish_voice_activity(state, audience, change.channel, current);
    }
    /// Ends every room of an installation that was turned off or removed.
    pub async fn remove(&self, state: &AppState, id: Id) {
        let (_, changed) =
            self.update(|rooms| rooms.retain(|(installed, _), _| *installed != id));
        announce(state, changed).await;
    }
    /// Tells a connection that has just taken a seat what runs in the room:
    /// somebody moved there may not see the channel, and so has not heard.
    pub(crate) fn announce_to_connection(&self, state: &AppState, connection: &str, channel: Id) {
        let active = voice_room(&self.rooms.lock().unwrap(), channel)
            .map(|(_, live)| summary(channel, live));
        if active.is_some() {
            publish_voice_activity(
                state,
                Audience::Connection(connection.to_owned()),
                channel,
                active,
            );
        }
    }
    /// Takes whoever no longer sits in a voice room out of the activity that
    /// runs there, once a seat was freed, handing a host's place on, and tells
    /// everybody who sees the channel.
    pub(crate) async fn sync_voice_channel(&self, state: &AppState, channel: Id) {
        let (_, changed) =
            self.update(|rooms| prune(rooms, state, Some(Scope::Voice(channel))));
        announce(state, changed).await;
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
/// A wildcard deployment has a configured parent. For a fresh instance use
/// the HTTPS origin the browser actually opened, including the public port.
pub(super) struct ActivityParent(String);

impl axum::extract::FromRequestParts<AppState> for ActivityParent {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &AppState,
    ) -> ApiResult<Self> {
        let origin = if state.settings().activity_domain.is_some() {
            config(state)?.0
        } else {
            let client = parts
                .extensions
                .get::<crate::http::ClientInfo>()
                .ok_or_else(ApiError::forbidden)?;
            let host = client
                .host
                .as_deref()
                .or_else(|| parts.headers.get(header::HOST)?.to_str().ok())
                .ok_or_else(ApiError::forbidden)?;
            let origin = format!("https://{host}");
            let url = stellarr_outbound::Url::parse(&origin).map_err(|_| ApiError::forbidden())?;
            if !client.is_https() || url.host_header() != host {
                return Err(ApiError::forbidden());
            }
            origin
        };
        approved_origin(&parts.headers, &origin)?;
        Ok(Self(origin))
    }
}
/// Why a launch or an end was refused, which the client reads from the
/// problem's `code` to say why.
enum Refusal {
    /// No such installation, or it is turned off, or nothing runs to join or
    /// to end.
    NotFound,
    /// Another activity runs in the voice channel, and the launch did not ask
    /// to swap it.
    Running,
    /// Only the host ends an activity or swaps it for another.
    NotHost,
    /// Every seat of the activity is taken.
    Full,
    /// The release does not run on the device the launch named.
    UnsupportedDevice,
}
impl From<Refusal> for ApiError {
    fn from(refusal: Refusal) -> ApiError {
        let (status, code, title) = match refusal {
            Refusal::NotFound => (
                StatusCode::NOT_FOUND,
                "not-found",
                "This activity is not installed, not enabled or not running.",
            ),
            Refusal::Running => (
                StatusCode::CONFLICT,
                "activity-running",
                "Another activity is running in this channel.",
            ),
            Refusal::NotHost => (
                StatusCode::FORBIDDEN,
                "not-host",
                "Only the host can end or swap this activity.",
            ),
            Refusal::Full => (StatusCode::CONFLICT, "full", "This activity is full."),
            Refusal::UnsupportedDevice => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "unsupported-device",
                "This activity does not support this device.",
            ),
        };
        Problem::new(status).title(title).with("code", code).into()
    }
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
/// Whether the person sees the voice channel an activity runs in, as their
/// channel list shows it, without sitting in it.
pub(super) async fn sees(state: &AppState, channel: Scope, user: Id) -> ApiResult<bool> {
    let (Scope::Voice(room), Some(realtime)) = (channel, state.realtime()) else {
        return Ok(false);
    };
    let realtime = realtime.clone();
    Ok(state
        .database()
        .read(move |db| -> Result<bool, DbError> {
            Ok(crate::voice::rooms::sees(db, realtime.presence(), room, user)?)
        })
        .await?)
}
fn approved_origin(headers: &HeaderMap, origin: &str) -> ApiResult<()> {
    if headers.get(header::ORIGIN).and_then(|h| h.to_str().ok()) == Some(origin) {
        Ok(())
    } else {
        Err(ApiError::forbidden())
    }
}
/// Ends what no longer runs and takes out whoever left, in the rooms of one
/// place or, without one, in all of them.
fn prune(rooms: &mut Rooms, state: &AppState, only: Option<Scope>) {
    rooms.retain(|(_, scope), live| {
        if only.is_some_and(|only| only != *scope) {
            return true;
        }
        live.sweep(state, *scope);
        live.alive(*scope)
    })
}
/// The instance's owner, whom the picker names as the one who installs
/// activities, and whether that is the person asking: the account the
/// operator panel lets in.
fn instance_owner(db: &rusqlite::Connection, viewer: Id) -> rusqlite::Result<(Value, bool)> {
    use rusqlite::OptionalExtension;
    let owner = match membership::instance_server(db)? {
        Some(server) => db
            .query_row(
                "SELECT u.\"Id\", u.\"DisplayName\", u.\"AvatarUrl\" FROM \"ServerMembers\" m
                 JOIN \"ServerRoles\" r ON r.\"Id\" = m.\"RoleId\" JOIN \"Users\" u ON u.\"Id\" = m.\"UserId\"
                 WHERE m.\"ServerId\" = ?1 AND r.\"IsOwner\" = 1 LIMIT 1",
                [server],
                |row| {
                    let id: Id = row.get(0)?;
                    let name: String = row.get(1)?;
                    let avatar: Option<String> = row.get(2)?;
                    Ok(json!({"id": id, "name": name, "avatar": avatar}))
                },
            )
            .optional()?,
        None => None,
    };
    let viewer_is_owner = membership::find_administered_server(db, viewer)?.is_some();
    Ok((json!(owner), viewer_is_owner))
}
#[derive(Deserialize)]
pub(super) struct ActiveLaunch {
    install: Id,
    access: Id,
}
#[derive(Deserialize)]
pub(super) struct Device {
    device: String,
    lease: Option<ActiveLaunch>,
}
pub(super) async fn available(
    State(state): State<AppState>,
    caller: Caller,
    Path(path): Path<String>,
    ActivityParent(_): ActivityParent,
    Json(input): Json<Device>,
) -> ApiResult<Json<Value>> {
    let channel = Scope::from_path(&path, caller.user)?;
    check(&state, channel, caller.user)?;
    let viewer = caller.user;
    let (owner, viewer_is_owner) = state
        .database()
        .read(move |db| -> Result<(Value, bool), DbError> { Ok(instance_owner(db, viewer)?) })
        .await?;
    let _gate = state.activities().changes.read().await;
    let all = library(&state).await?;
    // What only a voice channel runs, which a personal picker names so that
    // nobody looks for it there in vain.
    let voice_only: Vec<String> = match channel {
        Scope::Personal(_) => all
            .iter()
            .filter(|entry| {
                entry.enabled && !entry.manifest.launch_modes.contains(&LaunchMode::Personal)
            })
            .map(|entry| entry.manifest.name.clone())
            .collect(),
        Scope::Voice(_) => Vec::new(),
    };
    let (listed, changed) = state.activities().update(|rooms| -> ApiResult<(Vec<Value>, Option<Value>)> {
        prune(rooms, &state, None);
        if let Some(active) = input.lease {
            // A static page may never open an SDK connection. Its parent keeps
            // this specific launch alive for as long as the frame is displayed.
            let gone = || ApiError::status(StatusCode::GONE);
            let live = rooms.get_mut(&(active.install, channel)).ok_or_else(gone)?;
            let lease = live
                .leases
                .get_mut(&caller.user)
                .filter(|lease| lease.account_session == caller.session && lease.token == active.access)
                .ok_or_else(gone)?;
            let now = Instant::now();
            lease.touched = now;
            live.used = now;
        }
        let rooms: &Rooms = rooms;
        let list:Vec<Value>=all.into_iter().filter(|entry|entry.enabled).filter_map(|entry|{
            let live=rooms.get(&(entry.install,channel));
            let current=live.map_or(&entry,|room|&room.installed);
            if !current.manifest.launch_modes.contains(&channel.mode()){ return None; }
            let artwork=super::artwork::urls(current,&format!("/api/activities/{}/art/{}",channel.segment(),entry.install));
            Some(json!({"install":entry.install,"name":current.manifest.name,"version":current.manifest.version,"running":live.is_some(),
                   "description":current.manifest.description,"artwork":artwork,
                   "supported":current.manifest.supports(&input.device),"devices":current.manifest.devices,"seats":current.manifest.seats,
                   "permissions":current.manifest.permissions}))
        }).collect();
        let running = match channel {
            Scope::Voice(room) => voice_room(rooms, room).map(|(_, live)| summary(room, live)),
            Scope::Personal(_) => None,
        };
        Ok((list, running))
    });
    announce(&state, changed).await;
    let (list, running) = listed?;
    Ok(Json(json!({
        "activities": list,
        "voiceOnly": voice_only,
        "live": running,
        "owner": owner,
        "viewerIsOwner": viewer_is_owner,
    })))
}
#[derive(Deserialize)]
pub(super) struct Launch {
    #[serde(default)]
    install: Option<Id>,
    device: String,
    /// Swap the activity that runs in the voice channel for this one, which
    /// only its host may do.
    #[serde(default)]
    replace: bool,
}
pub(super) async fn launch(
    State(state): State<AppState>,
    caller: Caller,
    Path(path): Path<String>,
    ActivityParent(origin): ActivityParent,
    Json(input): Json<Launch>,
) -> ApiResult<Json<Value>> {
    let channel = Scope::from_path(&path, caller.user)?;
    let _gate = state.activities().changes.read().await;
    let all = library(&state).await?;
    check(&state, channel, caller.user)?;
    let user = state
        .database()
        .read(move |db| User::find(db, caller.user))
        .await?
        .ok_or_else(ApiError::forbidden)?;
    let (launched, changed) = state.activities().update(|rooms| -> ApiResult<Value> {
        prune(rooms, &state, None);
        // A voice channel runs one activity at a time; a person runs as many
        // personal ones as they like.
        let running = match channel {
            Scope::Voice(room) => voice_room(rooms, room).map(|(install, live)| (install, live.host)),
            Scope::Personal(_) => None,
        };
        let (install_id, replaced) = match (input.install, running) {
            (Some(requested), Some((current, host))) if requested != current => {
                if !input.replace {
                    return Err(Refusal::Running.into());
                }
                if host != caller.user {
                    return Err(Refusal::NotHost.into());
                }
                (requested, Some(current))
            }
            (Some(requested), _) => (requested, None),
            (None, Some((current, _))) => (current, None),
            (None, None) => {
                return Err(match channel {
                    Scope::Voice(_) => ApiError::from(Refusal::NotFound),
                    Scope::Personal(_) => ApiError::bad_request(),
                });
            }
        };
        let key = (install_id, channel);
        let current = all
            .into_iter()
            .find(|a| a.install == install_id && a.enabled);
        let allowed_mode = rooms.get(&key).map_or_else(
            || {
                current
                    .as_ref()
                    .map(|c| c.manifest.launch_modes.contains(&channel.mode()))
                    .ok_or(Refusal::NotFound)
            },
            |room| Ok(room.installed.manifest.launch_modes.contains(&channel.mode())),
        )?;
        if !allowed_mode {
            return Err(ApiError::forbidden());
        }
        if !rooms.contains_key(&key) {
            let current = current.ok_or(Refusal::NotFound)?;
            if !current.manifest.supports(&input.device) {
                return Err(Refusal::UnsupportedDevice.into());
            }
            if rooms.len().saturating_sub(usize::from(replaced.is_some())) >= 64 {
                return Err(ApiError::status(StatusCode::TOO_MANY_REQUESTS));
            }
            let started = Live::new(current, caller.user)?;
            // The host swaps the activity for another: the one that ran ends
            // for everybody in it, as an end does, and the host hosts the next.
            if let Some(replaced) = replaced {
                rooms.remove(&(replaced, channel));
            }
            rooms.insert(key, started);
        }
        let live = rooms.get_mut(&key).expect("joined room");
        if !live.installed.manifest.supports(&input.device) {
            return Err(Refusal::UnsupportedDevice.into());
        }
        let cap = live
            .installed
            .manifest
            .seats
            .map_or(128, |seats| usize::from(seats).min(128));
        // Somebody who opens the activity again keeps their place in the order.
        let joined = match live.leases.get(&caller.user) {
            Some(lease) => lease.joined,
            None if live.leases.len() >= cap => return Err(Refusal::Full.into()),
            None => {
                live.arrivals += 1;
                live.arrivals
            }
        };
        let token = Id::random();
        let ticket = Id::random();
        let asset_key = Id::random();
        let src = if let Some(domain) = &state.settings().activity_domain {
            let port = stellarr_outbound::Url::parse(&origin)
                .map_err(ApiError::internal)?
                .port();
            let suffix = if port == 443 {
                String::new()
            } else {
                format!(":{port}")
            };
            format!(
                "https://{}.{domain}{suffix}/_stellarr/start/{ticket}",
                live.asset_label
            )
        } else {
            format!("{origin}/_activity/{asset_key}/index.html")
        };
        live.leases.insert(
            caller.user,
            Lease {
                user: caller.user,
                joined,
                account_session: caller.session,
                account_version: user.session_version,
                token,
                ticket: Some(ticket),
                cookie: asset_key,
                parent_origin: origin,
                touched: Instant::now(),
            },
        );
        live.used = Instant::now();
        Ok(json!({
            "install": install_id,
            "src": src,
            "access": token,
            "title": live.installed.manifest.name,
            "permissions": live.installed.manifest.permissions,
            "youtube": live.installed.manifest.connects.iter().any(|c| c.host == "www.youtube-nocookie.com"),
            "host": live.host,
        }))
    });
    announce(&state, changed).await;
    Ok(Json(launched?))
}

pub(super) async fn leave(
    State(state): State<AppState>,
    caller: Caller,
    Path(path): Path<String>,
    ActivityParent(_): ActivityParent,
    Json(input): Json<ActiveLaunch>,
) -> ApiResult<Json<Value>> {
    let channel = Scope::from_path(&path, caller.user)?;
    let key = (input.install, channel);
    let (still_running, changed) = state.activities().update(|rooms| {
        prune(rooms, &state, None);
        if let Some(live) = rooms.get_mut(&key)
            && live
                .leases
                .get(&caller.user)
                .is_some_and(|l| l.account_session == caller.session && l.token == input.access)
        {
            live.depart(caller.user);
            if live.leases.is_empty() {
                rooms.remove(&key);
            }
        }
        rooms.contains_key(&key)
    });
    announce(&state, changed).await;
    Ok(Json(json!({ "ok": true, "running": still_running })))
}
#[derive(Deserialize)]
pub(super) struct End {
    /// The personal room to end; without it, every personal room of the
    /// account. A voice channel runs one activity, which is the one ended.
    #[serde(default)]
    install: Option<Id>,
}
/// Ends the activity for everybody in it, which only its host may do.
pub(super) async fn end(
    State(state): State<AppState>,
    caller: Caller,
    Path(path): Path<String>,
    ActivityParent(_): ActivityParent,
    input: Option<Json<End>>,
) -> ApiResult<Json<Value>> {
    let channel = Scope::from_path(&path, caller.user)?;
    let only = input.and_then(|Json(end)| end.install);
    let (ended, changed) = state.activities().update(|rooms| -> ApiResult<()> {
        prune(rooms, &state, None);
        let ending: Vec<Place> = rooms
            .keys()
            .copied()
            .filter(|(install, place)| *place == channel && only.is_none_or(|only| only == *install))
            .collect();
        if ending.is_empty() {
            return Err(Refusal::NotFound.into());
        }
        if ending.iter().any(|key| rooms[key].host != caller.user) {
            return Err(Refusal::NotHost.into());
        }
        // A room takes its authority with it: every lease, picture and the
        // session's state are gone, and a frame that still calls is told the
        // session ended.
        for key in ending {
            rooms.remove(&key);
        }
        Ok(())
    });
    announce(&state, changed).await;
    ended?;
    Ok(Json(json!({ "ok": true })))
}
#[derive(Deserialize)]
pub(super) struct Listed {
    server: Id,
}
/// What runs in the voice channels of a server that the person sees, for the
/// channel list to show until `VoiceActivityChanged` says otherwise.
pub(super) async fn running(
    State(state): State<AppState>,
    caller: Caller,
    Query(query): Query<Listed>,
) -> ApiResult<Json<Value>> {
    let (server, user) = (query.server, caller.user);
    let channels = state
        .database()
        .read(move |db| -> Result<Option<Vec<Id>>, DbError> {
            if !membership::is_member(db, server, user)? {
                return Ok(None);
            }
            let mut channels = crate::voice::voice_channels_of(db, server)?;
            if let Some(view) = visibility::of_member(db, user, server)? {
                channels.retain(|channel| view.channels.contains(channel));
            }
            Ok(Some(channels))
        })
        .await?
        .ok_or_else(ApiError::forbidden)?;
    let (listed, changed) = state.activities().update(|rooms| {
        prune(rooms, &state, None);
        let rooms: &Rooms = rooms;
        channels
            .iter()
            .filter_map(|channel| {
                voice_room(rooms, *channel).map(|(_, live)| {
                    json!({"channelId": channel, "activity": summary(*channel, live)})
                })
            })
            .collect::<Vec<Value>>()
    });
    announce(&state, changed).await;
    Ok(Json(json!(listed)))
}
#[derive(Deserialize)]
pub(super) struct Action {
    install: Id,
    access: Id,
    action: Value,
}
async fn in_session<T>(
    state: &AppState,
    channel: Scope,
    caller: Caller,
    input: &Action,
    op: impl FnOnce(&mut Live) -> ApiResult<T>,
) -> ApiResult<T> {
    let key = (input.install, channel);
    let (result, changed) = state.activities().update(|rooms| {
        // Whoever has gone is taken out first, a host handing over: the
        // people who stay are the ones who keep calling.
        prune(rooms, state, Some(channel));
        let result = rooms
            .get_mut(&key)
            .ok_or_else(|| ApiError::status(StatusCode::GONE))
            .and_then(|live| {
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
            });
        // The last person to close a voice activity ends it.
        if matches!(channel, Scope::Voice(_))
            && rooms.get(&key).is_some_and(|live| live.leases.is_empty())
        {
            rooms.remove(&key);
        }
        result
    });
    announce(state, changed).await;
    result
}

pub(super) async fn room(
    State(state): State<AppState>,
    caller: Caller,
    Path(path): Path<String>,
    _: ActivityParent,
    Json(input): Json<Action>,
) -> ApiResult<Response> {
    let channel = Scope::from_path(&path, caller.user)?;
    let _gate = state.activities().changes.read().await;
    let mut person = json!({"id": caller.user});
    if input.action["op"] == "open" {
        let user = state
            .database()
            .read(move |db| User::find(db, caller.user))
            .await?
            .ok_or_else(ApiError::forbidden)?;
        person = participant(&user);
    }
    let (mut output, progress) = in_session(&state, channel, caller, &input, |live| {
        let output = live.authority.run(&person, &input.action)?;
        let progress = matches!(
            input.action["op"].as_str(),
            Some("loadProgress" | "saveProgress")
        )
        .then(|| super::progress::ProgressKey {
            user: caller.user,
            source: live.installed.source.clone(),
            activity: live.installed.manifest.id.clone(),
        });
        if input.action["op"] == "close" && output.get("error").is_none() {
            live.depart(caller.user);
        }
        Ok((output, progress))
    })
    .await?;
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
    let mut value = output["value"].take();
    if let Some(key) = progress {
        value = key.run(&state, input.action).await?;
    } else if input.action["op"] == "open" {
        value["place"] = channel.place();
        value["launchMode"] = json!(channel.mode());
        value["standalone"] = json!(false);
        value["progress"] = json!(true);
    }
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(value.to_string()))
        .unwrap())
}
pub(super) async fn tile(
    State(state): State<AppState>,
    caller: Caller,
    Path(path): Path<String>,
    ActivityParent(_): ActivityParent,
    Json(input): Json<Action>,
) -> ApiResult<Json<Value>> {
    let channel = Scope::from_path(&path, caller.user)?;
    let _gate = state.activities().changes.read().await;
    in_session(&state, channel, caller, &input, |live| {
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
    .await
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
    let avatar = avatar_path(&path)?;
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
            if live.asset_label != label
                || avatar.is_some_and(|(owner, _)| {
                    !live.leases.contains_key(&owner) || !seated(&state, *channel, owner)
                })
            {
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
    verify_asset_session(&state, credential).await?;
    if let Some((owner, asset)) = avatar {
        return serve_avatar(&state, credential, owner, asset, headers).await;
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
    let (content_type, bytes) = read_asset(&state, install, digest, path).await?;
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

fn avatar_path(path: &str) -> ApiResult<Option<(Id, Id)>> {
    let Some(suffix) = path.strip_prefix("_stellarr/avatars/") else {
        return Ok(None);
    };
    let (owner, asset) = suffix.split_once('/').ok_or_else(ApiError::not_found)?;
    Ok(Some((
        Id::parse(owner).ok_or_else(ApiError::not_found)?,
        Id::parse(asset).ok_or_else(ApiError::not_found)?,
    )))
}

async fn serve_avatar(
    state: &AppState,
    credential: (Id, i64, Id),
    owner: Id,
    asset: Id,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let current = state
        .database()
        .read(move |db| -> ApiResult<bool> {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM Users WHERE Id=?1 AND AvatarUrl=?2)",
                rusqlite::params![owner, format!("/assets/{asset}")],
                |row| row.get(0),
            )?)
        })
        .await?;
    if !current {
        return Err(ApiError::forbidden());
    }
    let mut response = crate::assets::download(
        State(state.clone()),
        Caller {
            user: credential.0,
            session: credential.2,
        },
        Path(asset),
        headers,
    )
    .await?;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    Ok(response)
}

async fn verify_asset_session(state: &AppState, credential: (Id, i64, Id)) -> ApiResult<()> {
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
    Ok(())
}

async fn read_asset(
    state: &AppState,
    install: Id,
    digest: String,
    path: String,
) -> ApiResult<(&'static str, Bytes)> {
    Ok(match path.as_str() {
        "_stellarr/sdk/v1.js" => ("text/javascript", Bytes::from_static(SDK.as_bytes())),
        "_stellarr/sdk/media.js" => ("text/javascript", Bytes::from_static(MEDIA.as_bytes())),
        _ => {
            let mime = media_type(&path);
            let bytes = state
                .database()
                .read(move |db| -> ApiResult<Vec<u8>> {
                    use rusqlite::OptionalExtension;
                    db.query_row(
                    "SELECT f.Content FROM ActivityFiles f JOIN ActivityInstalls i ON i.Id=f.Install
                     WHERE i.Enabled=1 AND f.Install=?1 AND f.Digest=?2 AND f.Path=?3",
                    rusqlite::params![install, digest, path],
                    |row| row.get(0),
                ).optional()?.ok_or_else(ApiError::not_found)
                })
                .await?;
            (mime, Bytes::from(bytes))
        }
    })
}

/// This URL carries a read-only, per-launch capability. The sandbox sends
/// anonymous CORS requests for modules and fonts, including relative imports.
pub(super) async fn sandbox_asset(
    State(state): State<AppState>,
    Path((key, path)): Path<(Id, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    if state.settings().activity_domain.is_some()
        || headers.contains_key("service-worker")
        || headers
            .get("sec-fetch-dest")
            .is_some_and(|d| d == "serviceworker" || d == "sharedworker")
    {
        return Err(ApiError::forbidden());
    }
    let _gate = state.activities().changes.read().await;
    let avatar = avatar_path(&path)?;
    let (install, digest, manifest, origin, credential) = {
        let rooms = state.activities().rooms.lock().unwrap();
        rooms
            .iter()
            .find_map(|((install, scope), live)| {
                let lease = live.leases.values().find(|l| l.cookie == key)?;
                if avatar.is_some_and(|(owner, _)| {
                    !live.leases.contains_key(&owner) || !seated(&state, *scope, owner)
                }) {
                    return None;
                }
                (lease.touched.elapsed() < Duration::from_secs(60)
                    && seated(&state, *scope, lease.user))
                .then(|| {
                    (
                        *install,
                        live.installed.digest.clone(),
                        live.installed.manifest.clone(),
                        lease.parent_origin.clone(),
                        (lease.user, lease.account_version, lease.account_session),
                    )
                })
            })
            .ok_or_else(ApiError::forbidden)?
    };
    verify_asset_session(&state, credential).await?;
    if let Some((owner, asset)) = avatar {
        return serve_avatar(&state, credential, owner, asset, headers).await;
    }
    let (mime, bytes) = read_asset(&state, install, digest, path).await?;
    let files = format!("{origin}/_activity/{key}/");
    let external = manifest
        .connects
        .iter()
        .map(|c| format!("https://{}", c.host))
        .collect::<Vec<_>>()
        .join(" ");
    let frames = if external.is_empty() {
        "'none'"
    } else {
        &external
    };
    let policy = format!(
        "sandbox allow-scripts allow-forms allow-presentation; default-src 'none'; \
         script-src {files} {origin}/_stellarr/sdk/ 'unsafe-inline' 'wasm-unsafe-eval' {external}; \
         style-src {files} 'unsafe-inline' {external}; img-src {files} {origin}/assets/ data: blob: {external}; \
         media-src {files} blob: {external}; font-src {files} data: {external}; \
         connect-src {files} {origin}/_stellarr/sdk/ {external}; worker-src {files} blob:; \
         frame-src {frames}; frame-ancestors {origin}; base-uri 'none'; form-action 'none'; object-src 'none'"
    );
    let builder = Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::CONTENT_SECURITY_POLICY, policy)
        .header(header::REFERRER_POLICY, "strict-origin")
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(
            "Permissions-Policy",
            "microphone=(), camera=(), display-capture=()",
        );
    Ok(asset_content(builder, &headers, mime, bytes))
}

pub(super) async fn sandbox_sdk(Path(file): Path<String>) -> ApiResult<Response> {
    let source = match file.as_str() {
        "v1.js" => SDK,
        "media.js" => MEDIA,
        _ => return Err(ApiError::not_found()),
    };
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "text/javascript")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Body::from(source))
        .unwrap())
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
