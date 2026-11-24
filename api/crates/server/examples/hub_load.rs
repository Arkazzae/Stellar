//! The server with the hub and the accounts, and one route that publishes an
//! event to everybody, for `tests/realtime-load.mjs` to measure many idle
//! connections and one event to all of them.
//!
//!   hub_load PORT DATA_FOLDER
//!
//! It is a measuring tool and not a server to run: `GET /load/publish` and
//! `/load/setup-code` are open to anyone who can reach it.

use axum::extract::State;
use stellarr_api::events::{Audience, Event};
use stellarr_api::http::Endpoints;
use stellarr_api::realtime::Realtime;
use stellarr_api::settings::Settings;
use stellarr_api::state::AppState;
use stellarr_api::{Services, serve_with};

async fn publish(State(state): State<AppState>) -> String {
    state.events().publish(Event::new(
        Audience::Everyone,
        "Probe",
        &serde_json::json!({"at": "now"}),
    ));
    let connections = state
        .realtime()
        .map_or(0, |hub| hub.hub().connection_count());
    connections.to_string()
}

/// The code that founds the instance, which a server with a log would print.
async fn setup_code(State(state): State<AppState>) -> String {
    state.setup_code().current().unwrap_or_default()
}

#[tokio::main]
async fn main() {
    let mut arguments = std::env::args().skip(1);
    let port = arguments.next().expect("a port");
    let data = arguments.next().expect("a data folder");
    let variables = vec![
        ("STELLARR_ENVIRONMENT".to_string(), "Testing".to_string()),
        (
            "STELLARR_API_LISTEN".to_string(),
            format!("127.0.0.1:{port}"),
        ),
        ("STELLARR_DATA_ROOT".to_string(), data.clone()),
    ];
    let settings = Settings::load(&variables).expect("usable settings");
    let groups = vec![
        stellarr_api::health::routes(),
        stellarr_api::accounts::routes(),
        Endpoints::new()
            .get("/load/publish", publish)
            .get("/load/setup-code", setup_code),
    ];
    let services = Services {
        realtime: Some(Realtime::new()),
        ..Services::default()
    };
    serve_with(
        settings,
        groups,
        services,
        |addresses| println!("listening on {}", addresses[0]),
        async {
            let _ = tokio::signal::ctrl_c().await;
        },
    )
    .await
    .expect("the server ran");
}
