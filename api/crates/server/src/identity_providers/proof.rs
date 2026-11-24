//! The proof a change to the account's security settings asks for, when the
//! change is made in a transaction.
//!
//! An account with a second step needs one passed in the last few minutes; one
//! without needs a recent sign-in or its password. The password takes long to
//! check and there is one writer, so it is checked first, against the hash
//! read at that moment, and the transaction relies on the answer only if the
//! stored hash is still that one.

use rusqlite::{Connection, OptionalExtension};

use crate::auth::{Caller, fresh, password};
use crate::database::DbError;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

pub struct Proof {
    hash: Option<String>,
    proves: bool,
}

fn stored_hash(connection: &Connection, user: Id) -> rusqlite::Result<Option<String>> {
    Ok(connection
        .query_row(
            "SELECT \"PasswordHash\" FROM \"Users\" WHERE \"Id\" = ?1",
            [user],
            |row| row.get(0),
        )
        .optional()?
        .flatten())
}

impl Proof {
    /// Checks the candidate, but only when it could decide: when the session's
    /// own freshness already suffices, or the password would not be enough,
    /// nothing is hashed.
    pub async fn check(
        state: &AppState,
        caller: Caller,
        candidate: Option<&str>,
    ) -> Result<Proof, DbError> {
        let none = Proof {
            hash: None,
            proves: false,
        };
        let Some(candidate) = candidate.map(str::to_string) else {
            return Ok(none);
        };
        let (decisive, hash) = state
            .database()
            .read(move |connection| -> Result<(bool, Option<String>), DbError> {
                let now = Timestamp::now();
                let session = Some(caller.session);
                let without =
                    fresh::can_change_security(connection, caller.user, session, || false, now)?;
                let with =
                    fresh::can_change_security(connection, caller.user, session, || true, now)?;
                Ok((!without && with, stored_hash(connection, caller.user)?))
            })
            .await?;
        if !decisive {
            return Ok(none);
        }
        let checked = hash.clone();
        let proves = tokio::task::spawn_blocking(move || {
            password::verify(&candidate, checked.as_deref())
        })
        .await
        .map_err(|error| DbError::Task(error.to_string()))?;
        Ok(Proof { hash, proves })
    }

    /// Whether the session may change the account's security settings now.
    pub fn allows(&self, connection: &Connection, caller: Caller) -> Result<bool, DbError> {
        let stands = self.proves && stored_hash(connection, caller.user)? == self.hash;
        Ok(fresh::can_change_security(
            connection,
            caller.user,
            Some(caller.session),
            || stands,
            Timestamp::now(),
        )?)
    }
}
