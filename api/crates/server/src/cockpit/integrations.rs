//! Checks of the outside services the instance connects to: the mail
//! transport and the GIF service.

use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use stellarr_mail::Provider;
use stellarr_messages::MessageKey;
use stellarr_outbound::header::{ACCEPT, USER_AGENT};
use stellarr_outbound::{AddressPolicy, Client, HeaderValue, Request as Outgoing};

use super::settings;
use super::invalid;
use crate::encoding::percent_encode;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Problem, json_response};
use crate::mail::Outcome;
use crate::panel::{self};
use crate::permissions::membership::InstanceOwner;
use crate::secrets;
use crate::state::AppState;

const MAX_MAIL_ADDRESS_LENGTH: usize = 320;
const GIPHY_ADDRESS: &str = "https://api.giphy.com/";
const GIPHY_TIMEOUT: Duration = Duration::from_secs(10);
/// The most of GIPHY's answer that is read: the verdict is its status alone.
const GIPHY_ANSWER_LIMIT: u64 = 64 * 1024;

#[derive(Deserialize)]
struct Recipient {
    to: Option<String>,
}

#[derive(Serialize)]
struct Sent {
    id: String,
    provider: &'static str,
}

/// Sends one message to an address, by the transport the settings and secrets
/// resolve to, so the operator knows mail works before anybody needs it.
pub(super) async fn test_mail(State(state): State<AppState>, request: Request) -> ApiResult<Response> {
    let panel::Bound { server, body, .. } =
        panel::bind_then_gate::<Recipient>(&state, request, false).await?;
    let Some(body) = body else {
        return Err(ApiError::bad_request());
    };
    let messages = *state.messages();
    let recipient = body.to.as_deref().unwrap_or("").trim().to_string();
    // The address must be one, and nothing more: no name beside it, no comment.
    let plain = super::length(&recipient) <= MAX_MAIL_ADDRESS_LENGTH
        && super::address::try_create(&recipient)
            .is_some_and(|parsed| parsed.address == recipient);
    if !plain {
        return Err(invalid(
            "to",
            messages.get(MessageKey::CockpitTestMailRecipientInvalid, &[])?,
        ));
    }
    let reading = state.clone();
    let (row, secrets, name) = state
        .database()
        .read(move |db| -> Result<_, DbError> {
            let row = settings::load(db, &reading, server)?.settings;
            let secrets = secrets::read(db, &reading.settings().secrets, reading.sealer())?;
            let name: String = db.query_row(
                "SELECT \"Name\" FROM \"Servers\" WHERE \"Id\" = ?1",
                [server],
                |row| row.get(0),
            )?;
            Ok((row, secrets, name))
        })
        .await?;
    let transport = settings::transport(&row, &secrets);
    let environment = state.settings();
    let from_address = match row.mail_from_address.trim() {
        "" => environment.mail.from_address.trim().to_string(),
        saved => saved.to_string(),
    };
    if !row.mailing_enabled || from_address.is_empty() || !transport.ready {
        let detail = if transport.provider == Provider::Resend {
            let variable = secrets::RESEND_API_KEY.variable();
            messages.get(MessageKey::CockpitMailNotReadyDetail, &[("variable", &variable)])?
        } else {
            messages.get(MessageKey::CockpitMailNotReadySmtpDetail, &[])?
        };
        return Err(Problem::new(StatusCode::CONFLICT)
            .title(messages.get(MessageKey::CockpitMailNotReadyTitle, &[])?)
            .detail(detail)
            .into());
    }
    let from = if environment.mail.from_name.trim().is_empty() {
        from_address
    } else {
        format!("{} <{from_address}>", environment.mail.from_name.trim())
    };
    let mut id = [0u8; 16];
    aws_lc_rs::rand::fill(&mut id).map_err(|_| ApiError::internal("no random numbers"))?;
    // An address this client cannot write into a message (a quoted part
    // before the @, an address literal, one past 254 bytes) is one the mail
    // cannot go to: the operator is told so, as for any send that fails.
    let message = match stellarr_mail::delivery_test(
        &from,
        &recipient,
        &name,
        &environment.operations.public_url,
        &transport,
        u128::from_be_bytes(id),
        &messages,
    ) {
        Ok(message) => message,
        Err(error) => {
            return Err(Problem::new(StatusCode::BAD_GATEWAY)
                .title(messages.get(MessageKey::CockpitTestMailFailed, &[])?)
                .detail(error.to_string())
                .into());
        }
    };
    match state.mail().send(&message, &transport).await {
        Outcome::Sent { id } => Ok(json_response(
            StatusCode::OK,
            &Sent {
                id,
                provider: transport.provider.name(),
            },
        )),
        Outcome::Refused { reason } | Outcome::TryAgain { reason } => Err(Problem::new(StatusCode::BAD_GATEWAY)
            .title(messages.get(MessageKey::CockpitTestMailFailed, &[])?)
            .detail(reason)
            .into()),
        Outcome::Off | Outcome::NotConfigured => Err(ApiError::internal("mail was ready and is not")),
    }
}

/// What the GIF service said to a key.
#[derive(Debug, PartialEq, Eq)]
enum Check {
    Connected,
    KeyRefused(String),
    Unreachable(String),
}

async fn check_giphy(state: &AppState, key: &str) -> Check {
    let base = state
        .settings()
        .testing_giphy_url
        .as_deref()
        .unwrap_or(GIPHY_ADDRESS);
    let policy = if state.settings().testing_giphy_url.is_some() {
        AddressPolicy::Anywhere
    } else {
        AddressPolicy::PublicOnly
    };
    let url = format!("{base}v1/gifs/trending?api_key={}&limit=1&rating=g", percent_encode(key));
    let request = match Outgoing::get(&url) {
        Ok(request) => request
            .header(USER_AGENT, HeaderValue::from_static("Stellarr/2.0"))
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .timeout(GIPHY_TIMEOUT)
            .max_response_bytes(GIPHY_ANSWER_LIMIT)
            // The key is in the address, so no redirect may carry it on.
            .max_redirects(0)
            .policy(policy),
        Err(error) => return Check::Unreachable(error.to_string()),
    };
    match Client::new().send_collected(request).await {
        Ok(answer) => match answer.status.as_u16() {
            200 => Check::Connected,
            status @ (401 | 403) => Check::KeyRefused(format!("HTTP {status}")),
            status => Check::Unreachable(format!("HTTP {status}")),
        },
        Err(error) if error.is_timeout() => Check::Unreachable("timeout".to_string()),
        Err(error) => Check::Unreachable(error.to_string()),
    }
}

/// Tries the GIF service's key with one small request, so the operator knows
/// it works before people see GIFs.
pub(super) async fn test_giphy(
    State(state): State<AppState>,
    InstanceOwner(_): InstanceOwner,
) -> ApiResult<Response> {
    let reading = state.clone();
    let key = state
        .database()
        .read(move |db| -> Result<_, DbError> {
            let resolved = secrets::read(db, &reading.settings().secrets, reading.sealer())?;
            Ok(resolved.get(&secrets::GIPHY_API_KEY).clone())
        })
        .await?;
    let messages = *state.messages();
    if !key.is_set() {
        let variable = secrets::GIPHY_API_KEY.variable();
        return Err(Problem::new(StatusCode::CONFLICT)
            .title(messages.get(MessageKey::CockpitGiphyKeyMissing, &[("variable", &variable)])?)
            .into());
    }
    let sentence = match check_giphy(&state, key.expose()).await {
        Check::Connected => {
            return Ok(json_response(StatusCode::OK, &serde_json::json!({ "connected": true })));
        }
        Check::KeyRefused(reason) => messages.get(MessageKey::CockpitGiphyKeyRefused, &[("reason", &reason)])?,
        Check::Unreachable(reason) => messages.get(MessageKey::CockpitGiphyUnreachable, &[("reason", &reason)])?,
    };
    // The panel shows the detail, so it carries the whole sentence.
    Err(Problem::new(StatusCode::BAD_GATEWAY)
        .title(sentence.clone())
        .detail(sentence)
        .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_escaped_as_a_component_of_a_query() {
        assert_eq!(percent_encode("aZ09-._~"), "aZ09-._~");
        assert_eq!(percent_encode("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
    }
}
