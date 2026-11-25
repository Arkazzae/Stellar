use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use stellarr_discord::{
    BotReader, ChannelKind, ChannelSnapshot, DiscordChannelType, DiscordError,
    PermissionOverwriteSnapshot, RoleSnapshot, ServerSnapshot, SnapshotSource, VIEW_CHANNEL,
    compare_ids,
};

use crate::values::Timestamp;

pub(super) const READ_MESSAGE_HISTORY: u64 = 1 << 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub(super) enum History {
    Structure,
    All,
    Since { since: Timestamp },
}

impl History {
    pub(super) fn copies_messages(self) -> bool {
        self != Self::Structure
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Options {
    pub(super) source_id: String,
    pub(super) name: String,
    pub(super) channel_ids: Vec<String>,
    pub(super) history: History,
    pub(super) include_threads: bool,
}

/// Only include channels that are open to members as a whole. Declining any
/// explicit denial also covers individual overrides and malformed bit sets.
fn open_to_members(
    everyone: Option<&RoleSnapshot>,
    overwrites: &[PermissionOverwriteSnapshot],
    permission: u64,
) -> bool {
    let Some(everyone) = everyone else {
        return false;
    };
    let Ok(base) = everyone.permissions.parse::<u64>() else {
        return false;
    };
    let mut allowed = base & permission != 0;
    for overwrite in overwrites {
        let (Ok(allow), Ok(deny)) = (
            overwrite.allow.parse::<u64>(),
            overwrite.deny.parse::<u64>(),
        ) else {
            return false;
        };
        if deny & permission != 0 {
            return false;
        }
        if overwrite.target == stellarr_discord::OverwriteTarget::Role
            && overwrite.target_id == everyone.id
            && allow & permission != 0
        {
            allowed = true;
        }
    }
    allowed
}

pub(super) fn is_public(snapshot: &ServerSnapshot, channel: &ChannelSnapshot) -> bool {
    let everyone = snapshot
        .roles
        .iter()
        .find(|role| role.id == snapshot.source_id);
    open_to_members(everyone, &channel.overwrites, VIEW_CHANNEL)
}

pub(super) fn can_copy_history(snapshot: &ServerSnapshot, channel: &ChannelSnapshot) -> bool {
    let everyone = snapshot
        .roles
        .iter()
        .find(|role| role.id == snapshot.source_id);
    channel.kind == ChannelKind::Text
        && open_to_members(everyone, &channel.overwrites, READ_MESSAGE_HISTORY)
}

pub(super) fn has_own_messages(channel: &ChannelSnapshot) -> bool {
    channel.kind == ChannelKind::Text
        && !matches!(
            channel.discord_type,
            DiscordChannelType::Forum | DiscordChannelType::Media
        )
}

/// Preview and execution use the same catalog. Extra Discord resources and
/// private thread lists are outside the scope of the wizard.
pub(super) async fn catalog(
    bot: &BotReader,
    source_id: &str,
) -> Result<ServerSnapshot, DiscordError> {
    let info = bot.server(source_id).await?;
    let mut structure = bot.channels(&info.id).await?;
    let roles = bot.roles(&info.id).await?;
    structure.sections.sort_by(|a, b| {
        a.position
            .cmp(&b.position)
            .then_with(|| compare_ids(&a.id, &b.id))
    });
    structure.channels.sort_by(|a, b| {
        a.position
            .cmp(&b.position)
            .then_with(|| compare_ids(&a.id, &b.id))
    });
    Ok(ServerSnapshot {
        source: SnapshotSource::Bot,
        source_id: info.id,
        name: info.name,
        description: info.description,
        icon_url: info.icon_url,
        sections: structure.sections,
        channels: structure.channels,
        roles,
        emoji: Vec::new(),
        stickers: Vec::new(),
        members: Vec::new(),
        threads: Vec::new(),
        gaps: Vec::new(),
    })
}

pub(super) fn select(
    mut snapshot: ServerSnapshot,
    options: &Options,
) -> Result<ServerSnapshot, &'static str> {
    if snapshot.source_id != options.source_id {
        return Err("discord-selection-changed");
    }
    let ids: HashSet<&str> = options.channel_ids.iter().map(String::as_str).collect();
    let channels: Vec<_> = snapshot
        .channels
        .iter()
        .filter(|channel| ids.contains(channel.id.as_str()) && is_public(&snapshot, channel))
        .cloned()
        .collect();
    if channels.len() != ids.len() {
        return Err("discord-selection-changed");
    }
    if options.history.copies_messages() {
        let mut has_history = false;
        for channel in channels.iter().filter(|channel| {
            has_own_messages(channel)
                || (options.include_threads && channel.kind == ChannelKind::Text)
        }) {
            if !can_copy_history(&snapshot, channel) {
                return Err("discord-history-restricted");
            }
            has_history = true;
        }
        if !has_history {
            return Err("discord-no-history-channels");
        }
    }
    let sections: HashSet<_> = channels
        .iter()
        .filter_map(|ch| ch.section_id.as_deref())
        .collect();
    snapshot
        .sections
        .retain(|section| sections.contains(section.id.as_str()));
    snapshot.channels = channels;
    Ok(snapshot)
}

pub(super) async fn add_threads(
    bot: &BotReader,
    snapshot: &mut ServerSnapshot,
) -> Result<(), DiscordError> {
    let parents: HashSet<_> = snapshot
        .channels
        .iter()
        .filter(|channel| channel.kind == ChannelKind::Text)
        .map(|channel| channel.id.as_str())
        .collect();
    let mut seen = HashSet::new();
    let mut threads = bot.active_threads(&snapshot.source_id).await?;
    threads.retain(|thread| {
        !thread.private
            && parents.contains(thread.parent_channel_id.as_str())
            && seen.insert(thread.id.clone())
    });
    for parent in parents {
        let mut pages = bot.archived_threads(parent, false)?;
        while let Some(page) = pages.next_page().await? {
            threads.extend(page.into_iter().filter(|thread| {
                !thread.private
                    && thread.parent_channel_id == parent
                    && seen.insert(thread.id.clone())
            }));
        }
    }
    snapshot.threads = threads;
    Ok(())
}
