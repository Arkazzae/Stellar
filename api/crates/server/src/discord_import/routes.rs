use std::collections::HashSet;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use stellarr_discord::{BotReader, BotToken, ChannelKind, ServerSnapshot, VIEW_CHANNEL};

use super::jobs::{Jobs, Status, discord_error};
use super::plan::{self, History, Options};
use crate::auth::Caller;
use crate::http::{
    ApiError, ApiResult, Endpoints, Json, Path, Problem, RateLimitPolicy, json_response,
};
use crate::instance::starter::{self, ServerRequest};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

pub(crate) fn routes() -> Endpoints {
    Endpoints::new()
        .post("/api/servers/discord/servers", servers)
        .rate_limit(RateLimitPolicy::DiscordImport)
        .json_body()
        .authorized()
        .post("/api/servers/discord/preview", preview)
        .rate_limit(RateLimitPolicy::DiscordImport)
        .json_body()
        .authorized()
        .post("/api/servers/discord/imports", start)
        .rate_limit(RateLimitPolicy::DiscordImport)
        .json_body_up_to(16 * 1024)
        .authorized()
        .get("/api/servers/discord/imports/latest", latest)
        .authorized()
        .get("/api/servers/discord/imports/{id}", status)
        .authorized()
        .post("/api/servers/discord/imports/{id}/resume", resume)
        .rate_limit(RateLimitPolicy::DiscordImport)
        .json_body()
        .authorized()
        .delete("/api/servers/discord/imports/{id}", discard)
        .authorized()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Credentials {
    bot_token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewRequest {
    source_id: String,
    bot_token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartRequest {
    id: Id,
    bot_token: String,
    options: Options,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceServer {
    id: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BotServers {
    servers: Vec<SourceServer>,
    install_url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewChannel {
    id: String,
    name: String,
    kind: &'static str,
    section_id: Option<String>,
    can_copy_history: bool,
    has_own_messages: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewSection {
    id: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Preview {
    name: String,
    message_content: bool,
    sections: Vec<PreviewSection>,
    channels: Vec<PreviewChannel>,
    restricted_channels: usize,
}

fn problem(code: &'static str) -> ApiError {
    Problem::new(StatusCode::CONFLICT).with("code", code).into()
}

fn busy() -> ApiError {
    problem("discord-import-busy")
}

fn bot(state: &AppState, token: &str) -> ApiResult<BotReader> {
    if token.len() > 256 {
        return Err(problem("discord-invalid-token"));
    }
    let token = BotToken::new(token).map_err(|_| problem("discord-invalid-token"))?;
    Ok(state.discord_imports().discord.for_bot(token))
}

async fn servers(
    State(state): State<AppState>,
    _caller: Caller,
    Json(request): Json<Credentials>,
) -> ApiResult<Json<BotServers>> {
    let _permit = state
        .discord_imports()
        .previews
        .try_acquire()
        .map_err(|_| busy())?;
    let bot = bot(&state, &request.bot_token)?;
    let application = bot
        .current_application()
        .await
        .map_err(|err| problem(discord_error(err)))?;
    let app_id = application.id;
    if app_id.is_empty() || app_id.len() > 20 || !app_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(problem("discord-invalid-token"));
    }
    let sources = bot
        .list_servers()
        .await
        .map_err(|err| problem(discord_error(err)))?;
    let permissions = VIEW_CHANNEL | plan::READ_MESSAGE_HISTORY;
    Ok(Json(BotServers {
        servers: sources
            .into_iter()
            .map(|source| SourceServer {
                id: source.id,
                name: source.name,
            })
            .collect(),
        install_url: format!(
            "https://discord.com/oauth2/authorize?client_id={app_id}&scope=bot&permissions={permissions}&integration_type=0"
        ),
    }))
}

async fn preview(
    State(state): State<AppState>,
    _caller: Caller,
    Json(request): Json<PreviewRequest>,
) -> ApiResult<Json<Preview>> {
    let _permit = state
        .discord_imports()
        .previews
        .try_acquire()
        .map_err(|_| busy())?;
    let bot = bot(&state, &request.bot_token)?;
    let snapshot = plan::catalog(&bot, &request.source_id)
        .await
        .map_err(|err| problem(discord_error(err)))?;
    let application = bot
        .current_application()
        .await
        .map_err(|err| problem(discord_error(err)))?;
    let channels: Vec<_> = snapshot
        .channels
        .iter()
        .filter(|channel| plan::is_public(&snapshot, channel))
        .map(|channel| PreviewChannel {
            id: channel.id.clone(),
            name: channel.name.clone(),
            kind: match channel.kind {
                ChannelKind::Text => "text",
                ChannelKind::Voice => "voice",
            },
            section_id: channel.section_id.clone(),
            can_copy_history: plan::can_copy_history(&snapshot, channel),
            has_own_messages: plan::has_own_messages(channel),
        })
        .collect();
    let visible_sections: HashSet<_> = channels
        .iter()
        .filter_map(|channel| channel.section_id.as_deref())
        .collect();
    let sections = snapshot
        .sections
        .into_iter()
        .filter(|section| visible_sections.contains(section.id.as_str()))
        .map(|section| PreviewSection {
            id: section.id,
            name: section.name,
        })
        .collect();
    Ok(Json(Preview {
        name: snapshot.name,
        message_content: application.message_content_intent,
        restricted_channels: snapshot.channels.len() - channels.len(),
        sections,
        channels,
    }))
}

async fn selected(bot: &BotReader, options: &Options) -> ApiResult<ServerSnapshot> {
    if options.history.copies_messages()
        && !bot
            .current_application()
            .await
            .map_err(|err| problem(discord_error(err)))?
            .message_content_intent
    {
        return Err(problem("discord-message-content"));
    }
    let snapshot = plan::catalog(bot, &options.source_id)
        .await
        .map_err(|err| problem(discord_error(err)))?;
    plan::select(snapshot, options).map_err(problem)
}

async fn start(
    State(state): State<AppState>,
    caller: Caller,
    Json(request): Json<StartRequest>,
) -> ApiResult<Response> {
    let run_id = state.discord_imports().run_id();
    if let Some(job) = Jobs::find(&state, request.id, caller.user).await? {
        return Ok(json_response(StatusCode::OK, &job.status(run_id)));
    }
    let permit = state.discord_imports().reserve().ok_or_else(busy)?;
    // Another request could have created the job during the first read.
    if let Some(job) = Jobs::find(&state, request.id, caller.user).await? {
        return Ok(json_response(StatusCode::OK, &job.status(run_id)));
    }
    let mut options = request.options;
    let ids: HashSet<_> = options.channel_ids.iter().collect();
    if ids.is_empty()
        || ids.len() > 500
        || ids.len() != options.channel_ids.len()
        || matches!(options.history, History::Since { since } if since > Timestamp::now())
        || options.include_threads && !options.history.copies_messages()
    {
        return Err(problem("discord-invalid-selection"));
    }

    options.name = options.name.trim().to_owned();
    let bot = bot(&state, &request.bot_token)?;
    let snapshot = selected(&bot, &options).await?;
    let starter = starter::prepare(
        &ServerRequest {
            name: Some(options.name.clone()),
            description: snapshot
                .description
                .as_deref()
                .map(|description| super::names::clamp(description, 280)),
            voice_channel_name: Some(String::new()),
            ..ServerRequest::default()
        },
        state.messages(),
    )?;
    let job = Jobs::create(&state, request.id, caller.user, options, starter, &snapshot).await?;
    let status = job.status(run_id);
    Jobs::launch(state, permit, job, bot, snapshot);
    Ok(json_response(StatusCode::ACCEPTED, &status))
}

async fn status(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
) -> ApiResult<Json<Status>> {
    let id = crate::servers::guid(&id)?;
    let job = Jobs::find(&state, id, caller.user)
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(job.status(state.discord_imports().run_id())))
}

async fn latest(
    State(state): State<AppState>,
    caller: Caller,
) -> ApiResult<Json<Option<Status>>> {
    let found = Jobs::latest(&state, caller.user).await?;
    Ok(Json(found.map(|job| job.status(state.discord_imports().run_id()))))
}

async fn resume(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
    Json(request): Json<Credentials>,
) -> ApiResult<Response> {
    let id = crate::servers::guid(&id)?;
    let run_id = state.discord_imports().run_id();
    let permit = state.discord_imports().reserve().ok_or_else(busy)?;
    let job = Jobs::find(&state, id, caller.user)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let status = job.status(run_id);
    if status.state != "failed" && !(status.state == "completed" && job.has_pending_attachments()) {
        return Ok(json_response(StatusCode::OK, &status));
    }
    if job.published() && !job.still_owned(&state, caller.user).await? {
        return Err(ApiError::not_found());
    }
    let bot = bot(&state, &request.bot_token)?;
    let snapshot = selected(&bot, &job.options).await?;
    let job = Jobs::retry(&state, job).await?;
    let status = job.status(run_id);
    Jobs::launch(state, permit, job, bot, snapshot);
    Ok(json_response(StatusCode::ACCEPTED, &status))
}

async fn discard(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let id = crate::servers::guid(&id)?;
    let _permit = state.discord_imports().reserve().ok_or_else(busy)?;
    let job = Jobs::find(&state, id, caller.user)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if job.published() || job.status(state.discord_imports().run_id()).state != "failed" {
        return Err(problem("discord-import-not-failed"));
    }
    Jobs::discard(&state, job).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
