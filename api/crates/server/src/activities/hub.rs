use axum::response::IntoResponse;
use serde_json::json;
use stellarr_hub::HubError;

use super::live;
use crate::realtime::{Method, Registration, method::arguments};

pub fn registration() -> Registration {
    Registration::new().method(Method::new("ActivityRoom", |cx, args| async move {
        let [path, body] = arguments::exactly(args)?;
        let path = path.as_str().ok_or(HubError::InvalidArguments)?;
        let input = serde_json::from_value(body).map_err(|_| HubError::InvalidArguments)?;
        Ok(match live::hub_room(cx, path.to_owned(), input).await {
            Ok(reply) => reply,
            Err(error) => {
                let status = error.into_response().status();
                json!({"status": status.as_u16(), "error": status.canonical_reason().unwrap_or("Activity request failed")})
            }
        })
    }))
}
