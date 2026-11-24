//! Linking the identity a sign-in was asked to link, once the sign-in is
//! complete.
//!
//! A sign-in through a password can ask to link the identity a provider just
//! confirmed and no account had. The identity waits in a row bound to the
//! browser by a secret in a cookie; the sign-in is finished first and the link
//! comes after, inside the same transaction but apart from it, so that a link
//! that is refused never undoes a sign-in that was right.

use rusqlite::{OptionalExtension, Transaction, params};

use crate::auth::ceremony::Completed;
use crate::auth::{cookies, events, security_cookies, tokens};
use crate::database::DbError;
use crate::http::Origin;
use crate::values::{Id, Timestamp};

/// The cookie that binds the waiting identity to the browser.
fn cookie_name(https: bool) -> &'static str {
    if https {
        "__Host-stellarr-pending-identity"
    } else {
        "stellarr-dev-pending-identity"
    }
}

/// The line that takes the cookie of the waiting identity away.
pub(super) fn clear_cookie(https: bool) -> String {
    cookies::delete(cookie_name(https), https)
}

/// How the link came out, in the words the web's link page reads.
pub(super) const LINKED: &str = "linked";
const TAKEN: &str = "taken";
const OTHER: &str = "other";
pub(super) const FAILED: &str = "failed";

/// What the browser holds for the identity waiting for it, and what the
/// request says about itself, for the event the link writes.
#[derive(Clone)]
pub(super) struct Waiting {
    cookie: Option<(Id, String)>,
    ip_address: String,
    user_agent: String,
}

impl Waiting {
    pub fn of(origin: &Origin) -> Waiting {
        Waiting {
            cookie: security_cookies::parse_bound(
                &cookies::read(&origin.headers, cookie_name(origin.https)).unwrap_or_default(),
            ),
            ip_address: origin.address.clone(),
            user_agent: origin.user_agent.clone(),
        }
    }
}

/// What the instance's providers are called by the people who see them, by
/// id. This build reads no provider from its settings yet (signing in through
/// one is another group's), so no waiting identity can name one that is
/// configured: a link is always refused here, as `failed`, and the identity
/// is used up.
fn provider_name(_provider: &str) -> Option<String> {
    None
}

/// The link that was asked for, as the caller's own writes of a completed
/// sign-in. Nothing at all for a sign-in that asked for none.
pub(super) fn pending(
    tx: &Transaction,
    completed: &Completed,
    waiting: &Waiting,
) -> Result<Option<&'static str>, DbError> {
    let Some(expected) = completed.pending_identity else {
        return Ok(None);
    };
    link(tx, completed.user, expected, waiting, &provider_name).map(Some)
}

/// Gives the identity the browser waits with up for good, where a rollback of
/// the sign-in's own writes cannot bring it back: a link that failed with a
/// database error rolls its delete back with it, and a copy of the cookie
/// must not open the identity again.
pub(super) async fn forget(
    state: &crate::state::AppState,
    waiting: &Waiting,
    expected: Id,
) -> Result<(), DbError> {
    let waiting = waiting.clone();
    state
        .database()
        .write(move |tx| forget_in(tx, &waiting, expected))
        .await
}

fn forget_in(tx: &rusqlite::Connection, waiting: &Waiting, expected: Id) -> Result<(), DbError> {
    let Some((id, secret)) = &waiting.cookie else {
        return Ok(());
    };
    let hash: Option<String> = tx
        .query_row(
            "SELECT \"SecretHash\" FROM \"PendingExternalIdentities\" WHERE \"Id\" = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    if *id == expected && hash.is_some_and(|hash| tokens::matches(secret, &hash)) {
        tx.execute("DELETE FROM \"PendingExternalIdentities\" WHERE \"Id\" = ?1", [id])?;
    }
    Ok(())
}

/// Takes the identity waiting for this browser, if it is the one the sign-in
/// saw, and links it to the account if its provider is still configured.
fn link(
    tx: &Transaction,
    user: Id,
    expected: Id,
    waiting: &Waiting,
    provider_name: &dyn Fn(&str) -> Option<String>,
) -> Result<&'static str, DbError> {
    let now = Timestamp::now();
    let Some((id, secret)) = &waiting.cookie else {
        return Ok(FAILED);
    };
    let found: Option<(String, String, String, String)> = tx
        .query_row(
            "SELECT \"SecretHash\", \"ProviderId\", \"Issuer\", \"Subject\" FROM \"PendingExternalIdentities\"
             WHERE \"Id\" = ?1 AND \"ExpiresAt\" > ?2",
            params![id, now],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((secret_hash, provider, issuer, subject)) = found else {
        return Ok(FAILED);
    };
    // Only one request can take it, and only the one the sign-in saw: one
    // that replaced it since is left where it is.
    if !tokens::matches(secret, &secret_hash) || *id != expected {
        return Ok(FAILED);
    }
    let taken = tx.execute(
        "DELETE FROM \"PendingExternalIdentities\" WHERE \"Id\" = ?1",
        [id],
    )?;
    let Some(name) = provider_name(&provider).filter(|_| taken == 1) else {
        return Ok(FAILED);
    };

    let mut statement = tx.prepare(
        "SELECT \"UserId\", \"Issuer\", \"Subject\" FROM \"ExternalIdentities\"
         WHERE (\"Issuer\" = ?1 AND \"Subject\" = ?2) OR (\"UserId\" = ?3 AND \"Issuer\" = ?1)",
    )?;
    let existing: Vec<(Id, String, String)> = statement
        .query_map(params![issuer, subject, user], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    drop(statement);
    if existing
        .iter()
        .any(|(owner, have_issuer, have_subject)| {
            *have_issuer == issuer && *have_subject == subject && *owner == user
        })
    {
        return Ok(LINKED);
    }
    if !existing.is_empty() {
        return Ok(if existing.iter().any(|(owner, _, _)| *owner != user) {
            TAKEN
        } else {
            OTHER
        });
    }
    let inserted = tx.execute(
        "INSERT INTO \"ExternalIdentities\" (\"Id\", \"UserId\", \"Issuer\", \"Subject\", \"ProviderId\", \"CreatedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![Id::random(), user, issuer, subject, provider, now],
    );
    if let Err(error) = inserted {
        let error = DbError::Sqlite(error);
        // Linked by a request that won the race.
        if error.is_unique_violation() {
            return Ok(TAKEN);
        }
        return Err(error);
    }
    let alert = events::verified_email(tx, user)?;
    events::record(
        tx,
        user,
        events::IDENTITY_LINKED,
        &waiting.ip_address,
        &waiting.user_agent,
        &name,
        alert.as_deref(),
    )?;
    Ok(LINKED)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rusqlite::Connection;

    use super::*;
    use crate::testing::new_database;

    const SECRET: &str = "a secret only the browser has";

    struct World {
        _folder: crate::testing::TempDir,
        connection: Connection,
        user: Id,
        identity: Id,
    }

    fn world() -> World {
        let (folder, database) = new_database();
        let connection = Connection::open(database.path()).unwrap();
        let user = Id::random();
        let now = Timestamp::now();
        connection
            .execute(
                "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"IsDisabled\",
                     \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
                 VALUES (?1, 'maya', 'MAYA', 'Maya', NULL, 0, 1, 0, 'c', ?2)",
                params![user, now],
            )
            .unwrap();
        let mut world = World {
            _folder: folder,
            connection,
            user,
            identity: Id::random(),
        };
        world.wait("subject-1", now.plus(Duration::from_secs(600)));
        world
    }

    impl World {
        /// An identity waiting for a browser that holds `SECRET`.
        fn wait(&mut self, subject: &str, expires: Timestamp) {
            self.connection
                .execute("DELETE FROM \"PendingExternalIdentities\"", [])
                .unwrap();
            self.connection
                .execute(
                    "INSERT INTO \"PendingExternalIdentities\" (\"Id\", \"SecretHash\", \"ProviderId\", \"Issuer\", \"Subject\", \"EmailVerified\",
                         \"Amr\", \"ShortLived\", \"CreatedAt\", \"ExpiresAt\")
                     VALUES (?1, ?2, 'default', 'https://issuer.test', ?3, 0, '[]', 0, ?4, ?5)",
                    params![self.identity, tokens::hash(SECRET), subject, Timestamp::now(), expires],
                )
                .unwrap();
        }

        fn waiting(&self) -> Waiting {
            Waiting {
                cookie: Some((self.identity, SECRET.to_string())),
                ip_address: "203.0.113.9".into(),
                user_agent: "Firefox".into(),
            }
        }

        fn link(&mut self, waiting: &Waiting, expected: Id, provider: Option<&str>) -> &'static str {
            let tx = self.connection.transaction().unwrap();
            let provider = provider.map(str::to_string);
            let outcome = link(&tx, self.user, expected, waiting, &|_| provider.clone()).unwrap();
            tx.commit().unwrap();
            outcome
        }

        fn count(&self, table: &str) -> i64 {
            self.connection
                .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| row.get(0))
                .unwrap()
        }
    }

    #[test]
    fn a_waiting_identity_is_linked_once_and_written_down() {
        let mut world = world();
        let waiting = world.waiting();

        assert_eq!(world.link(&waiting, world.identity, Some("Company")), LINKED);

        assert_eq!(world.count("PendingExternalIdentities"), 0);
        assert_eq!(world.count("ExternalIdentities"), 1);
        let (kind, detail): (String, String) = world
            .connection
            .query_row("SELECT \"Kind\", \"Detail\" FROM \"SecurityEvents\"", [], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        assert_eq!((kind.as_str(), detail.as_str()), ("identity-linked", "Company"));
    }

    #[test]
    fn linking_the_same_identity_again_changes_nothing_and_a_taken_or_other_one_is_said_so() {
        let mut world = world();
        let waiting = world.waiting();
        assert_eq!(world.link(&waiting, world.identity, Some("Company")), LINKED);

        world.wait("subject-1", Timestamp::now().plus(Duration::from_secs(60)));
        assert_eq!(world.link(&waiting, world.identity, Some("Company")), LINKED);
        assert_eq!(world.count("ExternalIdentities"), 1);

        // The account has another identity at the same provider.
        world.wait("subject-2", Timestamp::now().plus(Duration::from_secs(60)));
        assert_eq!(world.link(&waiting, world.identity, Some("Company")), OTHER);

        // Somebody else has the identity.
        let other = Id::random();
        world
            .connection
            .execute(
                "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"IsDisabled\",
                     \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
                 VALUES (?1, 'noor', 'NOOR', 'Noor', NULL, 0, 1, 0, 'c', ?2)",
                params![other, Timestamp::now()],
            )
            .unwrap();
        world
            .connection
            .execute("UPDATE \"ExternalIdentities\" SET \"UserId\" = ?1", [other])
            .unwrap();
        world.wait("subject-1", Timestamp::now().plus(Duration::from_secs(60)));
        assert_eq!(world.link(&waiting, world.identity, Some("Company")), TAKEN);
        assert_eq!(world.count("ExternalIdentities"), 1);
    }

    #[test]
    fn what_cannot_be_taken_fails_the_link_and_only_a_taken_identity_is_used_up() {
        let mut world = world();
        let waiting = world.waiting();

        // No cookie, a wrong secret: nothing is taken.
        let none = Waiting { cookie: None, ..waiting.clone() };
        assert_eq!(world.link(&none, world.identity, Some("Company")), FAILED);
        let wrong = Waiting { cookie: Some((world.identity, "not the secret".into())), ..waiting.clone() };
        assert_eq!(world.link(&wrong, world.identity, Some("Company")), FAILED);
        // An identity that replaced the one the sign-in saw is left where it is.
        assert_eq!(world.link(&waiting, Id::random(), Some("Company")), FAILED);
        assert_eq!(world.count("PendingExternalIdentities"), 1);
        // One that ran out is not there.
        world.wait("subject-1", Timestamp::now().minus(Duration::from_secs(5)));
        assert_eq!(world.link(&waiting, world.identity, Some("Company")), FAILED);
        // A provider that is not configured: used up, and no link.
        world.wait("subject-1", Timestamp::now().plus(Duration::from_secs(60)));
        assert_eq!(world.link(&waiting, world.identity, None), FAILED);
        assert_eq!(world.count("PendingExternalIdentities"), 0);
        assert_eq!(world.count("ExternalIdentities"), 0);
    }

    #[test]
    fn an_identity_whose_link_failed_with_the_database_is_gone_all_the_same() {
        let mut world = world();
        let waiting = world.waiting();
        // The link fails after it took the identity: the table it writes to is gone.
        world.connection.execute_batch("DROP TABLE \"ExternalIdentities\"").unwrap();
        let tx = world.connection.transaction().unwrap();
        tx.execute_batch("SAVEPOINT caller_writes").unwrap();
        let failed = link(&tx, world.user, world.identity, &waiting, &|_| Some("Company".into()));
        assert!(failed.is_err());
        tx.execute_batch("ROLLBACK TO caller_writes; RELEASE caller_writes").unwrap();
        // The rollback brought the row back...
        let kept: i64 = tx.query_row("SELECT COUNT(*) FROM \"PendingExternalIdentities\"", [], |row| row.get(0)).unwrap();
        assert_eq!(kept, 1);
        // ...and giving it up afterwards, as the route does, takes it for good.
        let other = Waiting { cookie: Some((world.identity, "not the secret".into())), ..waiting.clone() };
        forget_in(&tx, &other, world.identity).unwrap();
        forget_in(&tx, &waiting, Id::random()).unwrap();
        assert_eq!(tx.query_row::<i64, _, _>("SELECT COUNT(*) FROM \"PendingExternalIdentities\"", [], |row| row.get(0)).unwrap(), 1);
        forget_in(&tx, &waiting, world.identity).unwrap();
        assert_eq!(tx.query_row::<i64, _, _>("SELECT COUNT(*) FROM \"PendingExternalIdentities\"", [], |row| row.get(0)).unwrap(), 0);
    }

    #[test]
    fn this_build_configures_no_provider_so_a_link_is_refused() {
        let mut world = world();
        let waiting = world.waiting();
        let tx = world.connection.transaction().unwrap();
        let completed = crate::auth::ceremony::Completed {
            user: world.user,
            session: Id::random(),
            security_version: 1,
            short_lived: false,
            evidence: "totp".into(),
            trusted_device_expires_at: None,
            recovery_codes: None,
            pending_identity: Some(world.identity),
            cookies: Vec::new(),
        };

        assert_eq!(pending(&tx, &completed, &waiting).unwrap(), Some(FAILED));
        let asked_none = crate::auth::ceremony::Completed { pending_identity: None, ..completed };
        assert_eq!(pending(&tx, &asked_none, &waiting).unwrap(), None);
    }
}
