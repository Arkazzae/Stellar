//! The ways an account signs in on its own: its password, a passkey, or an
//! identity at a provider that is configured. An account made through a
//! provider starts with that identity alone, so taking it away, or the passkey
//! that replaced it, could leave an account nobody can open but through a
//! reset by mail. The settings and the operator's panel ask here first and
//! refuse.
//!
//! The question and the removal that follows must be one transaction: two
//! removals at once would each count the other's way in as left, and both go.
//! A write of the database holds its lock from the start, so the second one
//! waits and counts what the first left; nothing more is needed here.

use rusqlite::{Connection, OptionalExtension, params};
use stellarr_messages::{MessageKey, Messages, MissingValue};

use crate::http::Problem;
use crate::values::Id;

/// The `code` of the problem that refuses to remove the last way in, so a
/// client can say it in its own words.
pub const REFUSAL_CODE: &str = "last-way-in";

/// The 409 that refuses to remove the last way in, with `title` as its
/// sentence.
pub fn refusal(title: String) -> Problem {
    Problem::new(axum::http::StatusCode::CONFLICT)
        .title(title)
        .with("code", REFUSAL_CODE)
}

/// [`refusal`] with the sentence for the passkey that was about to go.
pub fn passkey_refusal(messages: &Messages) -> Result<Problem, MissingValue> {
    Ok(refusal(
        messages.get(MessageKey::SecurityLastWayInPasskey, &[])?,
    ))
}

/// [`refusal`] with the sentence for the identity that was about to go.
pub fn identity_refusal(messages: &Messages) -> Result<Problem, MissingValue> {
    Ok(refusal(
        messages.get(MessageKey::SecurityLastWayInIdentity, &[])?,
    ))
}

/// The providers an instance has configured, in the order they are
/// configured, and the id the one without a name of its own goes by.
#[derive(Debug, Clone, Copy)]
pub struct Providers<'a> {
    pub configured: &'a [String],
    pub default_id: &'a str,
}

impl Providers<'_> {
    /// The provider an identity signs in through: the one it was linked
    /// through, or the original one when the link names none.
    pub fn of<'b>(&'b self, linked_through: Option<&'b str>) -> &'b str {
        linked_through.unwrap_or(self.default_id)
    }
}

/// [`Providers`] with its names owned, so that it can go into a transaction.
#[derive(Debug, Clone)]
pub struct OwnedProviders {
    pub configured: Vec<String>,
    pub default_id: String,
}

impl OwnedProviders {
    pub fn view(&self) -> Providers<'_> {
        Providers {
            configured: &self.configured,
            default_id: &self.default_id,
        }
    }
}

/// Whether the account still has a way in once the identity or the method
/// named is gone. A second step other than a passkey signs nobody in on its
/// own, and an identity at a provider that is no longer configured has no
/// button to press.
pub fn remain(
    connection: &Connection,
    providers: &Providers<'_>,
    user: Id,
    without_identity: Option<Id>,
    without_method: Option<Id>,
) -> rusqlite::Result<bool> {
    // The account is read here, in the transaction the question is asked in.
    let has_password: Option<bool> = connection
        .query_row(
            "SELECT \"PasswordHash\" IS NOT NULL FROM \"Users\" WHERE \"Id\" = ?1",
            [user],
            |row| row.get(0),
        )
        .optional()?;
    if has_password == Some(true) {
        return Ok(true);
    }
    let passkey: bool = connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"MfaMethods\" WHERE \"UserId\" = ?1 AND (?2 IS NULL OR \"Id\" <> ?2) AND \"Type\" = 'passkey'
                        AND \"ConfirmedAt\" IS NOT NULL AND \"RevokedAt\" IS NULL)",
        params![user, without_method],
        |row| row.get(0),
    )?;
    if passkey {
        return Ok(true);
    }
    Ok(!linked_providers(connection, providers, user, without_identity)?.is_empty())
}

/// The configured providers the account signs in through, each once and in
/// the order they are configured. They are also the only ones that can confirm
/// it is the account's owner with a fresh sign-in, so the settings offer
/// exactly these.
pub fn linked_providers(
    connection: &Connection,
    providers: &Providers<'_>,
    user: Id,
    without_identity: Option<Id>,
) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT \"ProviderId\" FROM \"ExternalIdentities\" WHERE \"UserId\" = ?1 AND (?2 IS NULL OR \"Id\" <> ?2)",
    )?;
    let linked: Vec<String> = statement
        .query_map(params![user, without_identity], |row| {
            row.get::<_, Option<String>>(0)
        })?
        .map(|id| id.map(|id| providers.of(id.as_deref()).to_string()))
        .collect::<rusqlite::Result<_>>()?;
    Ok(providers
        .configured
        .iter()
        .filter(|id| linked.contains(id))
        .cloned()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::fixtures::{add_method, add_user, open};

    fn link(connection: &Connection, user: Id, provider: Option<&str>) -> Id {
        let id = Id::random();
        connection
            .execute(
                "INSERT INTO \"ExternalIdentities\" (\"Id\", \"UserId\", \"Issuer\", \"Subject\", \"ProviderId\", \"CreatedAt\")
                 VALUES (?1, ?2, 'https://issuer/' || ?1, ?1, ?3, '2026-11-24 00:00:00')",
                params![id, user, provider],
            )
            .unwrap();
        id
    }

    fn without_password(connection: &Connection, user: Id) {
        connection
            .execute(
                "UPDATE \"Users\" SET \"PasswordHash\" = NULL WHERE \"Id\" = ?1",
                [user],
            )
            .unwrap();
    }

    fn providers(configured: &[String]) -> Providers<'_> {
        Providers {
            configured,
            default_id: "default",
        }
    }

    #[test]
    fn a_password_is_a_way_in_whatever_else_goes() {
        let (_folder, connection) = open();
        let user = add_user(&connection, None);
        let configured = vec!["default".to_string()];

        assert!(remain(&connection, &providers(&configured), user, None, None).unwrap());
    }

    #[test]
    fn an_account_with_nothing_has_no_way_in() {
        let (_folder, connection) = open();
        let user = add_user(&connection, None);
        without_password(&connection, user);
        let configured = vec!["default".to_string()];

        assert!(!remain(&connection, &providers(&configured), user, None, None).unwrap());
    }

    #[test]
    fn a_passkey_is_one_unless_it_is_the_one_going_or_is_not_in_use() {
        let (_folder, connection) = open();
        let user = add_user(&connection, None);
        without_password(&connection, user);
        let configured: Vec<String> = vec![];
        let passkey = add_method(&connection, user, "passkey", None);
        add_method(&connection, user, "totp", None);

        assert!(remain(&connection, &providers(&configured), user, None, None).unwrap());
        assert!(
            !remain(
                &connection,
                &providers(&configured),
                user,
                None,
                Some(passkey)
            )
            .unwrap()
        );
        // A second step that is not a passkey signs nobody in.
        connection
            .execute(
                "UPDATE \"MfaMethods\" SET \"RevokedAt\" = '2026-11-24 00:00:00' WHERE \"Id\" = ?1",
                [passkey],
            )
            .unwrap();
        assert!(!remain(&connection, &providers(&configured), user, None, None).unwrap());
        connection.execute("UPDATE \"MfaMethods\" SET \"RevokedAt\" = NULL, \"ConfirmedAt\" = NULL WHERE \"Id\" = ?1", [passkey]).unwrap();
        assert!(!remain(&connection, &providers(&configured), user, None, None).unwrap());
    }

    #[test]
    fn an_identity_is_one_while_its_provider_is_configured_and_it_is_not_the_one_going() {
        let (_folder, connection) = open();
        let user = add_user(&connection, None);
        without_password(&connection, user);
        let named = link(&connection, user, Some("work"));
        let configured = vec!["default".to_string(), "work".to_string()];

        assert!(remain(&connection, &providers(&configured), user, None, None).unwrap());
        assert!(
            !remain(
                &connection,
                &providers(&configured),
                user,
                Some(named),
                None
            )
            .unwrap()
        );
        // A provider that is not configured any more has no button to press.
        let only_default = vec!["default".to_string()];
        assert!(!remain(&connection, &providers(&only_default), user, None, None).unwrap());
        // An identity linked through no provider is the default one's.
        link(&connection, user, None);
        assert!(
            remain(
                &connection,
                &providers(&only_default),
                user,
                Some(named),
                None
            )
            .unwrap()
        );
    }

    #[test]
    fn the_providers_of_an_account_are_listed_once_each_in_the_order_they_are_configured() {
        let (_folder, connection) = open();
        let user = add_user(&connection, None);
        let other = add_user(&connection, None);
        link(&connection, user, Some("work"));
        link(&connection, user, Some("work"));
        link(&connection, user, None);
        link(&connection, user, Some("gone"));
        link(&connection, other, Some("school"));
        let configured = vec![
            "work".to_string(),
            "school".to_string(),
            "default".to_string(),
        ];

        let listed = linked_providers(&connection, &providers(&configured), user, None).unwrap();

        assert_eq!(listed, vec!["work", "default"]);
    }

    #[test]
    fn the_refusal_is_a_409_with_the_code_a_client_knows_and_a_sentence() {
        let english = stellarr_messages::Messages::new(stellarr_messages::Language::English);

        let json = serde_json::to_string(&passkey_refusal(&english).unwrap()).unwrap();
        let identity = serde_json::to_string(&identity_refusal(&english).unwrap()).unwrap();

        assert!(
            json.starts_with(
                "{\"type\":\"https://tools.ietf.org/html/rfc9110#section-15.5.10\",\"title\":\""
            ),
            "{json}"
        );
        assert!(
            json.contains("\"status\":409,\"code\":\"last-way-in\",\"traceId\""),
            "{json}"
        );
        assert_ne!(
            json.split("\"title\":\"").nth(1),
            identity.split("\"title\":\"").nth(1)
        );
    }
}
