//! Only the declared icon and cover are exposed on Stellarr's own origin.
//! Other release files always belong to its isolated activity host.
use axum::{body::Body, extract::State, http::header, response::Response};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use stellarr_embeds::ImageKind;

use super::{Installed, Manifest, invalid};
use crate::{
    auth::Caller,
    encoding::base64,
    http::{ApiError, ApiResult, Path},
    permissions::membership::InstanceOwner,
    state::AppState,
    values::Id,
};

fn declared<'a>(manifest: &'a Manifest, kind: &str) -> ApiResult<Option<&'a str>> {
    match kind {
        "icon" => Ok(manifest.icon.as_deref()),
        "cover" => Ok(manifest.cover.as_deref()),
        _ => Err(ApiError::not_found()),
    }
}
fn kind_of(path: &str, kind: &str, bytes: &[u8]) -> ApiResult<ImageKind> {
    let limit = if kind == "icon" {
        512 * 1024
    } else {
        2 * 1024 * 1024
    };
    let extension = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let media = match extension.as_str() {
        "jpg" | "jpeg" => ImageKind::Jpeg,
        "png" => ImageKind::Png,
        "webp" => ImageKind::Webp,
        _ => {
            return Err(invalid(format!(
                "Use a PNG, JPEG or WebP file for the {kind}."
            )));
        }
    };
    if path.len() > 512
        || path.contains(['\\', ':', '?', '#', '%'])
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|s| s.is_empty() || s.starts_with('.') || s == "_stellarr" || s == "node_modules")
    {
        return Err(invalid(format!(
            "The {kind} must be a relative file in the ZIP."
        )));
    }
    if bytes.len() > limit || ImageKind::sniff(bytes) != Some(media) {
        return Err(invalid(format!(
            "The {kind} must match its image extension and fit within {} KiB.",
            limit / 1024
        )));
    }
    Ok(media)
}
pub(super) fn preview(manifest: &Manifest, files: &BTreeMap<String, Vec<u8>>) -> ApiResult<Value> {
    let mut images = json!({"icon":null,"cover":null});
    for kind in ["icon", "cover"] {
        let Some(path) = declared(manifest, kind)? else {
            if kind == "icon" {
                return Err(invalid(
                    "Add an icon path to stellarr.json and include the PNG, JPEG or WebP file in the ZIP.",
                ));
            }
            continue;
        };
        let bytes = files
            .get(path)
            .ok_or_else(|| invalid(format!("The declared {kind} is missing from the ZIP.")))?;
        let media = kind_of(path, kind, bytes)?;
        images[kind] = json!(format!(
            "data:{};base64,{}",
            media.content_type(),
            base64(bytes)
        ));
    }
    Ok(images)
}
pub(super) fn urls(entry: &Installed, base: &str) -> Value {
    let url = |kind| {
        declared(&entry.manifest, kind)
            .ok()
            .flatten()
            .map(|_| format!("{base}/{}/{kind}", entry.digest))
    };
    json!({"icon":url("icon"),"cover":url("cover")})
}
async fn image(state: &AppState, id: Id, digest: String, kind: String) -> ApiResult<Response> {
    let (media, bytes) = state
        .database()
        .read(move |db| -> ApiResult<_> {
            let text: String = db
                .query_row(
                    "SELECT Manifest FROM ActivityVersions WHERE Install=?1 AND Digest=?2",
                    params![id, digest],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or_else(ApiError::not_found)?;
            let manifest: Manifest = serde_json::from_str(&text).map_err(ApiError::internal)?;
            let path = declared(&manifest, &kind)?.ok_or_else(ApiError::not_found)?;
            let bytes: Vec<u8> = db
                .query_row(
                    "SELECT Content FROM ActivityFiles WHERE Install=?1 AND Digest=?2 AND Path=?3",
                    params![id, digest, path],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or_else(ApiError::not_found)?;
            Ok((kind_of(path, &kind, &bytes)?, bytes))
        })
        .await?;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, media.content_type())
        .header(header::CACHE_CONTROL, "private, no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; sandbox; frame-ancestors 'none'",
        )
        .body(Body::from(bytes))
        .unwrap())
}
pub(super) async fn owner(
    State(state): State<AppState>,
    _: InstanceOwner,
    Path((id, digest, kind)): Path<(Id, String, String)>,
) -> ApiResult<Response> {
    image(&state, id, digest, kind).await
}
pub(super) async fn participant(
    State(state): State<AppState>,
    caller: Caller,
    Path((path, id, digest, kind)): Path<(String, Id, String, String)>,
) -> ApiResult<Response> {
    let _gate = state.activities().changes.read().await;
    let channel = super::live::Scope::from_path(&path, caller.user)?;
    // The icon of what runs in a voice channel is for everybody who sees the
    // channel, whose list shows it; the rest is for the people inside.
    if super::live::check(&state, channel, caller.user).is_err()
        && !(kind == "icon"
            && matches!(channel, super::live::Scope::Voice(_))
            && state.activities().pinned_digest(id, channel).is_some()
            && super::live::sees(&state, channel, caller.user).await?)
    {
        return Err(ApiError::forbidden());
    }
    let current = super::library(&state)
        .await?
        .into_iter()
        .find(|entry| entry.install == id && entry.enabled)
        .ok_or_else(ApiError::not_found)?;
    if state.activities().pinned_digest(id, channel).is_none()
        && !current.manifest.launch_modes.contains(&channel.mode())
    {
        return Err(ApiError::forbidden());
    }
    if state
        .activities()
        .pinned_digest(id, channel)
        .unwrap_or(current.digest)
        != digest
    {
        return Err(ApiError::not_found());
    }
    image(&state, id, digest, kind).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verifies_declared_raster_bytes_and_falls_back_for_optional_cover() {
        let mut manifest: Manifest = serde_json::from_value(
            json!({"protocol":1,"id":"x","name":"X","description":"X","version":"1.0.0"}),
        )
        .unwrap();
        let files = BTreeMap::from([(
            "icon.png".to_owned(),
            vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a],
        )]);
        assert!(preview(&manifest, &files).is_err());
        manifest.icon = Some("icon.png".into());
        assert!(preview(&manifest, &files).unwrap()["cover"].is_null());
        for path in [
            "missing.png",
            "../icon.png",
            "https://outside.test/file.png",
            "clip.svg",
        ] {
            manifest.cover = Some(path.into());
            assert!(preview(&manifest, &files).is_err());
        }
        assert!(kind_of("thumbnail.png", "icon", b"<svg/>").is_err());
        assert!(kind_of("thumbnail.png", "icon", &vec![0; 512 * 1024 + 1]).is_err());
    }
}
