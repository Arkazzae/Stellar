//! The methods that need nothing but the caller: choosing a status, and
//! reading the server's clock.

use serde_json::Value;
use stellarr_hub::HubError;

use super::application::{PresenceUpdate, announce};
use super::audience;
use super::method::{Method, Registration, arguments};
use super::presence::is_supported;
use crate::database::DbError;
use crate::values::Timestamp;

pub fn registration() -> Registration {
    Registration::new()
        .method(Method::new("SetPresence", set_presence))
        .method(Method::new("GetRtcClock", rtc_clock))
}

/// Says which status the account advertises on all its connections, and
/// tells the people who see it. Choosing the status it already has says
/// nothing to anybody, the caller included.
async fn set_presence(cx: super::Context, arguments: Vec<Value>) -> Result<Value, HubError> {
    let [value] = arguments::exactly(arguments)?;
    let Some(value) = arguments::text(value)?.filter(|value| is_supported(value)) else {
        return Err(HubError::visible("Unknown presence."));
    };
    let user = cx.user();
    if !cx.realtime().presence().set_presence(user, &value) {
        return Ok(Value::Null);
    }
    let groups = cx
        .state()
        .database()
        .read(move |db| -> Result<Vec<String>, DbError> {
            Ok(audience::presence_groups(db, user)?)
        })
        .await
        .map_err(|error| HubError::internal(error.to_string()))?;
    let public = cx.realtime().presence().public_presence_of(user, None);
    announce(cx.realtime(), &groups, user, &public);
    cx.send_to_caller(
        "PresenceChanged",
        &PresenceUpdate {
            user_id: user,
            presence: value,
        },
    );
    Ok(Value::Null)
}

/// The server's UTC clock in Unix milliseconds, which clients use to correct
/// for their own skew.
async fn rtc_clock(_: super::Context, arguments: Vec<Value>) -> Result<Value, HubError> {
    let [] = arguments::exactly(arguments)?;
    Ok(Value::from(Timestamp::now().unix_millis()))
}
