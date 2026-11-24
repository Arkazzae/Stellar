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

use super::email;
use crate::auth::flow::{self, Context, Options, Start};
use crate::auth::policy::{ADMINS, EVERYONE, SecurityPolicy};
use crate::auth::{cookies, password, session};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Origin, Problem, json_response, with_cookie, with_session_cookie};
use crate::instance::access::{self, MANAGE_SERVER};
use crate::instance::starter::{self, ServerRequest};
use crate::instance::users::User;
use crate::instance::{invites, welcome};
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
    c.is_alphanumeric() || c == '_' || c == '.' || c == '-'
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
    if !email::is_plain_address(request.email.as_deref().unwrap_or("").trim()) {
        errors.push(("email".into(), vec![messages.get(MessageKey::EmailInvalid, &[])?]));
    }
    Ok(errors)
}

/// Capital letters the way the stored, unique forms of names and addresses
/// are written: letter by letter, a letter that would become several staying
/// as it is.
pub fn to_upper_invariant(text: &str) -> String {
    text.chars()
        .map(|c| {
            let mut upper = c.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(one), None) => one,
                _ => c,
            }
        })
        .collect()
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

/// Where the new account goes, as far as the checks can tell before anything
/// is written.
enum Plan {
    /// The first account: it makes the instance.
    Founding(starter::Starter),
    /// A later one, with the server its invite belongs to and that server's
    /// role for plain members.
    Joining { invite: Id, server: Id, role: Id, role_is_owner: bool, role_permissions: i64 },
}

/// Everything that can refuse a registration, read before anything is written.
fn check(
    connection: &Connection,
    state: &AppState,
    request: &RegisterRequest,
    username: &str,
    email: &str,
) -> Result<Plan, ApiError> {
    let messages = state.messages();
    // Empty means no account at all. An instance with accounts but no server,
    // which only an edit of the database could leave, is not empty: it takes
    // an invite like any other, and nobody can found it again.
    let founding: bool = connection.query_row("SELECT NOT EXISTS (SELECT 1 FROM \"Users\")", [], |row| row.get(0))?;
    if founding && !state.setup_code().matches(request.setup_code.as_deref()) {
        return Err(Problem::validation(vec![("setupCode".into(), vec![messages.get(MessageKey::SetupCodeInvalid, &[])?])]).into());
    }
    let taken = find_taken(connection, &to_upper_invariant(username), &to_upper_invariant(email), messages)?;
    if !taken.is_empty() {
        return Err(Problem::validation(taken).into());
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
    Ok(Plan::Joining { invite: invite.id, server: invite.server, role, role_is_owner, role_permissions })
}

/// What a registration comes to once it is committed.
struct Registered {
    user: User,
    start: Start,
    welcome: Option<welcome::Welcome>,
    founded: bool,
}

/// A hue for the person's avatar, taken from their id: the hash code that
/// .NET gives a GUID, which is the exclusive or of its four 32-bit words as
/// they lie in memory.
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
) -> Result<Registered, DbError> {
    let messages = state.messages();
    let now = Timestamp::now();
    let id = Id::random();
    tx.execute(
        "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"Email\", \"NormalizedEmail\",
             \"ProviderVerifiedEmail\", \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, 0, 0, 0, ?8, ?9)",
        params![
            id,
            input.username,
            to_upper_invariant(&input.username),
            input.display_name,
            input.password_hash,
            input.email,
            to_upper_invariant(&input.email),
            format!("oklch(0.58 0.14 {})", avatar_hue(id)),
            now
        ],
    )?;
    let user = User::find(tx, id)?.ok_or_else(|| DbError::Task("the account just written is not there".into()))?;
    // The first account owns the new instance server; anybody else joins it
    // with the base role.
    let (server, role_is_owner, role_permissions, founded, invite) = match plan {
        Plan::Founding(starter) => {
            let added = starter::add(tx, &starter, messages, now)?;
            starter::admit(tx, added.id, id, added.owner_role, true, false, now)?;
            (added.id, true, access::ALL, true, None)
        }
        Plan::Joining { invite, server, role, role_is_owner, role_permissions } => {
            starter::admit(tx, server, id, role, role_is_owner, !role_is_owner, now)?;
            (server, role_is_owner, role_permissions, false, Some(invite))
        }
    };
    let welcome = welcome::create(tx, server, &user, now, messages)?;
    if let Some(invite) = invite {
        invites::record_use(tx, invite)?;
    }

    // The first owner cannot skip an instance-wide requirement by
    // registering: the decision is made against the role just assigned.
    let policy = SecurityPolicy::load(tx)?;
    let required_override = match policy.mfa_requirement {
        EVERYONE => Some(true),
        ADMINS => Some(role_is_owner || role_permissions & MANAGE_SERVER != 0),
        _ => None,
    };
    let start = flow::start(tx, context, messages, &user, "registration", &policy, Options { required_override, ..Options::default() })?;
    if let Start::Session(session) = &start {
        session.insert(tx)?;
    }
    Ok(Registered { user, start, welcome, founded })
}

struct NewAccount {
    username: String,
    display_name: String,
    email: String,
    password_hash: Option<String>,
}

pub async fn register(State(state): State<AppState>, origin: Origin, Json(request): Json<RegisterRequest>) -> ApiResult<Response> {
    let messages = state.messages();
    let errors = validate(&request, messages)?;
    if !errors.is_empty() {
        return Err(Problem::validation(errors).into());
    }
    if request.external {
        // An identity a provider confirmed for this browser is read from a
        // cookie that only a sign-in through a provider sets, and this server
        // has none yet: there is nothing waiting.
        return Err(Problem::new(StatusCode::BAD_REQUEST).title(messages.get(MessageKey::ProviderSignUpExpired, &[])?).into());
    }
    let username = request.username.as_deref().unwrap_or("").trim().to_string();
    let email = request.email.as_deref().unwrap_or("").trim().to_string();

    // Whether the instance is still empty is read and acted on inside one
    // critical section, so two first registrations cannot both found it.
    let _turn = state.registration_gate().lock().await;
    let plan = {
        let (state, request, username, email) = (state.clone(), clone_request(&request), username.clone(), email.clone());
        let database = state.database().clone();
        database.read(move |connection| check(connection, &state, &request, &username, &email)).await?
    };
    let password = request.password.clone().unwrap_or_default();
    let password_hash = tokio::task::spawn_blocking(move || password::hash(&password))
        .await
        .map_err(ApiError::internal)?;
    let input = NewAccount {
        username: username.clone(),
        display_name: request.display_name.as_deref().unwrap_or("").trim().to_string(),
        email: email.clone(),
        password_hash: Some(password_hash),
    };

    let trusted_device_cookie = cookies::read(&origin.cookies, trusted_device_name(origin.https));
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
    let registered = match outcome {
        Ok(registered) => registered,
        // Somebody else wrote the same name or address after our check:
        // answer as the check would have, not with a server error.
        Err(error) if error.is_unique_violation() => {
            let taken = state
                .database()
                .read({
                    let (state, username, email) = (state.clone(), username.clone(), email.clone());
                    move |connection| find_taken(connection, &to_upper_invariant(&username), &to_upper_invariant(&email), state.messages())
                })
                .await?;
            return Err(if taken.is_empty() { error.into() } else { Problem::validation(taken).into() });
        }
        Err(error) => return Err(error.into()),
    };

    if registered.founded {
        // The instance has its owner. Later registrations do not ask for the
        // code, and none could pass with it anyway: it is gone.
        state.setup_code().forget();
    }
    if let Some(welcome) = &registered.welcome {
        let announced = state
            .database()
            .read({
                let welcome = welcome.clone();
                move |connection| welcome::announcement(connection, &welcome)
            })
            .await?;
        state.events().publish(announced);
    }
    created(&state, &origin, registered)
}

fn clone_request(request: &RegisterRequest) -> RegisterRequest {
    RegisterRequest {
        username: request.username.clone(),
        display_name: request.display_name.clone(),
        email: request.email.clone(),
        password: None,
        invite_code: request.invite_code.clone(),
        initial_server: request.initial_server.clone(),
        setup_code: request.setup_code.clone(),
        external: request.external,
    }
}

pub fn trusted_device_name(https: bool) -> &'static str {
    if https { "__Host-stellarr-trust" } else { "stellarr-dev-trust" }
}

fn created(state: &AppState, origin: &Origin, registered: Registered) -> ApiResult<Response> {
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
                false,
            );
            let mut response = json_response(StatusCode::CREATED, &registered.user.dto("online"));
            response.headers_mut().insert(header::LOCATION, location);
            Ok(with_session_cookie(response, &cookie))
        }
        Start::Challenge { body, cookie } => {
            let mut response = json_response(StatusCode::CREATED, &body);
            response.headers_mut().insert(header::LOCATION, location);
            Ok(with_cookie(response, &cookie))
        }
        // A new account is never in need of recovery.
        Start::RecoveryRequired => Ok(StatusCode::CONFLICT.into_response()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn names_are_capitalized_letter_by_letter() {
        assert_eq!(to_upper_invariant("Maya.Lin"), "MAYA.LIN");
        assert_eq!(to_upper_invariant("zażółć"), "ZAŻÓŁĆ");
        // One letter that would become two stays as it is.
        assert_eq!(to_upper_invariant("straße"), "STRAßE");
    }
}
