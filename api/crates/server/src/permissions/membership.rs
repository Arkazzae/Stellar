//! Which server a request is about, and who may administer the instance as a
//! whole.
//!
//! The client names a membership with a header or a `serverId` query value;
//! without either, the server the caller joined first is used. A handler asks
//! for it by taking [`ActiveServer`] as an argument, after `Caller`, on a route
//! that is `.authorized()`: a caller who is no member of the server named is
//! answered 403 with no body, and a name that is no id 400 with no body, before
//! the handler runs. [`InstanceOwner`] is the same for the instance's own
//! panel.
//!
//! The first server read the body of a request before it asked about
//! membership, so a person who is no member and sent a body that cannot be read
//! was told 400; an argument is read before the body is, so here they are told
//! 403. Nothing a client could make use of.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use rusqlite::{Connection, OptionalExtension, params};
use stellarr_messages::MessageKey;

use crate::auth::Caller;
use crate::auth::policy::{OPTIONAL, SecurityPolicy};
use crate::database::DbError;
use crate::http::{ApiError, Problem};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// Names the server a request is about; without it the caller's first server
/// is used.
pub const SERVER_HEADER: &str = "X-Stellarr-Server";

/// The problem code of an owner refused only for a missing second step, which
/// lets the operator panel send them to confirm it instead of saying they are
/// not the owner.
pub const COCKPIT_MFA_REQUIRED: &str = "cockpit-mfa-required";

/// What the person asked for, and what came of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// The server the request is about, and the person is a member of it.
    Active(Id),
    /// Not a member of the server named, or of any when none was.
    NotMember,
    /// The name is not an id.
    Malformed,
}

/// The server the request is about, as a handler takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveServer(pub Id);

/// The name a request gives: the header when there is one, even an empty one,
/// else the first `serverId` of the query (its name in any case, its value
/// decoded). The first value counts where there are several.
pub fn selection(headers: &HeaderMap, query: Option<&str>) -> Option<String> {
    if let Some(value) = headers.get(SERVER_HEADER) {
        return Some(String::from_utf8_lossy(value.as_bytes()).into_owned());
    }
    query?.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        decode(name)
            .eq_ignore_ascii_case("serverId")
            .then(|| decode(value))
    })
}

/// A query component with `+` as a space and `%XX` as its byte; anything that
/// is not a valid escape stays as it was.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|b| char::from(*b).to_digit(16));
        match bytes[index] {
            b'+' => out.push(b' '),
            b'%' => match (hex(index + 1), hex(index + 2)) {
                (Some(high), Some(low)) => {
                    out.push(u8::try_from(high * 16 + low).unwrap_or(b'%'));
                    index += 2;
                }
                _ => out.push(b'%'),
            },
            byte => out.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// An id in any of the spellings the first server accepted: with or without
/// hyphens, in braces or parentheses, in either case, with spaces around.
pub fn parse_guid(text: &str) -> Option<Id> {
    let text = text.trim_matches(char::is_whitespace);
    let (open, close) = match text.chars().next()? {
        '{' => ('{', '}'),
        '(' => ('(', ')'),
        _ => (' ', ' '),
    };
    let inner = if open == ' ' {
        text
    } else {
        text.strip_prefix(open)?.strip_suffix(close)?
    };
    if open == '{'
        && let Some(parsed) = parse_hex_form(inner)
    {
        return Some(parsed);
    }
    let hex = |s: &str| {
        u128::from_str_radix(s, 16)
            .ok()
            .filter(|_| s.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    let digits = match inner.len() {
        32 if open == ' ' => inner.to_string(),
        36 => {
            let parts: Vec<&str> = inner.split('-').collect();
            let sizes: Vec<usize> = parts.iter().map(|p| p.len()).collect();
            if sizes != [8, 4, 4, 4, 12] {
                return None;
            }
            parts.concat()
        }
        _ => return None,
    };
    hex(&digits).map(|value| Id::from_bytes(value.to_be_bytes()))
}

/// `0x`-prefixed groups: `{0xdddddddd,0xdddd,0xdddd,{0xdd,...}}`, each group
/// with one up to its full number of digits.
fn parse_hex_form(inner: &str) -> Option<Id> {
    let mut parts = inner.splitn(4, ',');
    let group = |part: Option<&str>, digits: usize| -> Option<u64> {
        let part = part?.strip_prefix("0x")?;
        (!part.is_empty() && part.len() <= digits && part.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| u64::from_str_radix(part, 16).ok())
            .flatten()
    };
    let a = group(parts.next(), 8)?;
    let b = group(parts.next(), 4)?;
    let c = group(parts.next(), 4)?;
    let rest = parts.next()?.strip_prefix('{')?.strip_suffix('}')?;
    let bytes: Vec<&str> = rest.split(',').collect();
    if bytes.len() != 8 {
        return None;
    }
    let mut out = [0u8; 16];
    out[..4].copy_from_slice(&u32::try_from(a).ok()?.to_be_bytes());
    out[4..6].copy_from_slice(&u16::try_from(b).ok()?.to_be_bytes());
    out[6..8].copy_from_slice(&u16::try_from(c).ok()?.to_be_bytes());
    for (index, byte) in bytes.iter().enumerate() {
        out[8 + index] = u8::try_from(group(Some(byte), 2)?).ok()?;
    }
    Some(Id::from_bytes(out))
}

/// The server the request is about. A name that is blank is none, which
/// means the server the person joined first.
pub fn choose(db: &Connection, user: Id, selection: Option<&str>) -> rusqlite::Result<Choice> {
    let selection = selection.map(str::trim).filter(|name| !name.is_empty());
    let found: Option<Id> = match selection {
        None => db
            .query_row(
                "SELECT \"ServerId\" FROM \"ServerMembers\" WHERE \"UserId\" = ?1 ORDER BY \"JoinedAt\", \"ServerId\" LIMIT 1",
                [user],
                |row| row.get(0),
            )
            .optional()?,
        Some(name) => {
            let Some(requested) = parse_guid(name) else {
                return Ok(Choice::Malformed);
            };
            db.query_row(
                "SELECT \"ServerId\" FROM \"ServerMembers\" WHERE \"UserId\" = ?1 AND \"ServerId\" = ?2",
                params![user, requested],
                |row| row.get(0),
            )
            .optional()?
        }
    };
    Ok(found.map_or(Choice::NotMember, Choice::Active))
}

impl FromRequestParts<AppState> for ActiveServer {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<ActiveServer, ApiError> {
        let caller = *parts
            .extensions
            .get::<Caller>()
            .ok_or_else(ApiError::unauthorized)?;
        let named = selection(&parts.headers, parts.uri.query());
        let choice = state
            .database()
            .read(move |db| -> Result<Choice, DbError> {
                Ok(choose(db, caller.user, named.as_deref())?)
            })
            .await?;
        match choice {
            Choice::Active(server) => Ok(ActiveServer(server)),
            Choice::NotMember => Err(ApiError::forbidden()),
            Choice::Malformed => Err(ApiError::bad_request()),
        }
    }
}

/// The first server ever created is the instance server.
pub fn instance_server(db: &Connection) -> rusqlite::Result<Option<Id>> {
    db.query_row(
        "SELECT \"Id\" FROM \"Servers\" ORDER BY \"CreatedAt\", \"Id\" LIMIT 1",
        [],
        |row| row.get(0),
    )
    .optional()
}

/// The instance server when the user owns it, and none otherwise. Owning a
/// server that somebody created later gives no say over the instance.
pub fn find_administered_server(db: &Connection, user: Id) -> rusqlite::Result<Option<Id>> {
    let Some(server) = instance_server(db)? else {
        return Ok(None);
    };
    let owns = db
        .query_row(
            "SELECT r.\"IsOwner\" FROM \"ServerMembers\" m JOIN \"ServerRoles\" r ON r.\"Id\" = m.\"RoleId\"
             WHERE m.\"ServerId\" = ?1 AND m.\"UserId\" = ?2",
            params![server, user],
            |row| row.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(false);
    Ok(owns.then_some(server))
}

/// Whether the person is an administrator of the instance: a holder of a role
/// with the manage-server permission on the instance server, whichever role
/// of the several they hold it is on, or its owner.
pub fn is_instance_admin(db: &Connection, user: Id) -> rusqlite::Result<bool> {
    let Some(server) = instance_server(db)? else {
        return Ok(false);
    };
    Ok(super::roles::permissions_of_member(db, server, user)?
        .is_some_and(|held| held & super::privileges::MANAGE_SERVER != 0))
}

/// What the instance's panel makes of a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceAccess {
    /// The owner of the instance server, who may pass.
    Owner(Id),
    NotOwner,
    /// The owner, stopped only because the account's policy asks a second step
    /// of this session and it has not passed one.
    SecondStepRequired,
}

/// Whether the session may use the instance's panel. Where a second step is
/// asked for at all, the session itself has to have completed one that has
/// not run out: a remembered device is not enough here.
pub fn instance_access(
    db: &Connection,
    user: Id,
    session: Id,
    now: Timestamp,
) -> rusqlite::Result<InstanceAccess> {
    let Some(server) = find_administered_server(db, user)? else {
        return Ok(InstanceAccess::NotOwner);
    };
    let policy = SecurityPolicy::load(db)?;
    if policy.mfa_requirement != OPTIONAL {
        let fresh: bool = db.query_row(
            "SELECT EXISTS (SELECT 1 FROM \"UserSessions\" WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"RevokedAt\" IS NULL
               AND \"MfaCompletedAt\" IS NOT NULL AND \"ExpiresAt\" > ?3)",
            params![session, user, now],
            |row| row.get(0),
        )?;
        if !fresh {
            return Ok(InstanceAccess::SecondStepRequired);
        }
    }
    Ok(InstanceAccess::Owner(server))
}

/// The instance server, for a handler of the instance's panel: the caller owns
/// it and, where the policy asks, has passed a second step. Anybody else is
/// refused 403, with the code `cockpit-mfa-required` for the owner who lacks
/// the second step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceOwner(pub Id);

impl FromRequestParts<AppState> for InstanceOwner {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<InstanceOwner, ApiError> {
        let caller = *parts
            .extensions
            .get::<Caller>()
            .ok_or_else(ApiError::unauthorized)?;
        let access = state
            .database()
            .read(move |db| -> Result<InstanceAccess, DbError> {
                Ok(instance_access(
                    db,
                    caller.user,
                    caller.session,
                    Timestamp::now(),
                )?)
            })
            .await?;
        match access {
            InstanceAccess::Owner(server) => Ok(InstanceOwner(server)),
            InstanceAccess::NotOwner => Err(ApiError::forbidden()),
            InstanceAccess::SecondStepRequired => {
                let title = state
                    .messages()
                    .get(MessageKey::CockpitSecondStepRequired, &[])?;
                Err(Problem::new(StatusCode::FORBIDDEN)
                    .title(title)
                    .with("code", COCKPIT_MFA_REQUIRED)
                    .into())
            }
        }
    }
}
