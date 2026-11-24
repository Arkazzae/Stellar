//! Remembered devices: a browser that was given a token, once, after a second
//! step, so that the next sign-in need not ask for one. The token goes to the
//! browser in a cookie and only its SHA-256 is stored.
//!
//! A remembered device is a proof for an ordinary sign-in, never for a change
//! that needs a fresh second step, and never for the operator's panel when the
//! policy says so (see `policy`).

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use stellarr_messages::{MessageKey, Messages};

use super::flow::Context;
use super::security_cookies;
use super::tokens;
use crate::auth::policy::SecurityPolicy;
use crate::database::DbError;
use crate::values::{Id, Timestamp};

/// The longest token that is looked at; a longer cookie is not one of ours.
const MAX_TOKEN_LENGTH: usize = 512;

/// A remembered device that still counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedDevice {
    pub id: Id,
    /// The digest stored for the token, which the completing transaction
    /// checks again.
    pub token_hash: String,
    pub expires_at: Timestamp,
}

/// The device whose token the browser sent, if it is the account's, not
/// revoked and not out of date at `now`.
///
/// The row is found by the digest of the token. The token is 256 random bits,
/// so nobody can choose what the digest starts with, and how long the lookup
/// takes says nothing that helps.
pub fn find(
    connection: &Connection,
    user: Id,
    token: Option<&str>,
    now: Timestamp,
) -> rusqlite::Result<Option<TrustedDevice>> {
    let Some(token) =
        token.filter(|token| !token.trim().is_empty() && token.len() <= MAX_TOKEN_LENGTH)
    else {
        return Ok(None);
    };
    connection
        .query_row(
            "SELECT \"Id\", \"TokenHash\", \"ExpiresAt\" FROM \"TrustedDevices\"
             WHERE \"UserId\" = ?1 AND \"TokenHash\" = ?2 AND \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?3",
            params![user, tokens::hash(token), now],
            |row| {
                Ok(TrustedDevice {
                    id: row.get(0)?,
                    token_hash: row.get(1)?,
                    expires_at: row.get(2)?,
                })
            },
        )
        .optional()
}

/// A device that was just remembered, with the cookie that tells the browser.
#[derive(Clone, PartialEq, Eq)]
pub struct Issued {
    pub id: Id,
    /// What the list of devices calls it: the text for an unknown device.
    pub name: String,
    pub expires_at: Timestamp,
    /// The `Set-Cookie` line, which carries the token.
    pub cookie: String,
}

/// Only what is not the token.
impl std::fmt::Debug for Issued {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Issued")
            .field("id", &self.id)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Remembers the browser of this request for as long as the policy says.
pub(super) fn issue(
    tx: &Transaction,
    context: &Context<'_>,
    messages: &Messages,
    user: Id,
    policy: &SecurityPolicy,
    now: Timestamp,
) -> Result<Issued, DbError> {
    let token = tokens::random(32);
    let id = Id::random();
    let unknown = messages
        .get(MessageKey::UnknownDevice, &[])
        .map_err(|error| DbError::Task(error.to_string()))?;
    let ip_address = if context.ip_address.is_empty() {
        "unknown"
    } else {
        context.ip_address
    };
    let expires_at = now.plus(policy.trusted_device_lifetime());
    tx.execute(
        "INSERT INTO \"TrustedDevices\" (\"Id\", \"UserId\", \"TokenHash\", \"Name\", \"Device\", \"IpAddress\", \"CreatedAt\",
             \"ExpiresAt\", \"LastUsedAt\", \"RevokedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?7, NULL)",
        params![
            id,
            user,
            tokens::hash(&token),
            unknown,
            context.device(messages)?,
            ip_address,
            now,
            expires_at
        ],
    )?;
    Ok(Issued {
        id,
        name: unknown,
        expires_at,
        cookie: security_cookies::set_trusted_device(context.https, &token, expires_at),
    })
}

/// Spends the device on the sign-in that is being completed: checks, inside
/// the transaction that completes it, that it is still the account's and still
/// good, as it was when it was found, and notes that it was used. False when
/// it was revoked or has run out in between.
pub(super) fn use_for_sign_in(
    tx: &Transaction,
    user: Id,
    found: &TrustedDevice,
    now: Timestamp,
) -> Result<bool, DbError> {
    let changed = tx.execute(
        "UPDATE \"TrustedDevices\" SET \"LastUsedAt\" = ?4
         WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"TokenHash\" = ?3 AND \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?4",
        params![found.id, user, found.token_hash, now],
    )?;
    Ok(changed == 1)
}

/// Takes the remembered devices of the account back, all of them. How many
/// there were.
pub fn revoke_all(tx: &Transaction, user: Id, now: Timestamp) -> Result<usize, DbError> {
    Ok(tx.execute(
        "UPDATE \"TrustedDevices\" SET \"RevokedAt\" = ?2 WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL",
        params![user, now],
    )?)
}

/// Takes one remembered device of the account back. False when the account has
/// no such device that was not already taken back.
pub fn revoke(tx: &Transaction, user: Id, device: Id, now: Timestamp) -> Result<bool, DbError> {
    let changed = tx.execute(
        "UPDATE \"TrustedDevices\" SET \"RevokedAt\" = ?3 WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"RevokedAt\" IS NULL",
        params![device, user, now],
    )?;
    Ok(changed == 1)
}

/// A remembered device as the list in the account's settings shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: Id,
    pub name: String,
    pub device: String,
    pub ip_address: String,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
    pub last_used_at: Option<Timestamp>,
}

/// The devices that still count, the newest first.
pub fn list(connection: &Connection, user: Id, now: Timestamp) -> rusqlite::Result<Vec<Listed>> {
    let mut statement = connection.prepare(
        "SELECT \"Id\", \"Name\", \"Device\", \"IpAddress\", \"CreatedAt\", \"ExpiresAt\", \"LastUsedAt\" FROM \"TrustedDevices\"
         WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?2 ORDER BY \"CreatedAt\" DESC",
    )?;
    statement
        .query_map(params![user, now], |row| {
            Ok(Listed {
                id: row.get(0)?,
                name: row.get(1)?,
                device: row.get(2)?,
                ip_address: row.get(3)?,
                created_at: row.get(4)?,
                expires_at: row.get(5)?,
                last_used_at: row.get(6)?,
            })
        })?
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use stellarr_messages::Language;

    use super::*;
    use crate::auth::fixtures::{add_user, open};

    fn context(agent: &str) -> Context<'_> {
        Context {
            ip_address: "203.0.113.9",
            user_agent: agent,
            https: true,
            trusted_device_cookie: None,
            passkeys_available: false,
        }
    }

    fn token_of(cookie: &str) -> String {
        cookie
            .strip_prefix("__Host-stellarr-trust=")
            .and_then(|rest| rest.split(';').next())
            .unwrap()
            .to_string()
    }

    fn issued(connection: &mut Connection, user: Id, now: Timestamp) -> Issued {
        let tx = connection.transaction().unwrap();
        let issued = issue(
            &tx,
            &context("Firefox/130"),
            &Messages::new(Language::English),
            user,
            &SecurityPolicy::default(),
            now,
        )
        .unwrap();
        tx.commit().unwrap();
        issued
    }

    #[test]
    fn a_remembered_device_is_found_by_the_token_its_browser_holds_and_only_the_digest_is_stored() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let now = Timestamp::now();

        let issued = issued(&mut connection, user, now);
        let token = token_of(&issued.cookie);

        assert_eq!(token.len(), 43);
        let (hash, name, device, ip): (String, String, String, String) = connection
            .query_row(
                "SELECT \"TokenHash\", \"Name\", \"Device\", \"IpAddress\" FROM \"TrustedDevices\" WHERE \"Id\" = ?1",
                [issued.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(hash, tokens::hash(&token));
        assert_eq!(
            (name.as_str(), device.as_str(), ip.as_str()),
            ("Unknown device", "Firefox/130", "203.0.113.9")
        );
        let found = find(&connection, user, Some(&token), now).unwrap().unwrap();
        assert_eq!(found.id, issued.id);
        assert_eq!(
            found.expires_at,
            now.plus(Duration::from_secs(14 * 24 * 3600))
        );
        assert!(
            issued
                .cookie
                .contains(&format!("expires={}", found.expires_at.to_http_date()))
        );
        assert!(!format!("{issued:?}").contains(&token));
    }

    #[test]
    fn a_token_that_is_not_the_accounts_or_not_good_any_more_finds_nothing() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let other = add_user(&connection, None);
        let now = Timestamp::now();
        let issued = issued(&mut connection, user, now);
        let token = token_of(&issued.cookie);

        assert!(
            find(&connection, other, Some(&token), now)
                .unwrap()
                .is_none()
        );
        assert!(
            find(&connection, user, Some("another"), now)
                .unwrap()
                .is_none()
        );
        assert!(find(&connection, user, None, now).unwrap().is_none());
        assert!(find(&connection, user, Some("  "), now).unwrap().is_none());
        assert!(
            find(
                &connection,
                user,
                Some(&format!("{token}{}", "x".repeat(500))),
                now
            )
            .unwrap()
            .is_none()
        );
        // Out of date: on the second it ends it is no longer one.
        assert!(
            find(&connection, user, Some(&token), issued.expires_at)
                .unwrap()
                .is_none()
        );
        let before = issued.expires_at.minus(Duration::from_secs(1));
        assert!(
            find(&connection, user, Some(&token), before)
                .unwrap()
                .is_some()
        );
        let tx = connection.transaction().unwrap();
        assert!(revoke(&tx, user, issued.id, now).unwrap());
        tx.commit().unwrap();
        assert!(
            find(&connection, user, Some(&token), now)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_cookie_that_is_blank_or_too_long_is_not_looked_up_even_if_a_row_has_its_digest() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let now = Timestamp::now();
        let issued = issued(&mut connection, user, now);
        for (token, why) in [(" ".to_string(), "blank"), ("a".repeat(513), "too long")] {
            connection
                .execute(
                    "UPDATE \"TrustedDevices\" SET \"TokenHash\" = ?2 WHERE \"Id\" = ?1",
                    params![issued.id, tokens::hash(&token)],
                )
                .unwrap();
            assert!(
                find(&connection, user, Some(&token), now)
                    .unwrap()
                    .is_none(),
                "{why}"
            );
        }
        connection
            .execute(
                "UPDATE \"TrustedDevices\" SET \"TokenHash\" = ?2 WHERE \"Id\" = ?1",
                params![issued.id, tokens::hash(&"a".repeat(512))],
            )
            .unwrap();
        assert!(
            find(&connection, user, Some(&"a".repeat(512)), now)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn a_device_is_spent_on_a_sign_in_only_while_it_is_still_the_one_that_was_found() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let now = Timestamp::now();
        let issued = issued(&mut connection, user, now);
        let token = token_of(&issued.cookie);
        let found = find(&connection, user, Some(&token), now).unwrap().unwrap();
        let later = now.plus(Duration::from_secs(60));

        let tx = connection.transaction().unwrap();
        let other = add_user(&tx, None);
        assert!(!use_for_sign_in(&tx, other, &found, later).unwrap());
        let swapped = TrustedDevice {
            token_hash: tokens::hash("else"),
            ..found.clone()
        };
        assert!(!use_for_sign_in(&tx, user, &swapped, later).unwrap());
        assert!(!use_for_sign_in(&tx, user, &found, issued.expires_at).unwrap());
        assert!(use_for_sign_in(&tx, user, &found, later).unwrap());
        let used: Option<Timestamp> = tx
            .query_row(
                "SELECT \"LastUsedAt\" FROM \"TrustedDevices\" WHERE \"Id\" = ?1",
                [found.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(used, Some(later));
        // Revoked in between: the sign-in that found it must not go on.
        assert!(revoke(&tx, user, found.id, later).unwrap());
        assert!(!use_for_sign_in(&tx, user, &found, later).unwrap());
        let used: Option<Timestamp> = tx
            .query_row(
                "SELECT \"LastUsedAt\" FROM \"TrustedDevices\" WHERE \"Id\" = ?1",
                [found.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(used, Some(later));
    }

    #[test]
    fn taking_devices_back_counts_them_and_leaves_other_accounts_alone() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let other = add_user(&connection, None);
        let now = Timestamp::now();
        let first = issued(&mut connection, user, now);
        issued(&mut connection, user, now.plus(Duration::from_secs(1)));
        let theirs = issued(&mut connection, other, now);

        let tx = connection.transaction().unwrap();
        assert!(!revoke(&tx, other, first.id, now).unwrap());
        assert!(!revoke(&tx, user, Id::random(), now).unwrap());
        assert_eq!(revoke_all(&tx, user, now).unwrap(), 2);
        assert_eq!(revoke_all(&tx, user, now).unwrap(), 0);
        assert!(!revoke(&tx, user, first.id, now).unwrap());
        tx.commit().unwrap();

        assert_eq!(list(&connection, user, now).unwrap(), vec![]);
        assert_eq!(list(&connection, other, now).unwrap().len(), 1);
        assert_eq!(list(&connection, other, now).unwrap()[0].id, theirs.id);
    }

    #[test]
    fn the_list_has_the_newest_first_and_leaves_out_what_ran_out() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let now = Timestamp::now();
        let old = issued(
            &mut connection,
            user,
            now.minus(Duration::from_secs(20 * 24 * 3600)),
        );
        let first = issued(&mut connection, user, now.minus(Duration::from_secs(2)));
        let second = issued(&mut connection, user, now.minus(Duration::from_secs(1)));

        let ids: Vec<Id> = list(&connection, user, now)
            .unwrap()
            .into_iter()
            .map(|d| d.id)
            .collect();

        assert_eq!(ids, vec![second.id, first.id]);
        assert!(!ids.contains(&old.id));
    }

    #[test]
    fn a_device_without_a_name_from_the_browser_or_an_address_is_called_unknown() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let tx = connection.transaction().unwrap();
        let context = Context {
            ip_address: "",
            ..context("")
        };

        let issued = issue(
            &tx,
            &context,
            &Messages::new(Language::Polish),
            user,
            &SecurityPolicy::default(),
            Timestamp::now(),
        )
        .unwrap();

        let (device, ip): (String, String) = tx
            .query_row(
                "SELECT \"Device\", \"IpAddress\" FROM \"TrustedDevices\" WHERE \"Id\" = ?1",
                [issued.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(device, issued.name);
        assert_eq!(ip, "unknown");
    }
}
