//! `POST` and `DELETE /api/me/images/{kind}`: the avatar and the banner of a
//! profile, and the wallpaper behind the person's own app.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::params;
use stellarr_messages::MessageKey;
use stellarr_unicode::to_lower;

use super::attachments::{Attachment, compact, retire_if_unreferenced};
use crate::http::created;
use super::inspect;
use super::transfer::{read_image_form, reserve, store_file, stored_or_rollback};
use super::refusals::{Texts, refuse};
use crate::accounts::me::Account;
use crate::auth::Caller;
use crate::database::DbError;
use crate::events::{Audience, Event};
use crate::http::{ApiError, ApiResult, Path, json_response};
use crate::instance::users::User;
use crate::state::AppState;
use crate::values::Id;

/// One of the account's pictures, and the limits that come with it.
struct Kind {
    name: &'static str,
    column: &'static str,
    max_bytes: i64,
    too_large_detail: MessageKey,
    /// Shown to others with the profile, so a change is announced to everybody
    /// who sees the person. The wallpaper is not: nobody else learns it changed.
    public: bool,
}

const AVATAR: Kind = Kind {
    name: "avatar",
    column: "AvatarUrl",
    max_bytes: 4 * 1024 * 1024,
    too_large_detail: MessageKey::AvatarMaximum,
    public: true,
};
const BANNER: Kind = Kind {
    name: "banner",
    column: "BannerUrl",
    max_bytes: 8 * 1024 * 1024,
    too_large_detail: MessageKey::BannerMaximum,
    public: true,
};
/// The client fits it to 2560 pixels on its longest side and re-encodes it
/// before upload, which comes well under this; the cap is the banner's.
const WALLPAPER: Kind = Kind {
    name: "wallpaper",
    column: "WallpaperUrl",
    max_bytes: 8 * 1024 * 1024,
    too_large_detail: MessageKey::WallpaperMaximum,
    public: false,
};

fn find_kind(name: &str) -> Option<&'static Kind> {
    match to_lower(name.trim()).as_str() {
        "avatar" => Some(&AVATAR),
        "banner" => Some(&BANNER),
        "wallpaper" => Some(&WALLPAPER),
        _ => None,
    }
}

pub async fn upload(
    State(state): State<AppState>,
    caller: Caller,
    Path(name): Path<String>,
    request: Request,
) -> ApiResult<Response> {
    let mut received = read_image_form(&state, request, BANNER.max_bytes, &[]).await?;
    let file = received.files.remove(0).ok_or_else(ApiError::bad_request)?;
    let Some(kind) = find_kind(&name) else {
        return Err(ApiError::not_found());
    };
    let spec = match inspect::image(&file, kind.max_bytes) {
        Ok(spec) => spec,
        Err(refused) => {
            let texts = Texts {
                unsupported_title: MessageKey::ProfileImageTypeUnsupported,
                unsupported_detail: MessageKey::ImageTypesAllowed,
                too_large_title: MessageKey::ProfileImageTooLarge,
                too_large_detail: &|_| kind.too_large_detail,
                mismatch_title: MessageKey::ProfileImageTypeMismatch,
            };
            return Err(refuse(&state, &refused, &texts)?);
        }
    };
    let user = caller.user;
    let exists = state
        .database()
        .read(move |db| -> Result<bool, DbError> { Ok(User::find(db, user)?.is_some()) })
        .await?;
    if !exists {
        return Err(ApiError::internal("the signed-in account is not there"));
    }
    let lease = reserve(&state, user, file.length as i64).await?;
    let id = Id::random();
    let key = format!(
        "profiles/{}/{}/{}{}",
        compact(user),
        kind.name,
        compact(id),
        spec.extension
    );
    store_file(&state, &key, &file).await?;
    let attachment = Attachment::plain_image(id, user, key.clone(), &file, spec.content_type);
    let url = format!("/assets/{id}");
    let column = kind.column;
    let new_url = url.clone();
    let written = state
        .database()
        .write(move |tx| -> Result<(Option<String>, Account), DbError> {
            attachment.insert(tx)?;
            let previous: Option<String> = tx.query_row(
                &format!("SELECT \"{column}\" FROM \"Users\" WHERE \"Id\" = ?1"),
                [user],
                |row| row.get(0),
            )?;
            tx.execute(
                &format!("UPDATE \"Users\" SET \"{column}\" = ?1 WHERE \"Id\" = ?2"),
                params![new_url, user],
            )?;
            let account = Account::load(tx, user)?
                .ok_or_else(|| DbError::Task("the signed-in account is not there".to_string()))?;
            Ok((previous, account))
        })
        .await;
    let (previous, account) = stored_or_rollback(&state, vec![key], written.map_err(ApiError::from)).await?;
    drop(lease);
    retire_if_unreferenced(&state, previous, Some(user)).await;
    if kind.public {
        broadcast_user_updated(&state, user).await?;
    }
    Ok(created(url, &account))
}

pub async fn remove(
    State(state): State<AppState>,
    caller: Caller,
    Path(name): Path<String>,
) -> ApiResult<Response> {
    let Some(kind) = find_kind(&name) else {
        return Err(ApiError::not_found());
    };
    let user = caller.user;
    let column = kind.column;
    let (previous, account) = state
        .database()
        .write(move |tx| -> Result<(Option<String>, Account), DbError> {
            let previous: Option<String> = tx.query_row(
                &format!("SELECT \"{column}\" FROM \"Users\" WHERE \"Id\" = ?1"),
                [user],
                |row| row.get(0),
            )?;
            tx.execute(
                &format!("UPDATE \"Users\" SET \"{column}\" = NULL WHERE \"Id\" = ?1"),
                [user],
            )?;
            let account = Account::load(tx, user)?
                .ok_or_else(|| DbError::Task("the signed-in account is not there".to_string()))?;
            Ok((previous, account))
        })
        .await?;
    retire_if_unreferenced(&state, previous, Some(user)).await;
    if kind.public {
        broadcast_user_updated(&state, user).await?;
    }
    Ok(json_response(StatusCode::OK, &account))
}

/// Tells everybody what the person looks like now, and the person what they
/// themselves look like (their own status shows as it is).
pub(crate) async fn broadcast_user_updated(state: &AppState, user: Id) -> Result<(), ApiError> {
    // Order the profile snapshot with activity opens that also read the account.
    let activity_change = state.activities().changes.write().await;
    let row = state
        .database()
        .read(move |db| -> Result<Option<User>, DbError> { Ok(User::find(db, user)?) })
        .await?;
    let Some(row) = row else { return Ok(()) };
    state.activities().update_profile(&row)?;
    drop(activity_change);
    let public = row.dto(&state.presence().public_presence_of(user, None));
    let own = row.dto(&state.presence_of(user));
    state
        .events()
        .publish(Event::new(Audience::Everyone, "UserUpdated", &public));
    state
        .events()
        .publish(Event::new(Audience::Users(vec![user]), "UserUpdated", &own));
    Ok(())
}
