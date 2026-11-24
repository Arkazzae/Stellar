//! A small server built from the hub crate, for `interop/interop.mjs` to
//! drive over the Stellarr WebSocket protocol. It answers to the same method names
//! as the application's hub for the ones the script calls, and has a few
//! control endpoints for the things only the server side can start.
//!
//!   interop_server PORT
//!
//! Everyone with a cookie `session=NAME` is let in as NAME; anyone else is
//! refused. The control endpoints are open, so this is a test tool and not a
//! server to expose.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use http::StatusCode;
use serde::Serialize;
use serde_json::{Value, json};
use stellarr_hub::{Application, Call, Config, Hub, HubError, Outbound, router};

struct App;

#[derive(Serialize)]
struct PresenceUpdate<'a> {
    #[serde(rename = "userId")]
    user_id: &'a str,
    presence: &'a str,
}

const PRESENCES: [&str; 4] = ["online", "idle", "dnd", "offline"];

impl Application for App {
    type Identity = String;

    async fn connected(&self, call: &Call<String>) -> Result<(), HubError> {
        call.hub()
            .add_to_group(call.connection_id(), &format!("user:{}", call.identity()));
        call.hub().add_to_group(call.connection_id(), "everyone");
        let update = PresenceUpdate {
            user_id: call.identity(),
            presence: "online",
        };
        call.send(
            &Outbound::single("PresenceChanged", &update)
                .map_err(|e| HubError::internal(e.to_string()))?,
        );
        Ok(())
    }

    async fn disconnected(&self, _call: &Call<String>, _error: Option<&str>) {}

    fn has_method(&self, target: &str) -> bool {
        ["getrtcclock", "setpresence", "joinchannel", "echo"]
            .iter()
            .any(|m| m.eq_ignore_ascii_case(target))
    }

    async fn invoke(
        &self,
        call: &Call<String>,
        target: &str,
        arguments: Vec<Value>,
    ) -> Result<Value, HubError> {
        match target.to_ascii_lowercase().as_str() {
            "getrtcclock" => {
                if !arguments.is_empty() {
                    return Err(HubError::InvalidArguments);
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default();
                Ok(json!(now.as_millis() as u64))
            }
            "setpresence" => {
                let [Value::String(value)] = arguments.as_slice() else {
                    return Err(HubError::InvalidArguments);
                };
                if !PRESENCES.contains(&value.as_str()) {
                    return Err(HubError::visible("Unknown presence."));
                }
                let update = PresenceUpdate {
                    user_id: call.identity(),
                    presence: value,
                };
                let message = Outbound::single("PresenceChanged", &update)
                    .map_err(|e| HubError::internal(e.to_string()))?;
                // Every tab of the account, this one included, as the
                // application's own hub does for the account's other tabs.
                call.hub()
                    .send_to_group(&format!("user:{}", call.identity()), &message);
                Ok(Value::Null)
            }
            "joinchannel" => match arguments.as_slice() {
                [Value::String(id)] if id.len() == 36 => Err(HubError::visible("Unknown channel.")),
                [Value::String(_)] => Err(HubError::visible("Invalid channelId.")),
                _ => Err(HubError::InvalidArguments),
            },
            "echo" => Ok(arguments.into_iter().next().unwrap_or(Value::Null)),
            _ => Err(HubError::UnknownMethod),
        }
    }
}

fn query<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request.uri().query()?.split('&').find_map(|pair| {
        pair.split_once('=')
            .filter(|(key, _)| *key == name)
            .map(|(_, v)| v)
    })
}

async fn control(hub: Hub, request: Request) -> Response {
    match request.uri().path() {
        "/control/send" => {
            let group = query(&request, "group").unwrap_or("everyone").to_owned();
            let text = query(&request, "text").unwrap_or("").to_owned();
            let sent = hub.send_to_group(&group, &Outbound::single("Said", &text).unwrap());
            json!({ "sent": sent }).to_string().into_response()
        }
        // The clients are told they may start over, as at a restart.
        "/control/restart" => {
            hub.close_all();
            "ok".into_response()
        }
        "/control/abort" => {
            for id in hub.connection_ids() {
                hub.abort(&id);
            }
            "ok".into_response()
        }
        "/control/end-session" => {
            let user = query(&request, "user").unwrap_or("");
            for id in hub.group_members(&format!("user:{user}")) {
                hub.end_session(&id);
            }
            "ok".into_response()
        }
        "/control/count" => hub.connection_count().to_string().into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    let hub = Hub::new(&Config::default());
    let hub_router = router(
        "/hubs/app",
        Arc::new(App),
        hub.clone(),
        Config::default(),
        |parts| async move {
            let cookie = parts
                .headers
                .get("cookie")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            match cookie
                .split(';')
                .find_map(|part| part.trim().strip_prefix("session="))
            {
                Some(name) if !name.is_empty() => Ok(name.to_owned()),
                _ => Err(Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body(Body::empty())
                    .unwrap()),
            }
        },
    );
    let app = hub_router.route(
        "/control/{*rest}",
        any(move |request: Request| control(hub.clone(), request)),
    );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("the port is free");
    println!("listening on {}", listener.local_addr().unwrap().port());
    axum::serve(listener, app).await.unwrap();
}
