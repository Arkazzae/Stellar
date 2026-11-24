//! End-to-end encryption of direct conversations, as far as the server goes:
//! it keeps public keys, wrapped keys and opaque backups for the clients,
//! decides who may fetch which, and counts the attempts at a PIN. It never
//! sees a key it could use.
//!
//! `/api/me/encryption` is the account's side: its key, the PIN-protected
//! backup of that key, and the browsers that hold keys. `/api/social/
//! conversations/{id}/encryption` is one conversation's: the switch both people
//! agree on, the keys of both, and the epochs that new messages are encrypted
//! with.
//!
//! A device belongs to the session it was registered in and takes part only
//! while that session signs its browser in; see [`store`].

mod account;
mod announce;
mod direct;
mod problems;
mod store;
mod wire;

use crate::http::{Endpoints, RateLimitPolicy};

const CONVERSATION: &str = "/api/social/conversations/{conversation_id}/encryption";

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/me/encryption", account::read)
        .authorized()
        .post("/api/me/encryption/account-key", account::set_up)
        .json_body()
        .rate_limit(RateLimitPolicy::SecurityWrite)
        .authorized()
        .post("/api/me/encryption/account-key/reset", account::reset)
        .json_body()
        .rate_limit(RateLimitPolicy::Login)
        .authorized()
        .get("/api/me/encryption/backup", account::backup_parameters)
        .authorized()
        .put("/api/me/encryption/backup", account::replace_backup)
        .json_body()
        .rate_limit(RateLimitPolicy::Login)
        .authorized()
        .post("/api/me/encryption/backup/unlock", account::unlock)
        .json_body()
        .rate_limit(RateLimitPolicy::PinUnlock)
        .authorized()
        .get("/api/me/encryption/devices", account::list_devices)
        .authorized()
        .post("/api/me/encryption/devices", account::register_device)
        .json_body()
        .authorized()
        .delete("/api/me/encryption/devices/{device_id}", account::revoke_device)
        .authorized()
        .post(&format!("{CONVERSATION}/request"), direct::request)
        .authorized()
        .post(&format!("{CONVERSATION}/cancel"), direct::cancel)
        .authorized()
        .post(&format!("{CONVERSATION}/confirm"), direct::confirm)
        .authorized()
        .post(&format!("{CONVERSATION}/disable"), direct::disable)
        .authorized()
        .get(&format!("{CONVERSATION}/keys"), direct::keys)
        .authorized()
        .get(&format!("{CONVERSATION}/epochs"), direct::epochs)
        .authorized()
        .post(&format!("{CONVERSATION}/epochs"), direct::create_epoch)
        .json_body()
        .authorized()
}
