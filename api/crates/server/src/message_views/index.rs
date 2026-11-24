//! Finding things in a channel: a text search over everything that is part of a
//! message, and the pages of its media and links.

use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use rusqlite::{Connection, params};
use serde::Serialize;
use stellarr_messages::MessageKey;

use super::cursor::{IndexCursor, Kind, query_hash};
use super::search::build_query;
use super::text;
use crate::messages::dto::{self, MessageDto};
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE, valid_page_size};
use crate::http::{ApiError, ApiResult, Json, Path, Problem, QueryValues};
use crate::permissions::membership::ActiveServer;
use crate::permissions::visibility::{self, ChannelKind};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const SEARCH_RESULT_COUNT: i64 = 50;
const MIN_QUERY_UNITS: usize = 2;
const MAX_QUERY_UNITS: usize = 200;

/// One page of a channel's index. A search is ordered by relevance, the media
/// and links by recency.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexPage {
    messages: Vec<MessageDto>,
    /// Opaque; none on the last page.
    next_cursor: Option<String>,
}

/// What the page's query string says. A `limit` that is no number is refused
/// where it is read, before anything else about the request is looked at.
pub struct PageQuery {
    limit: Option<i32>,
    kind: Option<String>,
    query: Option<String>,
    cursor: Option<String>,
}

impl FromRequestParts<AppState> for PageQuery {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<PageQuery, ApiError> {
        let values = QueryValues::parse(parts.uri.query(), state.messages());
        Ok(PageQuery {
            limit: values.int("limit")?,
            kind: values.text("kind")?,
            query: values.text("query")?,
            cursor: values.text("cursor")?,
        })
    }
}

/// The text of a search, which is the only thing its query string holds.
pub struct SearchQuery(Option<String>);

impl FromRequestParts<AppState> for SearchQuery {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<SearchQuery, ApiError> {
        Ok(SearchQuery(
            QueryValues::parse(parts.uri.query(), state.messages()).text("query")?,
        ))
    }
}

/// The best matches in the channel, one page and no more.
pub async fn search(
    Path(channel): Path<Id>,
    SearchQuery(query): SearchQuery,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<MessageDto>>> {
    let needle = text::trim(query.as_deref().unwrap_or("")).to_string();
    if !(MIN_QUERY_UNITS..=MAX_QUERY_UNITS).contains(&text::units(&needle)) {
        return Err(query_length(&state)?);
    }

    let messages = *state.messages();
    let user = caller.user;
    let found = state
        .database()
        .read(move |db| -> Result<Option<Vec<MessageDto>>, DbError> {
            // A channel the caller does not view is one that does not exist.
            if visibility::find(db, user, server, channel, Some(ChannelKind::Text))?.is_none() {
                return Ok(None);
            }
            let (ids, _) = read_page(
                db,
                channel,
                Kind::All,
                Some(&needle),
                Timestamp::now(),
                0,
                SEARCH_RESULT_COUNT,
            )?;
            Ok(Some(dto::load(db, &messages, &ids, Some(user))?))
        })
        .await?;
    found.map(Json).ok_or_else(ApiError::not_found)
}

fn query_length(state: &AppState) -> ApiResult<ApiError> {
    let reason = state.messages().get(
        MessageKey::SearchQueryLength,
        &[("min", &MIN_QUERY_UNITS), ("max", &MAX_QUERY_UNITS)],
    )?;
    Ok(Problem::invalid_field("query", reason))
}

/// Pages through the channel's messages, all of them, only those with
/// attachments, or only those with links, optionally narrowed by a text. The
/// first page fixes the moment the result is read as of, so that messages that
/// arrive while the reader pages cannot shift what is still to come.
pub async fn page(
    Path(channel): Path<Id>,
    asked: PageQuery,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    State(state): State<AppState>,
) -> ApiResult<Json<IndexPage>> {
    if let Some(limit) = asked.limit
        && !valid_page_size(limit)
    {
        let reason = state
            .messages()
            .get(MessageKey::PageLimitRange, &[("max", &MAX_PAGE_SIZE)])?;
        return Err(Problem::invalid_field("limit", reason));
    }

    let Some(kind) = parse_kind(asked.kind.as_deref()) else {
        let reason = state.messages().get(MessageKey::IndexKindInvalid, &[])?;
        return Err(Problem::invalid_field("kind", reason));
    };

    let needle = asked
        .query
        .as_deref()
        .filter(|query| !query.chars().all(text::is_white_space))
        .map(|query| text::trim(query).to_string());
    if let Some(needle) = &needle
        && !(MIN_QUERY_UNITS..=MAX_QUERY_UNITS).contains(&text::units(needle))
    {
        return Err(query_length(&state)?);
    }

    let messages = *state.messages();
    let user = caller.user;
    let limit = asked.limit.unwrap_or(DEFAULT_PAGE_SIZE);
    let hash = query_hash(needle.as_deref());
    let cursor = asked.cursor;
    let outcome = state
        .database()
        .read(move |db| -> Result<Option<Result<IndexPage, ()>>, DbError> {
            // A channel the caller does not view is one that does not exist.
            if visibility::find(db, user, server, channel, Some(ChannelKind::Text))?.is_none() {
                return Ok(None);
            }
            let start = match cursor.as_deref() {
                None => IndexCursor {
                    channel,
                    snapshot: Timestamp::now(),
                    offset: 0,
                    kind,
                    query_hash: hash,
                },
                Some(text) => match IndexCursor::decode(text) {
                    // A cursor says where in one result it stands, so it
                    // means nothing next to another channel, kind or text.
                    Some(found)
                        if found.channel == channel
                            && found.kind == kind
                            && found.query_hash == hash =>
                    {
                        found
                    }
                    _ => return Ok(Some(Err(()))),
                },
            };
            let (ids, more) = read_page(
                db,
                channel,
                kind,
                needle.as_deref(),
                start.snapshot,
                i64::from(start.offset),
                i64::from(limit),
            )?;
            let next_cursor = if more {
                let delivered = i32::try_from(ids.len()).unwrap_or(i32::MAX);
                Some(
                    IndexCursor {
                        offset: start.offset.saturating_add(delivered),
                        ..start
                    }
                    .encode(),
                )
            } else {
                None
            };
            Ok(Some(Ok(IndexPage {
                messages: dto::load(db, &messages, &ids, Some(user))?,
                next_cursor,
            })))
        })
        .await?;
    match outcome {
        None => Err(ApiError::not_found()),
        Some(Ok(page)) => Ok(Json(page)),
        Some(Err(())) => {
            let reason = state.messages().get(MessageKey::IndexCursorInvalid, &[])?;
            Err(Problem::invalid_field("cursor", reason))
        }
    }
}

fn parse_kind(name: Option<&str>) -> Option<Kind> {
    let name = stellarr_unicode::to_lower(text::trim(name.unwrap_or("")));
    match name.as_str() {
        "" | "all" => Some(Kind::All),
        "media" => Some(Kind::Media),
        "links" => Some(Kind::Links),
        _ => None,
    }
}

/// One page of the channel's messages as of `snapshot`, and whether the index
/// holds messages beyond it. Without a query they come newest first, so that
/// paging stays stable; with one, best match first. A query with no word in it
/// matches nothing.
fn read_page(
    db: &Connection,
    channel: Id,
    kind: Kind,
    query: Option<&str>,
    snapshot: Timestamp,
    offset: i64,
    limit: i64,
) -> rusqlite::Result<(Vec<Id>, bool)> {
    let matching = query.map(build_query);
    if matches!(matching, Some(None)) {
        return Ok((Vec::new(), false));
    }
    // One more than asked for, to learn whether there is a next page.
    let take = limit + 1;
    let mut ids = match matching.flatten() {
        None => list_newest(db, channel, kind, snapshot, offset, take)?,
        Some(expression) => matches(db, channel, kind, &expression, snapshot, offset, take)?,
    };
    let more = i64::try_from(ids.len()).unwrap_or(i64::MAX) > limit;
    if more {
        ids.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    }
    Ok((ids, more))
}

fn list_newest(
    db: &Connection,
    channel: Id,
    kind: Kind,
    snapshot: Timestamp,
    offset: i64,
    take: i64,
) -> rusqlite::Result<Vec<Id>> {
    // Compared with a value rather than tested bare: only an equality lets the
    // database read the index of the filter in order. A bare test would have
    // it sort every entry of the channel for each page.
    let filter = match kind {
        Kind::All => "",
        Kind::Media => "AND \"HasAttachments\" = ?5",
        Kind::Links => "AND \"HasLinks\" = ?5",
    };
    let sql = format!(
        "SELECT \"MessageId\" FROM \"MessageIndexEntries\"
         WHERE \"ChannelId\" = ?1 AND \"CreatedAt\" <= ?2 {filter}
         ORDER BY \"CreatedAt\" DESC, \"MessageId\" DESC
         LIMIT ?3 OFFSET ?4"
    );
    let mut statement = db.prepare_cached(&sql)?;
    if kind == Kind::All {
        statement
            .query_map(params![channel, snapshot, take, offset], |row| row.get(0))?
            .collect()
    } else {
        statement
            .query_map(params![channel, snapshot, take, offset, true], |row| {
                row.get(0)
            })?
            .collect()
    }
}

fn matches(
    db: &Connection,
    channel: Id,
    kind: Kind,
    expression: &str,
    snapshot: Timestamp,
    offset: i64,
    take: i64,
) -> rusqlite::Result<Vec<Id>> {
    let mut statement = db.prepare_cached(
        "SELECT i.\"MessageId\"
         FROM \"MessageSearch\"
         JOIN \"MessageIndexEntries\" i ON i.rowid = \"MessageSearch\".rowid
         WHERE \"MessageSearch\" MATCH ?1
           AND i.\"ChannelId\" = ?2
           AND i.\"CreatedAt\" <= ?3
           AND (NOT ?4 OR i.\"HasAttachments\" = 1)
           AND (NOT ?5 OR i.\"HasLinks\" = 1)
         ORDER BY bm25(\"MessageSearch\"), i.\"CreatedAt\" DESC, i.\"MessageId\" DESC
         LIMIT ?6 OFFSET ?7",
    )?;
    statement
        .query_map(
            params![
                expression,
                channel,
                snapshot,
                kind == Kind::Media,
                kind == Kind::Links,
                take,
                offset
            ],
            |row| row.get(0),
        )?
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_is_read_without_regard_to_case_or_padding() {
        assert_eq!(parse_kind(None), Some(Kind::All));
        assert_eq!(parse_kind(Some("")), Some(Kind::All));
        assert_eq!(parse_kind(Some("  ")), Some(Kind::All));
        assert_eq!(parse_kind(Some(" Media ")), Some(Kind::Media));
        assert_eq!(parse_kind(Some("LINKS")), Some(Kind::Links));
        assert_eq!(parse_kind(Some("all")), Some(Kind::All));
        assert_eq!(parse_kind(Some("videos")), None);
    }
}
