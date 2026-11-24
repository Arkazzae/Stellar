//! The administration panel of the instance, `/api/cockpit`: everything the
//! owner of the instance server does to the installation as a whole rather
//! than to one server.
//!
//! Every route answers 403 to anybody but the owner of the instance server
//! (see [`crate::panel`] for the gate and the order its refusals come in).

pub mod runtime;

use std::sync::{Arc, OnceLock};

use crate::http::Endpoints;
use crate::logging::LogBuffer;

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

pub fn routes() -> Endpoints {
    Endpoints::new()
}
