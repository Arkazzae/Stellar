//! Key material for the tests, generated on the spot.

use aws_lc_rs::agreement::{self, ECDH_P256};

use crate::base64url;

pub struct SubscriptionKeys {
    pub p256dh: [u8; 65],
    pub p256dh_text: String,
    pub auth: [u8; 16],
    pub auth_text: String,
}

pub fn subscription_keys() -> SubscriptionKeys {
    let key = agreement::PrivateKey::generate(&ECDH_P256).unwrap();
    let p256dh = <[u8; 65]>::try_from(key.compute_public_key().unwrap().as_ref()).unwrap();
    let mut auth = [0u8; 16];
    aws_lc_rs::rand::fill(&mut auth).unwrap();
    SubscriptionKeys {
        p256dh,
        p256dh_text: base64url::encode(&p256dh),
        auth,
        auth_text: base64url::encode(&auth),
    }
}
