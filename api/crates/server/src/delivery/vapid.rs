//! The instance's managed or operator-supplied Web Push identity.

use stellarr_webpush::{GeneratedKeys, VapidKeys, generate_keys};
use crate::database::{Database, DbError};
use crate::seal::Sealer;
use crate::secrets::stored;
use crate::settings::PushSettings;

/// The contact and key pair, when push is on and they are a usable pair. A
/// value that is wrong makes push unavailable and nothing else, so an
/// installation with a typo in one variable still serves every other feature.
pub fn keys(settings: &PushSettings) -> Option<VapidKeys> {
    if !settings.enabled {
        return None;
    }
    VapidKeys::from_config(&settings.subject, &settings.public_key, &settings.private_key).ok()
}

/// Load or create the instance identity before the API and delivery worker start.
/// Explicit or incomplete operator keys always take precedence over the managed pair.
pub(crate) async fn provision(settings: &mut PushSettings, db: &Database, sealer: Sealer) {
    if !settings.enabled || !settings.public_key.is_empty() || !settings.private_key.is_empty() {
        return;
    }
    const ROW: &str = "webpush.vapid";
    const PURPOSE: &str = "webpush-identity";
    let pair = db.write(move |tx| -> Result<GeneratedKeys, DbError> {
        if let Some(pair) = stored::read(tx, &sealer, ROW, PURPOSE)? {
            return Ok(pair);
        }
        let pair = generate_keys().map_err(|e| DbError::Task(e.to_string()))?;
        stored::write(tx, &sealer, ROW, PURPOSE, &pair)?;
        Ok(pair)
    }).await;
    match pair {
        Ok(pair) => {
            settings.public_key = pair.public_key;
            settings.private_key = pair.private_key;
        }
        Err(error) => tracing::warn!(target: super::TARGET, %error, "Cannot load the Web Push identity"),
    }
}
