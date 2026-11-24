//! The administration panel of the instance, `/api/cockpit`: everything the
//! owner of the instance server does to the installation as a whole rather
//! than to one server.
//!
//! Every route answers 403 to anybody but the owner of the instance server
//! (see [`crate::panel`] for the gate and the order its refusals come in). A
//! route that reads a body binds it before it asks the gate, so a body that
//! cannot be read is a 400 for anybody who is signed in; a route with an id in
//! its path reads the id first, so an id that is none is a 404 for anybody
//! the pipeline lets by.

pub mod address;
mod diagnostics;
mod integrations;
mod overview;
mod people;
mod providers;
mod release;
pub mod runtime;
mod secrets;
mod security;
mod servers;
mod sessions;
mod settings;


use std::sync::{Arc, OnceLock};

use axum::response::Response;

use crate::database::DbError;
use crate::http::{ApiError, Endpoints, Problem, no_content};
use crate::logging::LogBuffer;
use crate::panel::FreshInstanceOwner;
use crate::values::Id;

/// What the panel keeps for the whole process: the log buffer it shows and the
/// history of the process's samples.
#[derive(Default)]
pub struct Kept {
    log: OnceLock<Arc<LogBuffer>>,
    history: runtime::History,
}

impl Kept {
    pub fn history(&self) -> &runtime::History {
        &self.history
    }

    /// The buffer of recent log lines; none in a process that was not given
    /// one, where the panel's log is empty.
    pub fn log(&self) -> Option<&Arc<LogBuffer>> {
        self.log.get()
    }

    /// Hands the buffer over, once, when the server starts.
    pub(crate) fn plug_in_log(&self, log: Arc<LogBuffer>) {
        let _ = self.log.set(log);
    }
}

/// A number written without a fraction when it has none, as the first
/// server's writer does.
pub(crate) fn plain_number<S: serde::Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        serializer.serialize_i64(*value as i64)
    } else {
        serializer.serialize_f64(*value)
    }
}

/// The id a request leaves out, which the first server read as the empty one.
pub(crate) fn nobody() -> Id {
    Id::from_bytes([0; 16])
}

/// Whether a failure of the database is a unique value taken meanwhile.
pub(crate) fn taken(error: rusqlite::Error) -> ApiError {
    let error = DbError::from(error);
    if error.is_unique_violation() {
        ApiError::conflict()
    } else {
        ApiError::from(error)
    }
}

/// A field that is refused, with the sentence of the instance's language.
pub(crate) fn invalid(field: &str, text: String) -> ApiError {
    Problem::validation(vec![(field.to_string(), vec![text])]).into()
}

/// The text cut to its length in UTF-16 units, which is how clients count.
pub(crate) fn length(text: &str) -> usize {
    text.encode_utf16().count()
}

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/cockpit/bootstrap", overview::bootstrap)
        .authorized()
        .get("/api/cockpit/activity", overview::activity)
        .authorized()
        .get("/api/cockpit/users", people::list)
        .authorized()
        .post("/api/cockpit/users", people::create)
        .json_body()
        .authorized()
        .patch("/api/cockpit/users/{userId}/role", people::set_role)
        .json_body()
        .authorized()
        .patch("/api/cockpit/users/{userId}/status", people::set_status)
        .json_body()
        .authorized()
        .get("/api/cockpit/users/{userId}/security", security::read)
        .authorized()
        .post("/api/cockpit/users/{userId}/security/recover", security::recover)
        .json_body()
        .authorized()
        .post(
            "/api/cockpit/users/{userId}/identities/{identityId}/remove",
            security::unlink,
        )
        .authorized()
        .post("/api/cockpit/roles", people::create_role)
        .json_body()
        .authorized()
        .patch("/api/cockpit/roles/{roleId}", people::update_role)
        .json_body()
        .authorized()
        .get("/api/cockpit/servers", servers::list)
        .authorized()
        .get("/api/cockpit/servers/{serverId}/members", servers::members)
        .authorized()
        .post("/api/cockpit/servers/{serverId}/transfer", servers::transfer)
        .json_body()
        .authorized()
        .delete("/api/cockpit/servers/{serverId}", servers::delete)
        .authorized()
        .get("/api/cockpit/sessions", sessions::list)
        .authorized()
        .delete("/api/cockpit/sessions/{sessionId}", sessions::revoke)
        .authorized()
        .get("/api/cockpit/runtime", diagnostics::runtime)
        .authorized()
        .get("/api/cockpit/logs", diagnostics::logs)
        .authorized()
        .get("/api/cockpit/logs/export", diagnostics::export)
        .authorized()
        .get("/api/cockpit/fresh-second-step", fresh_second_step)
        .authorized()
        .get("/api/cockpit/settings", settings::read)
        .authorized()
        .put("/api/cockpit/settings", settings::update)
        .json_body()
        .authorized()
        .get("/api/cockpit/desktop-release", release::read)
        .authorized()
        .put("/api/cockpit/desktop-release", release::update)
        .json_body()
        .authorized()
        .post("/api/cockpit/mail/test", integrations::test_mail)
        .json_body()
        .authorized()
        .post("/api/cockpit/integrations/giphy/test", integrations::test_giphy)
        .authorized()
        .get(
            "/api/cockpit/integrations/identity-providers",
            providers::list,
        )
        .authorized()
        .put(
            "/api/cockpit/integrations/identity-providers/{providerId}",
            providers::set_sign_up,
        )
        .json_body()
        .authorized()
        .get("/api/cockpit/secrets", secrets::list)
        .authorized()
        .put("/api/cockpit/secrets/{name}", secrets::set)
        .json_body()
        .authorized()
        .delete("/api/cockpit/secrets/{name}", secrets::clear)
        .authorized()
}

/// Asked before a plain download link is followed, so a refusal opens the
/// confirmation instead of a page of JSON.
async fn fresh_second_step(_: FreshInstanceOwner) -> Response {
    no_content()
}
