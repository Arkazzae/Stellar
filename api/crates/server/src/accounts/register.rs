//! Creates accounts. The first one makes the instance: a server, its two roles
//! and channels, and an owner, and it takes the setup code from the log. Every
//! later one needs the code of an invite and joins the server that invite
//! belongs to.

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rusqlite::{Connection, Transaction, params};
use serde::Deserialize;
use stellarr_messages::{MessageKey, Messages};
use stellarr_oidc::options::Provider;

use crate::auth::flow::{self, Context, Options, Start};
use crate::auth::policy::{ADMINS, EVERYONE, SecurityPolicy};
use crate::auth::{cookies, password, session};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Origin, Problem, json_response, with_cookie, with_session_cookie};
use crate::identity_providers::{self as providers, Pending, Requester};
use crate::instance::access::{self, MANAGE_SERVER};
use crate::instance::starter::{self, ServerRequest};
use crate::instance::users::User;
use crate::instance::{invites, welcome};
use crate::instance::addresses;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterRequest {
    username: Option<String>,
    display_name: Option<String>,
    email: Option<String>,
    password: Option<String>,
    invite_code: Option<String>,
    initial_server: Option<ServerRequest>,
    setup_code: Option<String>,
    #[serde(default)]
    external: bool,
}

/// The fields of a request that are not right, each with what is wrong, in the
/// order they are reported.
type Errors = Vec<(String, Vec<String>)>;

fn is_username_char(c: char) -> bool {
    // Letters and numbers by general category (L and N), as the first server's
    // rule reads them: a combining vowel sign is neither.
    stellarr_unicode::is_letter_or_number(c) || c == '_' || c == '.' || c == '-'
}

fn validate(request: &RegisterRequest, messages: &Messages) -> Result<Errors, ApiError> {
    let mut errors = Errors::new();
    if !request.external {
        let password = request.password.as_deref();
        let violation = if password::is_too_short(password) {
            Some(messages.get(MessageKey::PasswordTooShort, &[("min", &password::MIN_LENGTH)])?)
        } else if password::is_too_long(password) {
            Some(messages.get(MessageKey::PasswordTooLong, &[("max", &password::MAX_LENGTH)])?)
        } else {
            None
        };
        if let Some(violation) = violation {
            errors.push(("password".into(), vec![violation]));
        }
    }
    let username = request.username.as_deref().unwrap_or("").trim();
    let length = username.encode_utf16().count();
    if !(3..=32).contains(&length) || !username.chars().all(is_username_char) {
        errors.push(("username".into(), vec![messages.get(MessageKey::UsernameInvalid, &[])?]));
    }
    let display_name = request.display_name.as_deref().unwrap_or("").trim();
    if !(1..=64).contains(&display_name.encode_utf16().count()) {
        errors.push(("displayName".into(), vec![messages.get(MessageKey::RegistrationDisplayNameLength, &[])?]));
    }
    if !addresses::is_plain_address(request.email.as_deref().unwrap_or("").trim()) {
        errors.push(("email".into(), vec![messages.get(MessageKey::EmailInvalid, &[])?]));
    }
    Ok(errors)
}

/// Which of the name and the address are already an account's, as the
/// validation problem the person sees.
fn find_taken(
    connection: &Connection,
    username: &str,
    email: &str,
    messages: &Messages,
) -> Result<Errors, ApiError> {
    let mut statement = connection
        .prepare("SELECT \"NormalizedUsername\", \"NormalizedEmail\" FROM \"Users\" WHERE \"NormalizedUsername\" = ?1 OR \"NormalizedEmail\" = ?2")?;
    let existing: Vec<(String, Option<String>)> = statement
        .query_map(params![username, email], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut errors = Errors::new();
    if existing.iter().any(|(name, _)| name == username) {
        errors.push(("username".into(), vec![messages.get(MessageKey::UsernameTaken, &[])?]));
    }
    if existing.iter().any(|(_, address)| address.as_deref() == Some(email)) {
        errors.push(("email".into(), vec![messages.get(MessageKey::EmailTaken, &[])?]));
    }
    Ok(errors)
}

fn invite_problem(message: String) -> ApiError {
    Problem::validation(vec![("inviteCode".into(), vec![message])]).into()
}

/// The identity a provider confirmed for this browser, which the account is
/// made from instead of a password.
#[derive(Clone)]
struct External {
    pending: Pending,
    provider: Provider,
    held: providers::Held,
}

fn identity_taken(connection: &Connection, pending: &Pending) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"ExternalIdentities\" WHERE \"Issuer\" = ?1 AND \"Subject\" = ?2)",
        params![pending.issuer, pending.subject],
        |row| row.get(0),
    )
}

/// The identity got linked to an account meanwhile. It is the person's own, so
/// saying so tells them nothing about anybody else.
fn identity_linked_problem(state: &AppState, provider: &Provider) -> Result<ApiError, ApiError> {
    let name = providers::shown_name(provider, state.messages())?;
    let title = state.messages().get(MessageKey::ProviderIdentityAlreadyLinked, &[("provider", &name)])?;
    Ok(Problem::new(StatusCode::CONFLICT).title(title).into())
}

/// The identity this browser waits with, when it is still there, still of a
/// provider that is set up, and that provider may create accounts.
async fn resolve_external(state: &AppState, origin: &Origin) -> Result<External, ApiError> {
    let held = providers::held_pending(&origin.headers, origin.https);
    let looked = held.clone();
    let found = state
        .database()
        .read(move |connection| -> Result<_, DbError> {
            let Some(pending) = providers::find_pending(connection, looked.as_ref())? else {
                return Ok(None);
            };
            let allowed = providers::sign_up_allowed(connection, &pending.provider_id)?;
            Ok(Some((pending, allowed)))
        })
        .await?;
    let messages = state.messages();
    let provider = found.as_ref().and_then(|(pending, _)| state.settings().oidc.find(&pending.provider_id));
    let (Some((pending, allowed)), Some(provider), Some(held)) = (found.as_ref(), provider, held) else {
        return Err(Problem::new(StatusCode::BAD_REQUEST).title(messages.get(MessageKey::ProviderSignUpExpired, &[])?).into());
    };
    if !allowed {
        let name = providers::shown_name(provider, messages)?;
        return Err(Problem::new(StatusCode::FORBIDDEN)
            .title(messages.get(MessageKey::ProviderSignUpClosed, &[("provider", &name)])?)
            .into());
    }
    Ok(External { pending: pending.clone(), provider: provider.clone(), held })
}

/// Where the new account goes, as far as the checks can tell before anything
/// is written.
enum Plan {
    /// The first account: it makes the instance.
    Founding(starter::Starter),
    /// A later one, with the server its invite belongs to and that server's
    /// role for plain members.
    Joining { invite: Id, server: Id, channel: Option<Id>, role: Id, role_is_owner: bool, role_permissions: i64 },
}

/// Everything that can refuse a registration, read before anything is written.
fn check(
    connection: &Connection,
    state: &AppState,
    request: &RegisterRequest,
    username: &str,
    email: &str,
    external: Option<&External>,
) -> Result<Plan, ApiError> {
    let messages = state.messages();
    // Empty means no account at all. An instance with accounts but no server,
    // which only an edit of the database could leave, is not empty: it takes
    // an invite like any other, and nobody can found it again.
    let founding: bool = connection.query_row("SELECT NOT EXISTS (SELECT 1 FROM \"Users\")", [], |row| row.get(0))?;
    if founding && external.is_some() {
        // The owner's account is made with a password and the setup code.
        return Err(Problem::new(StatusCode::CONFLICT).title(messages.get(MessageKey::ProviderSignUpNotForSetup, &[])?).into());
    }
    if founding && !state.setup_code().matches(request.setup_code.as_deref()) {
        return Err(Problem::validation(vec![("setupCode".into(), vec![messages.get(MessageKey::SetupCodeInvalid, &[])?])]).into());
    }
    let taken = find_taken(connection, &crate::instance::names::to_upper(username), &crate::instance::names::to_upper(email), messages)?;
    if !taken.is_empty() {
        return Err(Problem::validation(taken).into());
    }
    if let Some(external) = external
        && identity_taken(connection, &external.pending)?
    {
        return Err(identity_linked_problem(state, &external.provider)?);
    }
    if founding {
        let configuration = request.initial_server.clone().unwrap_or_else(|| ServerRequest {
            name: Some("Stellarr".into()),
            ..ServerRequest::default()
        });
        return Ok(Plan::Founding(starter::prepare(&configuration, messages)?));
    }
    let Some(code) = request.invite_code.as_deref().filter(|code| !code.trim().is_empty()) else {
        return Err(invite_problem(messages.get(MessageKey::InviteRequired, &[])?));
    };
    let Some(invite) = invites::find_usable(connection, code, Timestamp::now())? else {
        return Err(invite_problem(messages.get(MessageKey::InviteUnusable, &[])?));
    };
    let exists: bool = connection.query_row("SELECT EXISTS (SELECT 1 FROM \"Servers\" WHERE \"Id\" = ?1)", [invite.server], |row| row.get(0))?;
    if !exists {
        return Err(invite_problem(messages.get(MessageKey::InviteServerGone, &[])?));
    }
    // The role plain members get: the system role that is not the owner's,
    // from the highest position value down.
    let role = connection
        .query_row(
            "SELECT \"Id\", \"IsOwner\", \"Permissions\" FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 AND \"IsSystem\" = 1 AND \"IsOwner\" = 0
             ORDER BY \"Position\" DESC, \"Id\" LIMIT 1",
            [invite.server],
            |row| Ok((row.get::<_, Id>(0)?, row.get::<_, bool>(1)?, row.get::<_, i64>(2)?)),
        )
        .map(Some)
        .or_else(|error| if error == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(error) })?;
    let Some((role, role_is_owner, role_permissions)) = role else {
        return Err(Problem::new(StatusCode::SERVICE_UNAVAILABLE)
            .title(messages.get(MessageKey::InstanceHasNoMemberRole, &[])?)
            .into());
    };
    Ok(Plan::Joining { invite: invite.id, server: invite.server, channel: invite.channel, role, role_is_owner, role_permissions })
}

/// What a registration comes to once it is committed.
struct Registered {
    user: User,
    start: Start,
    welcome: Option<welcome::Welcome>,
    founded: bool,
}

/// A hue for the person's avatar, taken from their id: the exclusive or of
/// its four 32-bit words, the first three read as the id's text gives them
/// (the first whole, the next two as 16-bit halves) and the last two
/// little-endian, taken modulo 360. Accounts made before keep the hue this
/// gave them, so it must not change.
fn avatar_hue(id: Id) -> u32 {
    let bytes = id.as_bytes();
    let word = |from: usize| u32::from_le_bytes([bytes[from], bytes[from + 1], bytes[from + 2], bytes[from + 3]]);
    let first = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let second = u32::from(u16::from_be_bytes([bytes[4], bytes[5]])) | u32::from(u16::from_be_bytes([bytes[6], bytes[7]])) << 16;
    (first ^ second ^ word(8) ^ word(12)) % 360
}

fn write(
    tx: &Transaction,
    state: &AppState,
    context: &Context<'_>,
    plan: Plan,
    input: &NewAccount,
) -> Result<Registered, WriteError> {
    let messages = state.messages();
    let now = Timestamp::now();
    // What decided the plan is decided again here, where nothing can change
    // it: the instance has still no account, the invite can still be used.
    match &plan {
        Plan::Founding(_) => {
            let empty: bool = tx.query_row("SELECT NOT EXISTS (SELECT 1 FROM \"Users\")", [], |row| row.get(0))?;
            if !empty {
                return Err(WriteError::Stale);
            }
        }
        Plan::Joining { invite, server, role, .. } => {
            let there: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"ServerRoles\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2)",
                params![role, server],
                |row| row.get(0),
            )?;
            if !there || !invites::record_use(tx, *invite, now)? {
                return Err(WriteError::Stale);
            }
        }
    }
    let id = Id::random();
    tx.execute(
        "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"Email\", \"NormalizedEmail\",
             \"ProviderVerifiedEmail\", \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?10, 0, 0, 0, ?8, ?9)",
        params![
            id,
            input.username,
            crate::instance::names::to_upper(&input.username),
            input.display_name,
            input.password_hash,
            input.email,
            crate::instance::names::to_upper(&input.email),
            format!("oklch(0.58 0.14 {})", avatar_hue(id)),
            now,
            vouched_email(input)
        ],
    )?;
    let user = User::find(tx, id)?.ok_or_else(|| DbError::Task("the account just written is not there".into()))?;
    if let Some(external) = &input.external {
        // A new account is told nothing by mail about its first link.
        let requester = Requester { ip_address: context.ip_address.to_string(), user_agent: context.user_agent.to_string() };
        providers::stage(tx, &requester, id, &external.provider, &external.pending.issuer, &external.pending.subject, false)?;
    }
    // The first account owns the new instance server; anybody else joins it
    // with the base role.
    let (server, role_is_owner, role_permissions, founded, invited_to) = match plan {
        Plan::Founding(starter) => {
            let added = starter::add(tx, &starter, messages, now)?;
            starter::admit(tx, added.id, id, added.owner_role, true, false, now)?;
            (added.id, true, access::ALL, true, None)
        }
        Plan::Joining { server, channel, role, role_is_owner, role_permissions, .. } => {
            starter::admit(tx, server, id, role, role_is_owner, !role_is_owner, now)?;
            (server, role_is_owner, role_permissions, false, channel)
        }
    };
    // An invite to a channel greets the newcomer there, as the first server did.
    let welcome = welcome::create_in(tx, server, &user, now, messages, invited_to)?;

    // The first owner cannot skip an instance-wide requirement by
    // registering: the decision is made against the role just assigned.
    let policy = SecurityPolicy::load(tx)?;
    let required_override = match policy.mfa_requirement {
        EVERYONE => Some(true),
        ADMINS => Some(role_is_owner || role_permissions & MANAGE_SERVER != 0),
        _ => None,
    };
    // An account made through a provider signs in as a sign-in through it
    // would: the provider's second step counts when the instance trusts it and
    // it is still recent now.
    let options = match &input.external {
        None => Options { required_override, ..Options::default() },
        Some(external) => Options {
            required_override,
            short_lived: external.pending.short_lived,
            second_step_at_provider: external.provider.passed_second_step(
                &external.pending.amr.split(',').filter(|value| !value.is_empty()).collect::<Vec<_>>(),
                external.pending.auth_time.as_deref(),
                now.unix_seconds(),
            ),
            ..Options::default()
        },
    };
    let source = if input.external.is_some() { "oidc" } else { "registration" };
    let start = flow::start(tx, context, messages, &user, source, &policy, options)?;
    if let Start::Session(session) = &start {
        session.insert(tx)?;
    }
    if let Some(external) = &input.external {
        // Used up: the identity is the account's now.
        if providers::take_pending(tx, Some(&external.held), Some(external.pending.id))?.is_none() {
            return Err(WriteError::PendingGone);
        }
    }
    Ok(Registered { user, start, welcome, founded })
}

/// Why the write of a registration did not happen.
enum WriteError {
    Db(DbError),
    /// What the checks found is no longer so.
    Stale,
    /// The identity a provider confirmed was used or ran out meanwhile.
    PendingGone,
}

impl From<DbError> for WriteError {
    fn from(error: DbError) -> WriteError {
        WriteError::Db(error)
    }
}

impl From<rusqlite::Error> for WriteError {
    fn from(error: rusqlite::Error) -> WriteError {
        WriteError::Db(error.into())
    }
}

struct NewAccount {
    username: String,
    display_name: String,
    email: String,
    password_hash: Option<String>,
    external: Option<External>,
}

/// The address counts as verified only when it is the one the provider said
/// it verified, not merely one it knows.
fn vouched_email(input: &NewAccount) -> Option<String> {
    let external = input.external.as_ref()?;
    let vouched = external.pending.email.as_deref().filter(|_| external.pending.email_verified)?;
    vouched
        .eq_ignore_ascii_case(&input.email)
        .then(|| crate::instance::names::to_upper(&input.email))
}

pub async fn register(State(state): State<AppState>, origin: Origin, Json(request): Json<RegisterRequest>) -> ApiResult<Response> {
    let messages = state.messages();
    let errors = validate(&request, messages)?;
    if !errors.is_empty() {
        return Err(Problem::validation(errors).into());
    }
    let external = if request.external { Some(resolve_external(&state, &origin).await?) } else { None };
    let username = request.username.as_deref().unwrap_or("").trim().to_string();
    let email = request.email.as_deref().unwrap_or("").trim().to_string();

    // Whether the instance is still empty is read and acted on inside one
    // critical section, so two first registrations cannot both found it. The
    // gate holds only against other registrations of this process, so what
    // decides is read again inside the transaction that writes, and a plan
    // that has gone stale is made again from the checks.
    let _turn = state.registration_gate().lock().await;
    let request = std::sync::Arc::new(request);
    let mut password_hash: Option<String> = None;
    let mut attempts = 0;
    let registered = loop {
        let plan = {
            let (state, request, username, email) = (state.clone(), request.clone(), username.clone(), email.clone());
            let database = state.database().clone();
            database.read({ let external = external.clone(); move |connection| check(connection, &state, &request, &username, &email, external.as_ref()) }).await?
        };
        let hash = match &password_hash {
            _ if external.is_some() => None,
            Some(hash) => Some(hash.clone()),
            None => {
                let password = request.password.clone().unwrap_or_default();
                let made = tokio::task::spawn_blocking(move || password::hash(&password)).await.map_err(ApiError::internal)?;
                password_hash = Some(made.clone());
                Some(made)
            }
        };
        let input = NewAccount {
            username: username.clone(),
            display_name: request.display_name.as_deref().unwrap_or("").trim().to_string(),
            email: email.clone(),
            password_hash: hash,
            external: external.clone(),
        };
        let trusted_device_cookie = cookies::read(&origin.headers, trusted_device_name(origin.https));
        let (context_state, context_origin) = (state.clone(), origin.clone());
        let outcome = state
            .database()
            .clone()
            .write(move |tx| {
                let context = Context {
                    ip_address: &context_origin.address,
                    user_agent: &context_origin.user_agent,
                    https: context_origin.https,
                    trusted_device_cookie: trusted_device_cookie.as_deref(),
                    passkeys_available: flow::passkeys_available(&context_state.settings().operations.public_url),
                };
                write(tx, &context_state, &context, plan, &input)
            })
            .await;
        match outcome {
            Ok(registered) => break registered,
            // What the checks found has changed since: look again.
            Err(WriteError::PendingGone) => {
                return Err(Problem::new(StatusCode::BAD_REQUEST).title(messages.get(MessageKey::ProviderSignUpExpired, &[])?).into());
            }
            Err(WriteError::Stale) if attempts < 3 => attempts += 1,
            Err(WriteError::Stale) => return Err(ApiError::internal("the instance kept changing under a registration")),
            // Somebody else wrote the same name or address after our check:
            // answer as the check would have, not with a server error.
            Err(WriteError::Db(error)) if error.is_unique_violation() => {
                let (taken, identity) = state
                    .database()
                    .read({
                        let (state, username, email, external) = (state.clone(), username.clone(), email.clone(), external.clone());
                        move |connection| -> Result<_, ApiError> {
                            let taken = find_taken(connection, &crate::instance::names::to_upper(&username), &crate::instance::names::to_upper(&email), state.messages())?;
                            let identity = match &external {
                                Some(external) => identity_taken(connection, &external.pending)?,
                                None => false,
                            };
                            Ok((taken, identity))
                        }
                    })
                    .await?;
                if !taken.is_empty() {
                    return Err(Problem::validation(taken).into());
                }
                if let Some(external) = external.as_ref().filter(|_| identity) {
                    return Err(identity_linked_problem(&state, &external.provider)?);
                }
                return Err(error.into());
            }
            Err(WriteError::Db(error)) => return Err(error.into()),
        }
    };

    if registered.founded {
        // The instance has its owner. Later registrations do not ask for the
        // code, and none could pass with it anyway: it is gone.
        state.setup_code().forget();
    }
    // The account exists from here on: a notice that cannot be announced is
    // logged, and the registration still succeeded.
    if let Some(welcome) = &registered.welcome {
        let announced = state
            .database()
            .read({
                let welcome = welcome.clone();
                move |connection| -> Result<_, DbError> { Ok(welcome::announcement(connection, &welcome)?) }
            })
            .await;
        match announced {
            Ok(event) => state.events().publish(event),
            Err(error) => tracing::warn!("the welcome of a new account could not be announced: {error}"),
        }
    }
    let short_lived = external.as_ref().is_some_and(|external| external.pending.short_lived);
    created(&state, &origin, registered, external.is_some(), short_lived)
}

/// Takes the cookie of the identity a registration used up away.
fn forget_identity(response: Response, origin: &Origin, through_provider: bool) -> Response {
    if through_provider {
        with_cookie(response, &providers::clear_pending_cookie(origin.https))
    } else {
        response
    }
}

pub fn trusted_device_name(https: bool) -> &'static str {
    if https { "__Host-stellarr-trust" } else { "stellarr-dev-trust" }
}

/// What the browser is told of a new account. One made through a provider
/// keeps the choice made before leaving for it, and takes the cookie of the
/// identity it used up away.
fn created(state: &AppState, origin: &Origin, registered: Registered, through_provider: bool, short_lived: bool) -> ApiResult<Response> {
    let location = HeaderValue::from_static("/api/me");
    match registered.start {
        Start::Session(new) => {
            // A registration with a password offers no choice and the account
            // stays signed in.
            let cookie = session::sign_in_cookie(
                state.sealer(),
                &state.settings().environment,
                origin.https,
                registered.user.id,
                registered.user.session_version,
                new.id,
                short_lived,
            );
            let mut response = json_response(StatusCode::CREATED, &registered.user.dto("online"));
            response.headers_mut().insert(header::LOCATION, location);
            let response = with_session_cookie(response, &cookie);
            Ok(forget_identity(response, origin, through_provider))
        }
        Start::Challenge { body, cookie } => {
            let mut response = json_response(StatusCode::CREATED, &body);
            response.headers_mut().insert(header::LOCATION, location);
            Ok(forget_identity(with_cookie(response, &cookie), origin, through_provider))
        }
        // A new account is never in need of recovery.
        Start::RecoveryRequired => Ok(StatusCode::CONFLICT.into_response()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_plan_that_went_stale_before_the_write_is_not_carried_out() {
        use crate::events::Unconnected;
        use crate::seal::Sealer;
        use crate::settings::Settings;
        use crate::testing::new_database;
        use std::sync::Arc;

        let (_folder, database) = new_database();
        let settings = Settings::load(&[("ASPNETCORE_ENVIRONMENT".into(), "Testing".into())]).unwrap();
        let state = AppState::new(settings, database.clone(), Arc::new(Unconnected), Arc::new(Unconnected), Sealer::from_key([1; 32]));
        let messages = state.messages();
        let starter = starter::prepare(&starter::ServerRequest { name: Some("Crew".into()), ..Default::default() }, messages).ok().unwrap();
        let input = || NewAccount { username: "maya".into(), display_name: "Maya".into(), email: "maya@example.test".into(), password_hash: None, external: None };
        // Somebody else has founded the instance since the check.
        let other = Id::random();
        database
            .write(move |tx| -> Result<(), DbError> {
                tx.execute(
                    "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"IsDisabled\", \"SessionVersion\",
                         \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\") VALUES (?1, 'x', 'X', 'X', 0, 0, 0, 'c', '2026-11-24 00:00:00')",
                    [other],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        let refused = database
            .write(move |tx| {
                let context = Context { ip_address: "1.2.3.4", user_agent: "", https: false, trusted_device_cookie: None, passkeys_available: false };
                write(tx, &state, &context, Plan::Founding(starter), &input()).map(|_| ())
            })
            .await;

        assert!(matches!(refused, Err(WriteError::Stale)));
        let accounts: i64 = rusqlite::Connection::open(database.path()).unwrap().query_row("SELECT count(*) FROM \"Users\"", [], |r| r.get(0)).unwrap();
        assert_eq!(accounts, 1);
    }

    #[test]
    fn the_hue_is_the_hash_code_of_the_guid() {
        // Ids and hues the first server gave accounts.
        for (id, hue) in [
            ("2bb89c37-c5f0-436a-84b7-a23c5b25ee9e", 272),
            ("049083c7-a28f-4b91-b21b-2a0920dc1686", 146),
        ] {
            assert_eq!(avatar_hue(Id::parse(id).unwrap()), hue, "{id}");
        }
    }

    #[test]
    fn names_are_stored_capitalized_by_the_simple_mapping_of_the_first_server() {
        assert_eq!(crate::instance::names::to_upper("Maya.Lin"), "MAYA.LIN");
        assert_eq!(crate::instance::names::to_upper("zażółć"), "ZAŻÓŁĆ");
        // A letter that would become two stays as it is, and the dotless i is
        // not capitalized at all.
        assert_eq!(crate::instance::names::to_upper("straße bıll"), "STRAßE BıLL");
    }

    #[test]
    fn a_user_name_is_letters_and_numbers_by_category() {
        for ok in ['a', 'ż', 'ａ', '名', '2', '²', 'Ⅷ', '٣', '_', '.', '-'] {
            assert!(is_username_char(ok), "{ok}");
        }
        for refused in ['ि', '्', '\u{0301}', '\u{200d}', '+', ' ', '😀'] {
            assert!(!is_username_char(refused), "{refused}");
        }
    }
}
