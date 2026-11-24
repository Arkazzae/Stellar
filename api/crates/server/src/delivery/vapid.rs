//! The instance's push identity, from what the operator configured.

use stellarr_webpush::VapidKeys;

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
