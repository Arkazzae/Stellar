//! `POST /api/auth/desktop/browser/exchange`: the application trades the
//! request id and the secret it kept for a session of its own, once the person
//! has approved in the browser. It is polled while the person signs in, so a
//! poll that finds nothing yet costs one read.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use stellarr_messages::MessageKey;

use super::no_store;
use crate::auth::desktop_exchange::{Exchanged, exchange_browser_login};
use crate::auth::flow::{self, Context};
use crate::http::{ApiResult, Json, Origin, Problem, json_response, with_session_cookie};
use crate::instance::users::User;
use crate::state::AppState;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Exchange {
    request_id: Option<String>,
    secret: Option<String>,
}

#[derive(Serialize)]
struct Pending {
    status: &'static str,
}

pub async fn exchange(
    State(state): State<AppState>,
    origin: Origin,
    request: Result<Json<Exchange>, crate::http::ApiError>,
) -> ApiResult<Response> {
    // A body that is not the request is refused before the route marks its
    // answer as not to be stored, as the first server's binding was.
    let Json(request) = request?;
    let context = Context {
        ip_address: &origin.address,
        user_agent: &origin.user_agent,
        https: origin.https,
        trusted_device_cookie: None,
        passkeys_available: flow::passkeys_available(&state.settings().operations.public_url),
    };
    let answer = exchange_browser_login(
        &state,
        &context,
        request.request_id.as_deref().unwrap_or(""),
        request.secret.as_deref().unwrap_or(""),
    )
    .await?;
    match answer {
        Exchanged::Pending => Ok(no_store(json_response(
            StatusCode::ACCEPTED,
            &Pending { status: "pending" },
        ))),
        Exchanged::Invalid => {
            let title = state.messages().get(MessageKey::BrowserLoginLinkInvalid, &[])?;
            Ok(no_store(
                Problem::new(StatusCode::BAD_REQUEST).title(title).into_response(),
            ))
        }
        Exchanged::Forbidden => Ok(no_store(StatusCode::FORBIDDEN.into_response())),
        Exchanged::SignedIn(session) => {
            let user = state
                .database()
                .read(move |connection| User::find(connection, session.user).map(|user| (user, session.cookie)))
                .await?;
            let (Some(user), cookie) = (user.0, user.1) else {
                return Ok(no_store(StatusCode::FORBIDDEN.into_response()));
            };
            Ok(with_session_cookie(
                json_response(StatusCode::OK, &user.dto("online")),
                &cookie,
            ))
        }
    }
}
