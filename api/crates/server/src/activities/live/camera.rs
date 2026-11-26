use super::{Action, LEASE_LIFETIME, Place, Scope, in_session};
use crate::{
    auth::Caller,
    http::{ApiError, ApiResult},
    realtime::{Context, Realtime, throttle::Policy},
    state::AppState,
    values::Id,
    voice::plane::{Opening, open_camera},
};
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) struct CameraGrant {
    realtime: Arc<Realtime>,
    connection: String,
    room: Id,
    id: Id,
}

impl Drop for CameraGrant {
    fn drop(&mut self) {
        self.realtime
            .presence()
            .revoke_camera(&self.connection, self.room, self.id);
    }
}
/// Only the trusted parent has the launch credential. The SFU room is chosen
/// by the server and exists independently of the surrounding voice call.
pub(in crate::activities) async fn hub_camera(
    cx: Context,
    path: String,
    input: Action,
) -> ApiResult<Value> {
    let caller = Caller {
        user: cx.user(),
        session: cx.session(),
    };
    let scope = Scope::from_path(&path, caller.user)?;
    let state = cx.state();
    let (room, grant) = {
        let _gate = state.activities().changes.read().await;
        let opening = in_session(state, scope, caller, &input, &cx.who().origin, |live| {
            if !live
                .installed
                .manifest
                .permissions
                .iter()
                .any(|p| p == "media.cameraTile")
            {
                return Err(ApiError::forbidden());
            }
            let room = live.camera_room;
            let lease = live.leases.get_mut(&caller.user).unwrap();
            match input.action["op"].as_str() {
                Some("close") => {
                    let grant = input.action["grant"]
                        .as_str()
                        .and_then(Id::parse)
                        .ok_or_else(ApiError::bad_request)?;
                    if lease
                        .camera
                        .as_ref()
                        .is_some_and(|g| g.connection == cx.connection_id() && g.id == grant)
                    {
                        lease.camera = None;
                    }
                    Ok(None)
                }
                Some("open") => {
                    if !cx
                        .realtime()
                        .throttle()
                        .try_acquire(cx.connection_id(), Policy::MediaSession)
                    {
                        return Err(ApiError::status(StatusCode::TOO_MANY_REQUESTS));
                    }
                    let id = Id::random();
                    let voice = match scope {
                        Scope::Voice(channel) => Some(channel),
                        Scope::Personal(_) => None,
                    };
                    if !cx.realtime().presence().grant_camera(
                        cx.connection_id(),
                        caller.user,
                        room,
                        voice,
                        id,
                    ) {
                        return Err(ApiError::forbidden());
                    }
                    lease.camera = Some(CameraGrant {
                        realtime: cx.realtime().clone(),
                        connection: cx.connection_id().into(),
                        room,
                        id,
                    });
                    Ok(Some((room, id)))
                }
                _ => Err(ApiError::bad_request()),
            }
        })
        .await?;
        let Some(opening) = opening else {
            return Ok(json!({"value": {}}));
        };
        opening
    };
    tokio::spawn(expire_camera(
        state.clone(),
        (input.install, scope),
        caller.user,
        grant,
    ));
    match open_camera(state, cx.connection_id(), room, caller.user, grant).await {
        Opening::Opened(session) => {
            Ok(json!({"value": {"channelId": room, "sessionId": session, "grant": grant}}))
        }
        Opening::NotSeated => Err(ApiError::forbidden()),
        Opening::Unavailable => {
            cx.realtime()
                .presence()
                .revoke_camera(cx.connection_id(), room, grant);
            Err(ApiError::status(StatusCode::SERVICE_UNAVAILABLE))
        }
    }
}

/// RTP can outlive the last SDK request. Check the current lease at its
/// deadline even when everyone in the room has stopped sending heartbeats.
async fn expire_camera(state: AppState, place: Place, user: Id, grant: Id) {
    let mut remaining = LEASE_LIFETIME;
    loop {
        tokio::time::sleep(remaining).await;
        let rooms = state.activities().rooms.read().unwrap();
        let Some(room) = rooms.get(&place) else {
            return;
        };
        let mut live = room.lock().unwrap();
        let Some(lease) = live.leases.get_mut(&user) else {
            return;
        };
        let Some(camera) = lease.camera.as_ref().filter(|c| c.id == grant) else {
            return;
        };
        if !camera
            .realtime
            .presence()
            .has_camera_grant(&camera.connection, camera.room, grant)
        {
            lease.camera = None;
            return;
        }
        remaining = LEASE_LIFETIME.saturating_sub(lease.touched.elapsed());
        if remaining.is_zero() {
            lease.camera = None;
            return;
        }
    }
}
