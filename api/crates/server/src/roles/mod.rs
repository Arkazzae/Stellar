//! The roles of a server and who holds them, and the overrides of what a role
//! or a member may do in one channel or section.
//!
//! A member holds any number of roles besides the base role, which everybody
//! holds. Nobody can hand out, or take away, a power they do not have
//! themselves: every route here asks `permissions::authorization` for that and
//! turns its answer into the refusal the clients know.
//!
//! `GET /api/admin/roles`, `POST /api/admin/roles`,
//! `PATCH` and `DELETE /api/admin/roles/{roleId}`, `GET /api/admin/members`,
//! `PATCH /api/admin/members/{userId}/role`,
//! and under `/api/admin/channels/{id}/permissions` and
//! `/api/admin/sections/{id}/permissions` the overrides of roles and, below
//! `/members`, of members.

pub(crate) mod announce;
mod assignment;
mod catalog;

pub(crate) use catalog::{invalid_role, validate};
mod overrides;

use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use crate::http::{ApiError, Endpoints, Problem};
use crate::permissions::privileges;
use crate::permissions::roles::Role;
use crate::values::Id;
use stellarr_messages::{MessageKey, Messages, MissingValue};

/// The server a request is about, asked for like [`ActiveServer`] but refused
/// only when the handler asks for it. A route that reads a body takes this
/// one, so that a body that cannot be read is answered 400 before a person who
/// is no member is answered 403, as the first server did.
pub struct Chosen(crate::permissions::membership::Choice);

impl Chosen {
    /// The server, or the refusal of a person who is no member of it.
    pub fn server(&self) -> Result<Id, ApiError> {
        use crate::permissions::membership::Choice;
        match self.0 {
            Choice::Active(server) => Ok(server),
            Choice::NotMember => Err(ApiError::forbidden()),
            Choice::Malformed => Err(ApiError::bad_request()),
        }
    }
}

impl axum::extract::FromRequestParts<crate::state::AppState> for Chosen {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &crate::state::AppState,
    ) -> Result<Chosen, ApiError> {
        let caller = *parts
            .extensions
            .get::<crate::auth::Caller>()
            .ok_or_else(ApiError::unauthorized)?;
        let named = crate::permissions::membership::selection(&parts.headers, parts.uri.query());
        let choice = state
            .database()
            .read(move |db| -> Result<_, crate::database::DbError> {
                Ok(crate::permissions::membership::choose(db, caller.user, named.as_deref())?)
            })
            .await?;
        Ok(Chosen(choice))
    }
}

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/admin/roles", catalog::list)
        .authorized()
        .post("/api/admin/roles", catalog::create)
        .json_body()
        .authorized()
        .patch("/api/admin/roles/{roleId}", catalog::update)
        .json_body()
        .authorized()
        .delete("/api/admin/roles/{roleId}", catalog::delete)
        .authorized()
        .get("/api/admin/members", assignment::list)
        .authorized()
        .patch("/api/admin/members/{userId}/role", assignment::assign)
        .json_body()
        .authorized()
        .merge(overrides::routes())
}

/// A role with the two columns the rules do not read.
#[derive(Debug, Clone)]
pub(crate) struct RoleRow {
    pub(crate) role: Role,
    pub(crate) name: String,
    pub(crate) color: String,
}

const ROLE_COLUMNS: &str =
    "\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\"";

impl RoleRow {
    pub(crate) fn read(row: &Row<'_>) -> rusqlite::Result<RoleRow> {
        Ok(RoleRow {
            role: Role {
                id: row.get(0)?,
                server: row.get(1)?,
                permissions: row.get(4)?,
                position: row.get(5)?,
                is_system: row.get(6)?,
                is_owner: row.get(7)?,
            },
            name: row.get(2)?,
            color: row.get(3)?,
        })
    }

    /// The role of the server with this id.
    pub(crate) fn find(db: &Connection, server: Id, id: Id) -> rusqlite::Result<Option<RoleRow>> {
        db.query_row(
            &format!("SELECT {ROLE_COLUMNS} FROM \"ServerRoles\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2"),
            params![id, server],
            RoleRow::read,
        )
        .optional()
    }

    /// Every role of the server, by position as the database gives them.
    fn all(db: &Connection, server: Id) -> rusqlite::Result<Vec<RoleRow>> {
        let mut statement = db.prepare(&format!(
            "SELECT {ROLE_COLUMNS} FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 ORDER BY \"Position\""
        ))?;
        statement.query_map([server], RoleRow::read)?.collect()
    }

    pub(crate) fn dto(&self, member_count: usize) -> RoleDto {
        RoleDto {
            id: self.role.id,
            name: self.name.clone(),
            color: self.color.clone(),
            permissions: privileges::to_names(self.role.permissions),
            member_count,
            is_system: self.role.is_system,
            is_owner: self.role.is_owner,
        }
    }
}

/// A role of the server; `member_count` counts who holds it, and for the base
/// role every member.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoleDto {
    pub(crate) id: Id,
    name: String,
    color: String,
    permissions: Vec<&'static str>,
    member_count: usize,
    is_system: bool,
    is_owner: bool,
}

/// A change of roles on the instance's own server needs a second step passed
/// in this session lately, for an account that has one; other servers ask for
/// nothing. Nothing is written when it is refused.
async fn require_fresh(
    state: &crate::state::AppState,
    caller: crate::auth::Caller,
    server: Id,
) -> Result<(), ApiError> {
    let fresh = state
        .database()
        .read(move |db| -> Result<bool, ApiError> {
            Ok(crate::auth::fresh::has_fresh_second_step_for_server(
                db,
                caller.user,
                Some(caller.session),
                server,
                crate::values::Timestamp::now(),
            )?)
        })
        .await?;
    if fresh {
        Ok(())
    } else {
        Err(crate::auth::fresh::refusal(state.messages())?.into())
    }
}

/// The answer that says the server has no role for plain members.
fn no_member_role(messages: &Messages) -> Result<ApiError, MissingValue> {
    Ok(Problem::new(StatusCode::SERVICE_UNAVAILABLE)
        .title(messages.get(MessageKey::ServerHasNoMemberRole, &[])?)
        .into())
}

/// A field that is refused, in the language of the instance.
fn invalid(field: &str, text: String) -> ApiError {
    Problem::validation(vec![(field.to_string(), vec![text])]).into()
}

/// An id that a request leaves out, which the first server read as the empty
/// one.
fn nil() -> Id {
    Id::from_bytes([0; 16])
}

/// Whether a failed statement is a unique value that was taken.
fn taken(error: rusqlite::Error) -> ApiError {
    let error = crate::database::DbError::from(error);
    if error.is_unique_violation() {
        ApiError::conflict()
    } else {
        ApiError::from(error)
    }
}

/// The whole of a response that carries nothing.
fn no_content() -> Response {
    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    response
}
