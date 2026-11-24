//! Joining a server, through its own door or an invite: what is written, what
//! is refused and what is announced afterwards.

use axum::http::StatusCode;
use stellarr_messages::{MessageKey, Messages};

use super::announce;
use crate::database::DbError;
use crate::http::{ApiError, Problem};
use crate::instance::admission::{self, Admission};
use crate::instance::users::User;
use crate::instance::welcome::Welcome;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// What asking to join came to, as far as the transaction goes.
pub enum Entry {
    /// The person is a member now, or was already one. Either way the
    /// invitations to the server were taken out of their inbox.
    Joined {
        added: bool,
        welcome: Option<Welcome>,
        cleared: usize,
    },
    Banned,
    NoMemberRole,
}

/// Admits `user` to `server` and clears the invitations to it from their
/// inbox, in the transaction of the caller. A person who is a member already
/// gets the same answer as one who just joined.
pub fn enter(
    tx: &rusqlite::Transaction,
    server: Id,
    user: Id,
    now: Timestamp,
    messages: &Messages,
    invited_to: Option<Id>,
) -> Result<Entry, DbError> {
    let account = User::find(tx, user)?
        .ok_or_else(|| DbError::Task("the signed-in account is not there".into()))?;
    let (added, welcome) = match admission::admit(tx, server, &account, now, messages, invited_to)? {
        Admission::Banned => return Ok(Entry::Banned),
        Admission::NoMemberRole => return Ok(Entry::NoMemberRole),
        Admission::AlreadyMember => (false, None),
        Admission::Added(welcome) => (true, welcome),
    };
    let cleared = crate::inbox::clear_joined_invitations(tx, server, user)?;
    Ok(Entry::Joined {
        added,
        welcome,
        cleared,
    })
}

/// What a join left to announce.
pub struct Done {
    pub welcome: Option<Welcome>,
    pub cleared: usize,
}

/// The answer to a refusal, or what is left to announce.
pub fn joined(entry: Entry, state: &AppState) -> Result<Done, ApiError> {
    match entry {
        Entry::Joined {
            welcome, cleared, ..
        } => Ok(Done { welcome, cleared }),
        Entry::Banned => Err(ApiError::forbidden()),
        Entry::NoMemberRole => Err(Problem::new(StatusCode::SERVICE_UNAVAILABLE)
            .title(state.messages().get(MessageKey::ServerHasNoMemberRole, &[])?)
            .into()),
    }
}

/// Tells the hub what the join changed, once it has been committed.
pub async fn publish(state: &AppState, user: Id, server: Id, done: Done) {
    announce::welcomed(state, done.welcome).await;
    if done.cleared > 0 {
        crate::inbox::announce_invitations_changed(state.events(), user, server);
    }
}
