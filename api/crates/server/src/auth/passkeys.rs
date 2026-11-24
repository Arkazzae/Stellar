//! Passkeys and security keys, on the side of the application: what is kept
//! between the options a browser is given and its answer, the rows of the
//! credentials, and the checks the WebAuthn library leaves to its caller
//! (see `stellarr_webauthn`): a challenge is used once, a credential id is
//! unique across all accounts, and the signature count is replaced only if it
//! is still the one that was read.
//!
//! Nothing here is sealed, so a passkey set up on the first server works here
//! and the other way round: the credential id, the COSE public key and the
//! handle are stored as they are, the handle in the byte order the first
//! server used (see [`user_handle`]).
//!
//! A passkey can answer three things: a challenge that waits for a second
//! step, an enrolment of a new one, and a sign-in with no password at all.
//! Each has its options kept in a row of its own (`AuthChallenges` for the
//! first two, `PasskeySignIns` for the last), which the first answer takes,
//! right or wrong.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use stellarr_webauthn as webauthn;
use webauthn::{
    AssertionRequest, AssertionResponse, Challenge, CredentialDescriptor, RegistrationResponse,
    RelyingParty, StoredCredential, Transport,
};

use super::events;
use super::flow::{Context, StoredChallenge};
use super::mfa::{self, Outcome, Proof};
use super::mfa_methods::PASSKEY;
use super::security_cookies;
use super::tokens;
use crate::database::DbError;
use crate::values::{Id, Timestamp};

/// A software authenticator, which the library's own tests are written with.
#[cfg(test)]
#[path = "../../../webauthn/tests/support/mod.rs"]
mod authenticator;

/// How long the options of a sign-in without a password stay good: a prompt
/// left open longer starts again.
pub const SIGN_IN_LIFETIME: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// The handle a passkey carries for the account, the same for every passkey of
/// it: the account's id, which says nothing about the person. The first server
/// wrote it as .NET lays a GUID out in memory, with the first three groups
/// byte-swapped, and so does this.
pub fn user_handle(user: Id) -> [u8; 16] {
    let bytes = user.as_bytes();
    let mut handle = [0u8; 16];
    handle[0..4].copy_from_slice(&[bytes[3], bytes[2], bytes[1], bytes[0]]);
    handle[4..6].copy_from_slice(&[bytes[5], bytes[4]]);
    handle[6..8].copy_from_slice(&[bytes[7], bytes[6]]);
    handle[8..].copy_from_slice(&bytes[8..]);
    handle
}

/// A stored credential of an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub id: Id,
    pub method: Id,
    pub user: Id,
    pub credential_id: Vec<u8>,
    pub public_key: Vec<u8>,
    pub user_handle: Vec<u8>,
    pub sign_count: i64,
}

/// The credentials that still count: their method is confirmed and in use.
const ACTIVE: &str = "SELECT c.\"Id\", c.\"MethodId\", c.\"UserId\", c.\"CredentialId\", c.\"PublicKey\", c.\"UserHandle\", c.\"SignCount\", c.\"Transports\"
    FROM \"WebAuthnCredentials\" c JOIN \"MfaMethods\" m ON m.\"Id\" = c.\"MethodId\"
    WHERE m.\"Type\" = 'passkey' AND m.\"ConfirmedAt\" IS NOT NULL AND m.\"RevokedAt\" IS NULL";

fn credential_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<Credential> {
    Ok(Credential {
        id: row.get(0)?,
        method: row.get(1)?,
        user: row.get(2)?,
        credential_id: row.get(3)?,
        public_key: row.get(4)?,
        user_handle: row.get(5)?,
        sign_count: row.get(6)?,
    })
}

/// The transports a stored column names: a JSON list of their names. A list
/// that cannot be read, because it holds a name that is not one, names none.
pub fn read_transports(stored: &str) -> Vec<Transport> {
    let Ok(names) = serde_json::from_str::<Vec<String>>(stored) else {
        return Vec::new();
    };
    let transports: Vec<Transport> = names.iter().filter_map(|n| Transport::parse(n)).collect();
    if transports.len() == names.len() {
        transports
    } else {
        Vec::new()
    }
}

/// The column for the transports a browser reported: a JSON list, or nothing.
pub fn write_transports(transports: &[Transport]) -> String {
    if transports.is_empty() {
        return String::new();
    }
    let names: Vec<&str> = transports.iter().map(|t| t.as_str()).collect();
    serde_json::to_string(&names).expect("a list of names is JSON")
}

/// What the browser is told about the account's passkeys, so that it offers
/// the right ones, or refuses one already registered.
pub fn descriptors(
    connection: &Connection,
    user: Id,
) -> rusqlite::Result<Vec<CredentialDescriptor>> {
    let mut statement = connection.prepare(&format!("{ACTIVE} AND c.\"UserId\" = ?1"))?;
    statement
        .query_map([user], |row| {
            let stored: String = row.get(7)?;
            Ok(CredentialDescriptor {
                id: row.get(3)?,
                transports: read_transports(&stored),
            })
        })?
        .collect()
}

/// The credential with this id, if it still counts and, when an account is
/// named, is that account's.
pub fn find_active(
    connection: &Connection,
    credential_id: &[u8],
    user: Option<Id>,
) -> rusqlite::Result<Option<Credential>> {
    connection
        .query_row(
            &format!("{ACTIVE} AND c.\"CredentialId\" = ?1 AND (?2 IS NULL OR c.\"UserId\" = ?2)"),
            params![credential_id, user],
            credential_of,
        )
        .optional()
}

/// Whether any credential, of any account and of a method in use or not, has
/// this id.
pub fn is_taken(connection: &Connection, credential_id: &[u8]) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"WebAuthnCredentials\" WHERE \"CredentialId\" = ?1)",
        [credential_id],
        |row| row.get(0),
    )
}

/// What the stored options of a ceremony say, which the answer is checked
/// against: the challenge, and the credentials it named.
struct Kept {
    challenge: Vec<u8>,
    allowed: Vec<Vec<u8>>,
}

fn read_kept(options: &str) -> Option<Kept> {
    let value: serde_json::Value = serde_json::from_str(options).ok()?;
    let challenge = webauthn::b64::decode(value.get("challenge")?.as_str()?)?;
    let allowed = match value.get("allowCredentials") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(list) => list
            .as_array()?
            .iter()
            .map(|entry| webauthn::b64::decode(entry.get("id")?.as_str()?))
            .collect::<Option<Vec<_>>>()?,
    };
    Some(Kept { challenge, allowed })
}

/// Keeps the options of a ceremony on the challenge, replacing any it had.
fn keep_on_challenge(tx: &Transaction, challenge: Id, options: &str) -> Result<(), DbError> {
    tx.execute(
        "UPDATE \"AuthChallenges\" SET \"WebAuthnOptions\" = ?2 WHERE \"Id\" = ?1",
        params![challenge, options],
    )?;
    Ok(())
}

/// Takes the options kept on the challenge, once: the first answer gets them,
/// whether it is right or wrong, and the next gets nothing.
fn take_from_challenge(tx: &Transaction, challenge: Id) -> Result<Option<String>, DbError> {
    let kept: Option<Option<String>> = tx
        .query_row(
            "SELECT \"WebAuthnOptions\" FROM \"AuthChallenges\" WHERE \"Id\" = ?1",
            [challenge],
            |row| row.get(0),
        )
        .optional()?;
    let Some(kept) = kept.flatten() else {
        return Ok(None);
    };
    let taken = tx.execute(
        "UPDATE \"AuthChallenges\" SET \"WebAuthnOptions\" = NULL WHERE \"Id\" = ?1 AND \"WebAuthnOptions\" = ?2",
        params![challenge, kept],
    )?;
    Ok((taken == 1).then_some(kept))
}

fn new_challenge() -> Challenge {
    Challenge::random().expect("the system random number generator failed")
}

/// Whether the answer is right for the credential and the options, and, if it
/// is, records the new signature count: false when another request used the
/// count first, so two answers with the same count cannot both pass.
fn verify_and_count(
    tx: &Transaction,
    relying_party: &RelyingParty,
    credential: &Credential,
    kept: &Kept,
    require_user_handle: bool,
    response: &AssertionResponse,
    now: Timestamp,
) -> Result<bool, DbError> {
    let Ok(sign_count) = u32::try_from(credential.sign_count) else {
        return Ok(false);
    };
    let allowed: Vec<&[u8]> = kept.allowed.iter().map(Vec::as_slice).collect();
    let request = AssertionRequest {
        relying_party,
        challenge: &kept.challenge,
        allowed_credentials: &allowed,
        require_user_handle,
    };
    let stored = StoredCredential {
        credential_id: &credential.credential_id,
        public_key: &credential.public_key,
        user_handle: &credential.user_handle,
        sign_count,
    };
    let Ok(assertion) = webauthn::verify_assertion(&request, &stored, response) else {
        return Ok(false);
    };
    let counted = tx.execute(
        "UPDATE \"WebAuthnCredentials\" SET \"SignCount\" = ?3, \"BackedUp\" = ?4, \"LastUsedAt\" = ?5
         WHERE \"Id\" = ?1 AND \"SignCount\" = ?2",
        params![
            credential.id,
            credential.sign_count,
            i64::from(assertion.sign_count),
            assertion.backed_up,
            now
        ],
    )?;
    if counted != 1 {
        return Ok(false);
    }
    tx.execute(
        "UPDATE \"MfaMethods\" SET \"LastUsedAt\" = ?2 WHERE \"Id\" = ?1",
        params![credential.method, now],
    )?;
    Ok(true)
}

/// The answer's credential id, if it can be read.
fn raw_id_of(response: &AssertionResponse) -> Option<Vec<u8>> {
    webauthn::b64::decode(&response.raw_id).filter(|id| !id.is_empty())
}

// ---- answering a challenge that waits for a second step ----

/// What asking for the options of a second step came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Begin {
    /// The options to send to the browser, which are kept on the challenge.
    Options(String),
    /// The challenge does not wait for a second step.
    WrongChallenge,
    /// The account has no passkey.
    Unavailable,
}

/// The options for answering a challenge with one of the account's passkeys.
pub fn begin_second_step(
    tx: &Transaction,
    relying_party: &RelyingParty,
    challenge: &StoredChallenge,
) -> Result<Begin, DbError> {
    if !mfa::answers_with_a_code(challenge) {
        return Ok(Begin::WrongChallenge);
    }
    let credentials = descriptors(tx, challenge.user)?;
    if credentials.is_empty() {
        return Ok(Begin::Unavailable);
    }
    let options = webauthn::assertion_options(relying_party, &credentials, new_challenge())
        .expect("options for a known relying party");
    keep_on_challenge(tx, challenge.id, &options.json)?;
    Ok(Begin::Options(options.json))
}

/// Checks a passkey's answer to the options of a challenge, and spends the
/// options, whatever the answer.
pub fn verify_second_step(
    tx: &Transaction,
    relying_party: &RelyingParty,
    context: &Context<'_>,
    challenge: &StoredChallenge,
    response: Option<&AssertionResponse>,
    now: Timestamp,
) -> Result<Outcome, DbError> {
    if !mfa::answers_with_a_code(challenge) {
        return Ok(Outcome::WrongChallenge);
    }
    if events::failure_budget_spent(tx, challenge.user, now)? {
        return Ok(Outcome::TooManyFailures);
    }
    let kept = take_from_challenge(tx, challenge.id)?.and_then(|json| read_kept(&json));
    let credential = match response.and_then(raw_id_of) {
        Some(raw_id) => find_active(tx, &raw_id, Some(challenge.user))?,
        None => None,
    };
    if let (Some(kept), Some(credential), Some(response)) = (&kept, &credential, response)
        && verify_and_count(tx, relying_party, credential, kept, false, response, now)?
    {
        return Ok(Outcome::Proved(Proof {
            evidence: PASSKEY,
            fresh: true,
            trusted_device: None,
        }));
    }
    mfa::refuse(tx, context, challenge, PASSKEY, now)
}

// ---- adding a passkey ----

/// The options for adding a passkey to the account of a challenge that lets
/// one be added. None for any other challenge. The passkeys the account has
/// are excluded, so one key is not registered twice.
pub fn begin_registration(
    tx: &Transaction,
    relying_party: &RelyingParty,
    challenge: &StoredChallenge,
    username: &str,
    display_name: &str,
) -> Result<Option<String>, DbError> {
    if !mfa::enrols(challenge) {
        return Ok(None);
    }
    let handle = user_handle(challenge.user);
    let exclude = descriptors(tx, challenge.user)?;
    let user = webauthn::User {
        handle: &handle,
        name: username,
        display_name,
    };
    let options = webauthn::creation_options(relying_party, &user, &exclude, new_challenge())
        .expect("options for a known relying party");
    keep_on_challenge(tx, challenge.id, &options.json)?;
    Ok(Some(options.json))
}

/// What checking the answer that registers a passkey came to.
#[derive(Debug)]
pub enum Registered {
    /// Right; [`store`] adds it.
    Ready(webauthn::Registration),
    /// Wrong. An attempt of the challenge was spent and an event written.
    Refused,
    TooManyFailures,
    WrongChallenge,
}

/// Checks the answer to the options of an enrolment, and spends the options,
/// whatever the answer. A credential id that any account has is refused.
pub fn check_registration(
    tx: &Transaction,
    relying_party: &RelyingParty,
    context: &Context<'_>,
    challenge: &StoredChallenge,
    response: Option<&RegistrationResponse>,
    now: Timestamp,
) -> Result<Registered, DbError> {
    if !mfa::enrols(challenge) {
        return Ok(Registered::WrongChallenge);
    }
    if events::failure_budget_spent(tx, challenge.user, now)? {
        return Ok(Registered::TooManyFailures);
    }
    let kept = take_from_challenge(tx, challenge.id)?.and_then(|json| read_kept(&json));
    if let (Some(kept), Some(response)) = (kept, response)
        && let Ok(registration) =
            webauthn::verify_registration(relying_party, &kept.challenge, response)
        && !is_taken(tx, &registration.credential_id)?
    {
        return Ok(Registered::Ready(registration));
    }
    mfa::refuse(tx, context, challenge, "passkey-enrollment", now)?;
    Ok(Registered::Refused)
}

/// Adds the passkey as a confirmed method of the account, with its credential.
/// The id of the method, or none when the credential id is taken. Two
/// requests cannot race here, since a write takes the database's lock before
/// it looks; the unique index on the id still refuses a second copy, and that
/// refusal (see `DbError::is_unique_violation`) rolls everything back.
pub fn store(
    tx: &Transaction,
    user: Id,
    name: &str,
    registration: &webauthn::Registration,
    now: Timestamp,
) -> Result<Option<Id>, DbError> {
    if is_taken(tx, &registration.credential_id)? {
        return Ok(None);
    }
    let method = Id::random();
    tx.execute(
        "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\", \"LastUsedAt\",
             \"ProtectedSecret\", \"EmailAddress\", \"LastTotpStep\", \"RevokedAt\")
         VALUES (?1, ?2, 'passkey', ?3, ?4, ?4, ?4, NULL, NULL, NULL, NULL)",
        params![method, user, name, now],
    )?;
    tx.execute(
        "INSERT INTO \"WebAuthnCredentials\" (\"Id\", \"MethodId\", \"UserId\", \"CredentialId\", \"PublicKey\", \"UserHandle\", \"SignCount\",
             \"BackupEligible\", \"BackedUp\", \"AaGuid\", \"Transports\", \"CreatedAt\", \"LastUsedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
        params![
            Id::random(),
            method,
            user,
            registration.credential_id,
            registration.public_key,
            user_handle(user).as_slice(),
            i64::from(registration.sign_count),
            registration.backup_eligible,
            registration.backed_up,
            Id::from_bytes(registration.aaguid).to_db_text(),
            write_transports(&registration.transports),
            now,
        ],
    )?;
    Ok(Some(method))
}

// ---- signing in with a passkey alone ----

/// The options of a sign-in with no password, and the cookie that binds them
/// to this browser.
#[derive(Clone, PartialEq, Eq)]
pub struct SignInOptions {
    pub json: String,
    /// The `Set-Cookie` line; it carries the secret.
    pub cookie: String,
}

impl std::fmt::Debug for SignInOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignInOptions").finish_non_exhaustive()
    }
}

/// Makes the options of a sign-in. They name no passkey: the browser offers
/// the ones it holds for this site. Options that ran out or were used are
/// swept away on the way.
pub fn begin_sign_in(
    tx: &Transaction,
    relying_party: &RelyingParty,
    https: bool,
    now: Timestamp,
) -> Result<SignInOptions, DbError> {
    tx.execute(
        "DELETE FROM \"PasskeySignIns\" WHERE \"ExpiresAt\" <= ?1 OR \"ConsumedAt\" IS NOT NULL",
        [now],
    )?;
    let options = webauthn::assertion_options(relying_party, &[], new_challenge())
        .expect("options for a known relying party");
    let secret = tokens::random(32);
    let id = Id::random();
    let expires_at = now.plus(SIGN_IN_LIFETIME);
    tx.execute(
        "INSERT INTO \"PasskeySignIns\" (\"Id\", \"SecretHash\", \"Options\", \"CreatedAt\", \"ExpiresAt\", \"ConsumedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
        params![id, tokens::hash(&secret), options.json, now, expires_at],
    )?;
    Ok(SignInOptions {
        json: options.json,
        cookie: security_cookies::set_passkey(https, id, &secret, expires_at),
    })
}

/// What a sign-in with a passkey came to.
#[derive(Debug, PartialEq, Eq)]
pub enum SignIn {
    /// The passkey is the account's and the answer was right. The account is
    /// not disabled and is not waiting for an operator's recovery.
    Accepted { user: Id },
    /// For whatever reason, and the same whatever it was.
    Rejected,
}

/// Checks the answer of a sign-in with a passkey, against the options the
/// browser's cookie names. The options are spent by the first answer, right or
/// wrong. The passkey says which account it is; a known account that is
/// refused gets a failed sign-in written to its events.
pub fn sign_in(
    tx: &Transaction,
    relying_party: &RelyingParty,
    context: &Context<'_>,
    cookie: Option<&(Id, String)>,
    response: Option<&AssertionResponse>,
    now: Timestamp,
) -> Result<SignIn, DbError> {
    let Some((id, secret)) = cookie else {
        return Ok(SignIn::Rejected);
    };
    let row: Option<(String, String)> = tx
        .query_row(
            "SELECT \"SecretHash\", \"Options\" FROM \"PasskeySignIns\" WHERE \"Id\" = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((hash, options)) = row else {
        return Ok(SignIn::Rejected);
    };
    if !tokens::matches(secret, &hash) {
        return Ok(SignIn::Rejected);
    }
    let claimed = tx.execute(
        "UPDATE \"PasskeySignIns\" SET \"ConsumedAt\" = ?2 WHERE \"Id\" = ?1 AND \"ConsumedAt\" IS NULL AND \"ExpiresAt\" > ?2",
        params![id, now],
    )?;
    let Some(response) = response else {
        return Ok(SignIn::Rejected);
    };
    let (Some(raw_id), Some(handle)) = (
        raw_id_of(response),
        response
            .response
            .user_handle
            .as_deref()
            .and_then(webauthn::b64::decode)
            .filter(|handle| !handle.is_empty()),
    ) else {
        return Ok(SignIn::Rejected);
    };
    if claimed != 1 {
        return Ok(SignIn::Rejected);
    }
    // The handle names the account; the key is looked up by its id alone.
    let Some(credential) = find_active(tx, &raw_id, None)? else {
        return Ok(SignIn::Rejected);
    };
    if !tokens::fixed_time_equals(&handle, &credential.user_handle) {
        return Ok(SignIn::Rejected);
    }
    let allowed_to_sign_in: bool = tx.query_row(
        "SELECT NOT \"IsDisabled\" AND NOT \"SecurityRecoveryRequired\" FROM \"Users\" WHERE \"Id\" = ?1",
        [credential.user],
        |row| row.get(0),
    )?;
    let kept = read_kept(&options);
    let right = allowed_to_sign_in
        && match kept {
            Some(kept) => {
                verify_and_count(tx, relying_party, &credential, &kept, true, response, now)?
            }
            None => false,
        };
    if !right {
        events::record_from(
            tx,
            context,
            credential.user,
            events::LOGIN_FAILED,
            "passkey",
            None,
        )?;
        return Ok(SignIn::Rejected);
    }
    Ok(SignIn::Accepted {
        user: credential.user,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use stellarr_webauthn::Algorithm;

    use super::authenticator::{self as soft, AssertionSpec, RegistrationSpec};
    use super::*;
    use crate::auth::fixtures::{add_challenge, add_user, attempts_of, events_of, open};
    use crate::auth::flow::{PURPOSE_ENROLLMENT, PURPOSE_MFA, PURPOSE_REVERIFY};

    fn context() -> Context<'static> {
        Context {
            ip_address: "203.0.113.9",
            user_agent: "Firefox/130",
            https: true,
            trusted_device_cookie: None,
            passkeys_available: true,
        }
    }

    fn challenge_of(options: &str) -> Vec<u8> {
        read_kept(options)
            .expect("options with a challenge")
            .challenge
    }

    /// An account with a passkey of the soft authenticator, registered through
    /// the whole ceremony, and the credential id it has.
    fn enrolled(connection: &mut Connection, user: Id, credential_id: &[u8]) -> Vec<u8> {
        let challenge = add_challenge(connection, user, PURPOSE_ENROLLMENT);
        let tx = connection.transaction().unwrap();
        let options = begin_registration(&tx, &soft::relying_party(), &challenge, "maya", "Maya")
            .unwrap()
            .unwrap();
        let mut spec = RegistrationSpec::new(&challenge_of(&options));
        spec.credential_id = credential_id.to_vec();
        let response = spec.build(soft::key(Algorithm::Es256));
        let Registered::Ready(registration) = check_registration(
            &tx,
            &soft::relying_party(),
            &context(),
            &challenge,
            Some(&response),
            Timestamp::now(),
        )
        .unwrap() else {
            panic!("the registration was right");
        };
        assert!(
            store(&tx, user, "Key", &registration, Timestamp::now())
                .unwrap()
                .is_some()
        );
        tx.commit().unwrap();
        credential_id.to_vec()
    }

    fn assert_with(
        options: &str,
        credential_id: &[u8],
        user: Id,
        sign_count: u32,
    ) -> AssertionResponse {
        let mut spec = AssertionSpec::new(credential_id, &challenge_of(options));
        spec.sign_count = sign_count;
        spec.user_handle = Some(webauthn::b64::encode(&user_handle(user)));
        spec.build(soft::key(Algorithm::Es256))
    }

    fn second_step(
        connection: &mut Connection,
        challenge: &StoredChallenge,
        response: Option<&AssertionResponse>,
    ) -> Outcome {
        let tx = connection.transaction().unwrap();
        let outcome = verify_second_step(
            &tx,
            &soft::relying_party(),
            &context(),
            challenge,
            response,
            Timestamp::now(),
        )
        .unwrap();
        tx.commit().unwrap();
        outcome
    }

    fn options_for(connection: &mut Connection, challenge: &StoredChallenge) -> String {
        let tx = connection.transaction().unwrap();
        let Begin::Options(options) =
            begin_second_step(&tx, &soft::relying_party(), challenge).unwrap()
        else {
            panic!("options were expected");
        };
        tx.commit().unwrap();
        options
    }

    fn proved() -> Outcome {
        Outcome::Proved(Proof {
            evidence: "passkey",
            fresh: true,
            trusted_device: None,
        })
    }

    #[test]
    fn the_handle_is_the_accounts_id_laid_out_as_the_first_server_laid_it() {
        // What the first server's own code gives for this id (Guid.ToByteArray).
        let id = Id::parse("049083c7-a28f-4b91-b21b-2a0920dc1686").unwrap();

        assert_eq!(
            crate::encoding::hex_upper(&user_handle(id)),
            "C78390048FA2914BB21B2A0920DC1686"
        );
    }

    #[test]
    fn transports_are_written_as_the_first_server_wrote_them_and_read_back() {
        let all = [
            Transport::Usb,
            Transport::Nfc,
            Transport::Ble,
            Transport::Hybrid,
            Transport::Internal,
        ];

        assert_eq!(
            write_transports(&all),
            r#"["usb","nfc","ble","hybrid","internal"]"#
        );
        assert_eq!(
            write_transports(&[Transport::SmartCard]),
            r#"["smart-card"]"#
        );
        assert_eq!(write_transports(&[]), "");
        assert_eq!(
            read_transports(r#"["usb","nfc","ble","hybrid","internal"]"#),
            all
        );
        // What cannot be read names none, as in the first server.
        assert_eq!(read_transports(""), vec![]);
        assert_eq!(read_transports("usb,nfc"), vec![]);
        assert_eq!(read_transports(r#"["usb","teleport"]"#), vec![]);
        assert_eq!(read_transports("[]"), vec![]);
    }

    #[test]
    fn a_passkey_is_stored_as_the_first_server_stores_one() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let id = enrolled(&mut connection, user, &[9; 32]);

        fn column<T: rusqlite::types::FromSql>(connection: &Connection, name: &str) -> T {
            connection
                .query_row(
                    &format!("SELECT \"{name}\" FROM \"WebAuthnCredentials\""),
                    [],
                    |row| row.get(0),
                )
                .unwrap()
        }
        let credential_id: Vec<u8> = column(&connection, "CredentialId");
        let key: Vec<u8> = column(&connection, "PublicKey");
        let handle: Vec<u8> = column(&connection, "UserHandle");
        let (count, eligible, backed_up): (i64, bool, bool) = (
            column(&connection, "SignCount"),
            column(&connection, "BackupEligible"),
            column(&connection, "BackedUp"),
        );
        let (aaguid, transports): (String, String) = (
            column(&connection, "AaGuid"),
            column(&connection, "Transports"),
        );

        assert_eq!(credential_id, id);
        assert_eq!(key, soft::key(Algorithm::Es256).cose);
        assert_eq!(handle, user_handle(user));
        assert_eq!((count, eligible, backed_up), (0, false, false));
        assert_eq!(aaguid, "07070707-0707-0707-0707-070707070707");
        assert_eq!(transports, r#"["internal"]"#);
        let (kind, confirmed): (String, bool) = connection
            .query_row("SELECT \"Type\", \"ConfirmedAt\" IS NOT NULL FROM \"MfaMethods\" WHERE \"UserId\" = ?1", [user], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((kind.as_str(), confirmed), ("passkey", true));
        let listed = descriptors(&connection, user).unwrap();
        assert_eq!(
            listed,
            vec![CredentialDescriptor {
                id: vec![9; 32],
                transports: vec![Transport::Internal]
            }]
        );
    }

    #[test]
    fn a_registration_is_refused_for_a_credential_id_any_account_has_even_with_its_method_taken_back()
     {
        let (_folder, mut connection) = open();
        let first = add_user(&connection, None);
        let second = add_user(&connection, None);
        enrolled(&mut connection, first, &[9; 32]);
        connection
            .execute(
                "UPDATE \"MfaMethods\" SET \"RevokedAt\" = ?1",
                [Timestamp::now()],
            )
            .unwrap();
        let challenge = add_challenge(&connection, second, PURPOSE_ENROLLMENT);
        let tx = connection.transaction().unwrap();
        let options = begin_registration(&tx, &soft::relying_party(), &challenge, "lee", "Lee")
            .unwrap()
            .unwrap();
        let mut spec = RegistrationSpec::new(&challenge_of(&options));
        spec.credential_id = vec![9; 32];
        let response = spec.build(soft::key(Algorithm::Es256));

        let checked = check_registration(
            &tx,
            &soft::relying_party(),
            &context(),
            &challenge,
            Some(&response),
            Timestamp::now(),
        )
        .unwrap();

        assert!(matches!(checked, Registered::Refused));
        tx.commit().unwrap();
        assert_eq!(attempts_of(&connection, challenge.id), (1, false));
        assert_eq!(
            events_of(&connection, second, "mfa-failed"),
            vec!["passkey-enrollment"]
        );
        // And storing it directly is refused too.
        let registration = soft::enroll(Algorithm::Es256);
        let tx = connection.transaction().unwrap();
        let taken = webauthn::Registration {
            credential_id: vec![9; 32],
            ..registration
        };
        assert_eq!(
            store(&tx, second, "Key", &taken, Timestamp::now()).unwrap(),
            None
        );
    }

    #[test]
    fn the_options_of_an_enrolment_are_for_enrolling_challenges_and_exclude_what_the_account_has() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        enrolled(&mut connection, user, &[9; 32]);
        let enrolling = add_challenge(&connection, user, PURPOSE_ENROLLMENT);
        let signing_in = add_challenge(&connection, user, PURPOSE_MFA);
        let tx = connection.transaction().unwrap();

        assert_eq!(
            begin_registration(&tx, &soft::relying_party(), &signing_in, "maya", "Maya").unwrap(),
            None
        );
        let options = begin_registration(&tx, &soft::relying_party(), &enrolling, "maya", "Maya")
            .unwrap()
            .unwrap();
        let json: serde_json::Value = serde_json::from_str(&options).unwrap();

        assert_eq!(
            json["excludeCredentials"][0]["id"],
            webauthn::b64::encode(&[9; 32])
        );
        assert_eq!(
            json["user"]["id"],
            webauthn::b64::encode(&user_handle(user))
        );
        assert_eq!(json["user"]["name"], "maya");
        // The options are kept on the challenge for the answer.
        let kept: Option<String> = tx
            .query_row(
                "SELECT \"WebAuthnOptions\" FROM \"AuthChallenges\" WHERE \"Id\" = ?1",
                [enrolling.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept.as_deref(), Some(options.as_str()));
        assert!(matches!(
            check_registration(
                &tx,
                &soft::relying_party(),
                &context(),
                &signing_in,
                None,
                Timestamp::now()
            )
            .unwrap(),
            Registered::WrongChallenge
        ));
    }

    #[test]
    fn a_passkey_answers_the_options_of_a_challenge_once() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let credential = enrolled(&mut connection, user, &[9; 32]);
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);
        let options = options_for(&mut connection, &challenge);
        let json: serde_json::Value = serde_json::from_str(&options).unwrap();
        assert_eq!(
            json["allowCredentials"][0]["id"],
            webauthn::b64::encode(&credential)
        );
        let answer = assert_with(&options, &credential, user, 1);

        assert_eq!(
            second_step(&mut connection, &challenge, Some(&answer)),
            proved()
        );

        let (count, used): (i64, Option<Timestamp>) = connection
            .query_row(
                "SELECT \"SignCount\", \"LastUsedAt\" FROM \"WebAuthnCredentials\"",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert!(used.is_some());
        // The options were taken with the first answer: the same answer again
        // finds none, and costs an attempt.
        assert_eq!(
            second_step(&mut connection, &challenge, Some(&answer)),
            Outcome::Refused
        );
        assert_eq!(attempts_of(&connection, challenge.id), (1, false));
        assert_eq!(events_of(&connection, user, "mfa-failed"), vec!["passkey"]);
    }

    #[test]
    fn a_wrong_answer_spends_the_options_so_the_right_one_cannot_follow_it() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let credential = enrolled(&mut connection, user, &[9; 32]);
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);
        let options = options_for(&mut connection, &challenge);
        let mut spec = AssertionSpec::new(&credential, &challenge_of(&options));
        spec.user_handle = Some(webauthn::b64::encode(&user_handle(user)));
        let forged = spec.build(soft::other_key(Algorithm::Es256));
        let right = assert_with(&options, &credential, user, 1);

        assert_eq!(
            second_step(&mut connection, &challenge, Some(&forged)),
            Outcome::Refused
        );
        assert_eq!(
            second_step(&mut connection, &challenge, Some(&right)),
            Outcome::Refused
        );

        assert_eq!(attempts_of(&connection, challenge.id), (2, false));
        let count: i64 = connection
            .query_row(
                "SELECT \"SignCount\" FROM \"WebAuthnCredentials\"",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn a_signature_count_that_does_not_rise_is_refused_and_two_answers_cannot_both_count() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let credential = enrolled(&mut connection, user, &[9; 32]);
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);
        let options = options_for(&mut connection, &challenge);
        let answer = assert_with(&options, &credential, user, 5);
        let kept = read_kept(&options).unwrap();
        let tx = connection.transaction().unwrap();
        let read = find_active(&tx, &credential, Some(user)).unwrap().unwrap();

        // Another request counted first: the count in the row is not the one read.
        tx.execute("UPDATE \"WebAuthnCredentials\" SET \"SignCount\" = 2", [])
            .unwrap();
        assert!(
            !verify_and_count(
                &tx,
                &soft::relying_party(),
                &read,
                &kept,
                false,
                &answer,
                Timestamp::now()
            )
            .unwrap()
        );
        let count: i64 = tx
            .query_row(
                "SELECT \"SignCount\" FROM \"WebAuthnCredentials\"",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
        // With the count it was read at, it passes once and moves the count.
        tx.execute("UPDATE \"WebAuthnCredentials\" SET \"SignCount\" = 0", [])
            .unwrap();
        assert!(
            verify_and_count(
                &tx,
                &soft::relying_party(),
                &read,
                &kept,
                false,
                &answer,
                Timestamp::now()
            )
            .unwrap()
        );
        assert!(
            !verify_and_count(
                &tx,
                &soft::relying_party(),
                &read,
                &kept,
                false,
                &answer,
                Timestamp::now()
            )
            .unwrap()
        );
        // A count that went down is the library's refusal.
        let again = find_active(&tx, &credential, Some(user)).unwrap().unwrap();
        let lower = assert_with(&options, &credential, user, 3);
        assert!(
            !verify_and_count(
                &tx,
                &soft::relying_party(),
                &again,
                &kept,
                false,
                &lower,
                Timestamp::now()
            )
            .unwrap()
        );
    }

    #[test]
    fn only_a_passkey_of_the_account_that_is_in_use_answers() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let other = add_user(&connection, None);
        let theirs = enrolled(&mut connection, other, &[7; 32]);
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);

        // The account has none: no options.
        let tx = connection.transaction().unwrap();
        assert_eq!(
            begin_second_step(&tx, &soft::relying_party(), &challenge).unwrap(),
            Begin::Unavailable
        );
        tx.commit().unwrap();
        let mine = enrolled(&mut connection, user, &[9; 32]);
        let options = options_for(&mut connection, &challenge);

        // Another account's key, with the options of this one.
        let answer = assert_with(&options, &theirs, other, 1);
        assert_eq!(
            second_step(&mut connection, &challenge, Some(&answer)),
            Outcome::Refused
        );
        // The same account's, once its method is taken back, finds nothing.
        let options = options_for(&mut connection, &challenge);
        let answer = assert_with(&options, &mine, user, 1);
        connection
            .execute(
                "UPDATE \"MfaMethods\" SET \"RevokedAt\" = ?2 WHERE \"UserId\" = ?1",
                params![user, Timestamp::now()],
            )
            .unwrap();
        assert_eq!(
            second_step(&mut connection, &challenge, Some(&answer)),
            Outcome::Refused
        );
        let tx = connection.transaction().unwrap();
        assert_eq!(
            begin_second_step(&tx, &soft::relying_party(), &challenge).unwrap(),
            Begin::Unavailable
        );
    }

    #[test]
    fn a_credential_of_another_account_does_not_answer_even_when_the_options_name_none() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let other = add_user(&connection, None);
        let theirs = enrolled(&mut connection, other, &[7; 32]);
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);
        // Options that name no credential: only the account's own scoping is left.
        let tx = connection.transaction().unwrap();
        let options =
            webauthn::assertion_options(&soft::relying_party(), &[], new_challenge()).unwrap();
        keep_on_challenge(&tx, challenge.id, &options.json).unwrap();
        tx.commit().unwrap();
        let answer = assert_with(&options.json, &theirs, other, 1);

        assert_eq!(
            second_step(&mut connection, &challenge, Some(&answer)),
            Outcome::Refused
        );

        let count: i64 = connection
            .query_row(
                "SELECT \"SignCount\" FROM \"WebAuthnCredentials\"",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn a_missing_answer_is_a_refusal_and_a_challenge_for_something_else_is_not_answered() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        enrolled(&mut connection, user, &[9; 32]);
        let challenge = add_challenge(&connection, user, PURPOSE_REVERIFY);
        options_for(&mut connection, &challenge);

        assert_eq!(
            second_step(&mut connection, &challenge, None),
            Outcome::Refused
        );

        let enrolling = add_challenge(&connection, user, PURPOSE_ENROLLMENT);
        let tx = connection.transaction().unwrap();
        assert_eq!(
            begin_second_step(&tx, &soft::relying_party(), &enrolling).unwrap(),
            Begin::WrongChallenge
        );
        assert_eq!(
            verify_second_step(
                &tx,
                &soft::relying_party(),
                &context(),
                &enrolling,
                None,
                Timestamp::now()
            )
            .unwrap(),
            Outcome::WrongChallenge
        );
    }

    #[test]
    fn an_account_that_failed_too_often_is_not_asked_and_keeps_its_options() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let credential = enrolled(&mut connection, user, &[9; 32]);
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);
        let options = options_for(&mut connection, &challenge);
        let answer = assert_with(&options, &credential, user, 1);
        let tx = connection.transaction().unwrap();
        for _ in 0..10 {
            events::record(&tx, user, events::MFA_FAILED, "", "", "totp", None).unwrap();
        }
        tx.commit().unwrap();

        assert_eq!(
            second_step(&mut connection, &challenge, Some(&answer)),
            Outcome::TooManyFailures
        );

        let kept: Option<String> = connection
            .query_row(
                "SELECT \"WebAuthnOptions\" FROM \"AuthChallenges\" WHERE \"Id\" = ?1",
                [challenge.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(kept.is_some());
        assert_eq!(attempts_of(&connection, challenge.id), (0, false));
    }

    // ---- signing in with a passkey alone ----

    fn sign_in_options(connection: &mut Connection, now: Timestamp) -> (String, (Id, String)) {
        let tx = connection.transaction().unwrap();
        let begun = begin_sign_in(&tx, &soft::relying_party(), true, now).unwrap();
        tx.commit().unwrap();
        let value = begun
            .cookie
            .strip_prefix("__Host-stellarr-passkey=")
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        (begun.json, security_cookies::parse_bound(value).unwrap())
    }

    fn sign_in_as(
        connection: &mut Connection,
        cookie: Option<&(Id, String)>,
        response: Option<&AssertionResponse>,
        now: Timestamp,
    ) -> SignIn {
        let tx = connection.transaction().unwrap();
        let outcome = sign_in(
            &tx,
            &soft::relying_party(),
            &context(),
            cookie,
            response,
            now,
        )
        .unwrap();
        tx.commit().unwrap();
        outcome
    }

    #[test]
    fn a_passkey_signs_its_account_in_once_with_the_options_its_browser_was_given() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let credential = enrolled(&mut connection, user, &[9; 32]);
        let now = Timestamp::now();
        let (options, cookie) = sign_in_options(&mut connection, now);
        let json: serde_json::Value = serde_json::from_str(&options).unwrap();
        assert!(
            json["allowCredentials"]
                .as_array()
                .is_none_or(|list| list.is_empty())
        );
        let answer = assert_with(&options, &credential, user, 1);

        let outcome = sign_in_as(&mut connection, Some(&cookie), Some(&answer), now);

        assert_eq!(outcome, SignIn::Accepted { user });
        assert_eq!(
            sign_in_as(&mut connection, Some(&cookie), Some(&answer), now),
            SignIn::Rejected
        );
        let count: i64 = connection
            .query_row(
                "SELECT \"SignCount\" FROM \"WebAuthnCredentials\"",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn a_sign_in_is_refused_for_a_browser_that_was_not_given_the_options() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let credential = enrolled(&mut connection, user, &[9; 32]);
        let now = Timestamp::now();
        let (options, (id, secret)) = sign_in_options(&mut connection, now);
        let answer = assert_with(&options, &credential, user, 1);

        assert_eq!(
            sign_in_as(&mut connection, None, Some(&answer), now),
            SignIn::Rejected
        );
        assert_eq!(
            sign_in_as(
                &mut connection,
                Some(&(id, "wrong".into())),
                Some(&answer),
                now
            ),
            SignIn::Rejected
        );
        assert_eq!(
            sign_in_as(
                &mut connection,
                Some(&(Id::random(), secret.clone())),
                Some(&answer),
                now
            ),
            SignIn::Rejected
        );
        // A browser that does not know the secret does not use the options up.
        assert_eq!(
            sign_in_as(&mut connection, Some(&(id, secret)), Some(&answer), now),
            SignIn::Accepted { user }
        );
    }

    #[test]
    fn options_that_ran_out_or_were_answered_wrongly_are_no_more_good() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let credential = enrolled(&mut connection, user, &[9; 32]);
        let now = Timestamp::now();
        let (options, cookie) = sign_in_options(&mut connection, now);
        let answer = assert_with(&options, &credential, user, 1);
        let late = now.plus(SIGN_IN_LIFETIME);

        assert_eq!(
            sign_in_as(&mut connection, Some(&cookie), Some(&answer), late),
            SignIn::Rejected
        );

        // A wrong answer uses them up as well.
        let (options, cookie) = sign_in_options(&mut connection, now);
        let mut spec = AssertionSpec::new(&credential, &challenge_of(&options));
        spec.user_handle = Some(webauthn::b64::encode(&user_handle(user)));
        let forged = spec.build(soft::other_key(Algorithm::Es256));
        let right = assert_with(&options, &credential, user, 1);
        assert_eq!(
            sign_in_as(&mut connection, Some(&cookie), Some(&forged), now),
            SignIn::Rejected
        );
        assert_eq!(
            sign_in_as(&mut connection, Some(&cookie), Some(&right), now),
            SignIn::Rejected
        );
        assert_eq!(
            events_of(&connection, user, "login-failed"),
            vec!["passkey"]
        );
    }

    #[test]
    fn a_sign_in_needs_the_handle_and_it_must_be_the_keys() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let other = add_user(&connection, None);
        let credential = enrolled(&mut connection, user, &[9; 32]);
        let now = Timestamp::now();

        for (handle, why) in [
            (None, "no handle"),
            (
                Some(webauthn::b64::encode(&user_handle(other))),
                "another account's handle",
            ),
            (Some(String::new()), "an empty handle"),
        ] {
            let (options, cookie) = sign_in_options(&mut connection, now);
            let mut spec = AssertionSpec::new(&credential, &challenge_of(&options));
            spec.user_handle = handle;
            let answer = spec.build(soft::key(Algorithm::Es256));
            assert_eq!(
                sign_in_as(&mut connection, Some(&cookie), Some(&answer), now),
                SignIn::Rejected,
                "{why}"
            );
        }
        let (_, cookie) = sign_in_options(&mut connection, now);
        assert_eq!(
            sign_in_as(&mut connection, Some(&cookie), None, now),
            SignIn::Rejected
        );
        // A key nobody has: no account is named, so nothing is written for one.
        let (options, cookie) = sign_in_options(&mut connection, now);
        let answer = assert_with(&options, &[1; 32], user, 1);
        assert_eq!(
            sign_in_as(&mut connection, Some(&cookie), Some(&answer), now),
            SignIn::Rejected
        );
        assert!(events_of(&connection, user, "login-failed").is_empty());
    }

    #[test]
    fn a_disabled_account_or_one_waiting_for_recovery_is_not_signed_in_by_its_passkey() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let credential = enrolled(&mut connection, user, &[9; 32]);
        let now = Timestamp::now();

        for column in ["IsDisabled", "SecurityRecoveryRequired"] {
            connection
                .execute(&format!("UPDATE \"Users\" SET \"{column}\" = 1"), [])
                .unwrap();
            let (options, cookie) = sign_in_options(&mut connection, now);
            let answer = assert_with(&options, &credential, user, 1);
            assert_eq!(
                sign_in_as(&mut connection, Some(&cookie), Some(&answer), now),
                SignIn::Rejected,
                "{column}"
            );
            connection
                .execute(&format!("UPDATE \"Users\" SET \"{column}\" = 0"), [])
                .unwrap();
        }
        assert_eq!(
            events_of(&connection, user, "login-failed"),
            vec!["passkey", "passkey"]
        );
        let count: i64 = connection
            .query_row(
                "SELECT \"SignCount\" FROM \"WebAuthnCredentials\"",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn options_that_ran_out_or_were_used_are_swept_when_new_ones_are_made() {
        let (_folder, mut connection) = open();
        let now = Timestamp::now();
        sign_in_options(&mut connection, now.minus(Duration::from_secs(3600)));
        let (_, used) = sign_in_options(&mut connection, now);
        sign_in_as(&mut connection, Some(&used), None, now);
        sign_in_options(&mut connection, now);

        let remaining: i64 = connection
            .query_row("SELECT COUNT(*) FROM \"PasskeySignIns\"", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
    }
}
