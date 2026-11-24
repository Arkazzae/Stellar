//! Turns Discord's JSON shapes into the snapshot. A record without the id it
//! is known by is left out rather than guessed at; every other missing field
//! takes the value Discord documents as its default.

use std::collections::HashSet;

use crate::cdn::{self, StickerFormat};
use crate::snapshot::{
    AttachmentSnapshot, AuthorSnapshot, ChannelKind, ChannelSnapshot, DiscordChannelType,
    EmbedSummary, EmojiSnapshot, ForwardedSnapshot, MemberSnapshot, MessageSnapshot,
    OverwriteTarget, PermissionOverwriteSnapshot, ReactionEmoji, ReactionSnapshot, ReactionUser,
    RoleSnapshot, SectionSnapshot, ServerSnapshot, SnapshotSource, StickerReference,
    StickerSnapshot, ThreadSnapshot,
};
use crate::time::{Timestamp, parse_timestamp, snowflake_time};
use crate::wire::{
    List, WireAttachment, WireChannel, WireEmoji, WireGuild, WireMember, WireMessage,
    WireOverwrite, WireRole, WireSticker, WireTemplate, WireUser,
};

/// The id a template gives the role every member has.
pub(crate) const TEMPLATE_EVERYONE_ID: &str = "0";

const HAS_THREAD_FLAG: i64 = 1 << 5;
const REPLY_TYPE: i32 = 19;
const FORWARD_REFERENCE: i32 = 1;

/// Message types that are somebody speaking: ordinary, reply, slash command,
/// context menu command.
const SPOKEN_TYPES: [i32; 4] = [0, REPLY_TYPE, 20, 23];

pub(crate) fn from_template(template: &WireTemplate) -> ServerSnapshot {
    let empty = WireGuild::default();
    let guild = template
        .serialized_source_guild
        .0
        .as_ref()
        .unwrap_or(&empty);
    let source_id = template.source_guild_id.get().unwrap_or("");
    let (sections, channels) = structure(&guild.channels);
    ServerSnapshot {
        source: SnapshotSource::Template,
        source_id: source_id.to_owned(),
        name: guild
            .name
            .get()
            .or_else(|| template.name.get())
            .unwrap_or("")
            .to_owned(),
        description: guild.description.owned(),
        icon_url: if source_id.is_empty() {
            None
        } else {
            cdn::server_icon(
                source_id,
                guild.icon_hash.get().or_else(|| guild.icon.get()),
            )
        },
        sections,
        channels,
        roles: roles(&guild.roles, TEMPLATE_EVERYONE_ID),
        emoji: Vec::new(),
        stickers: Vec::new(),
        members: Vec::new(),
        threads: Vec::new(),
        gaps: Vec::new(),
    }
}

pub(crate) fn roles(roles: &List<WireRole>, everyone_id: &str) -> Vec<RoleSnapshot> {
    roles
        .0
        .iter()
        .enumerate()
        .filter_map(|(index, role)| {
            let role = role.as_ref()?;
            let id = role.id.get()?;
            let color = role
                .colors
                .0
                .as_ref()
                .and_then(|colors| colors.primary_color.0)
                .or(role.color.0)
                .filter(|&color| color > 0);
            Some(RoleSnapshot {
                id: id.to_owned(),
                name: role.name.get().unwrap_or("").to_owned(),
                color,
                // A template lists its roles in order and gives no positions.
                position: role
                    .position
                    .0
                    .unwrap_or_else(|| i32::try_from(index).unwrap_or(i32::MAX)),
                hoisted: role.hoist.0.unwrap_or(false),
                mentionable: role.mentionable.0.unwrap_or(false),
                managed: role.managed.0.unwrap_or(false),
                permissions: role.permissions.get().unwrap_or("0").to_owned(),
                is_everyone: id == everyone_id,
            })
        })
        .collect()
}

/// Splits a server's channel list into its categories and the channels
/// Stellarr can have.
///
/// Discord's directory channels (a hub's list of servers) and any type newer
/// than this reader are left out; threads never come in this list.
pub(crate) fn structure(
    channels: &List<WireChannel>,
) -> (Vec<SectionSnapshot>, Vec<ChannelSnapshot>) {
    let all: Vec<&WireChannel> = channels
        .present()
        .filter(|channel| channel.id.get().is_some())
        .collect();
    let sections: Vec<SectionSnapshot> = all
        .iter()
        .filter(|channel| channel.kind.0 == Some(4))
        .map(|channel| SectionSnapshot {
            id: channel.id.get().unwrap_or("").to_owned(),
            name: channel.name.get().unwrap_or("").to_owned(),
            position: channel.position.0.unwrap_or(0),
            overwrites: overwrites(&channel.permission_overwrites),
        })
        .collect();
    let section_ids: HashSet<&str> = sections.iter().map(|section| section.id.as_str()).collect();

    let mut mapped = Vec::new();
    for channel in all {
        let shape = match channel.kind.0 {
            Some(0) => (ChannelKind::Text, DiscordChannelType::Conversation),
            Some(2) => (ChannelKind::Voice, DiscordChannelType::Conversation),
            Some(5) => (ChannelKind::Text, DiscordChannelType::Announcements),
            Some(13) => (ChannelKind::Voice, DiscordChannelType::Stage),
            Some(15) => (ChannelKind::Text, DiscordChannelType::Forum),
            Some(16) => (ChannelKind::Text, DiscordChannelType::Media),
            _ => continue,
        };
        mapped.push(ChannelSnapshot {
            id: channel.id.get().unwrap_or("").to_owned(),
            name: channel.name.get().unwrap_or("").to_owned(),
            kind: shape.0,
            discord_type: shape.1,
            topic: channel.topic.owned(),
            slow_mode_seconds: channel.rate_limit_per_user.0.unwrap_or(0),
            user_limit: channel.user_limit.0.unwrap_or(0),
            position: channel.position.0.unwrap_or(0),
            section_id: channel
                .parent_id
                .get()
                .filter(|parent| section_ids.contains(parent))
                .map(str::to_owned),
            overwrites: overwrites(&channel.permission_overwrites),
        });
    }
    (sections, mapped)
}

fn overwrites(overwrites: &List<WireOverwrite>) -> Vec<PermissionOverwriteSnapshot> {
    overwrites
        .present()
        .filter_map(|overwrite| {
            let id = overwrite.id.get()?;
            let target = match overwrite.kind.0 {
                Some(0) => OverwriteTarget::Role,
                Some(1) => OverwriteTarget::Member,
                _ => return None,
            };
            Some(PermissionOverwriteSnapshot {
                target,
                target_id: id.to_owned(),
                allow: overwrite.allow.get().unwrap_or("0").to_owned(),
                deny: overwrite.deny.get().unwrap_or("0").to_owned(),
            })
        })
        .collect()
}

pub(crate) fn member(member: &WireMember, server_id: &str) -> Option<MemberSnapshot> {
    let user = member.user.0.as_ref()?;
    let id = user.id.get()?;
    Some(MemberSnapshot {
        id: id.to_owned(),
        username: user.username.get().unwrap_or("").to_owned(),
        display_name: user.global_name.owned(),
        nickname: member.nick.owned(),
        avatar_url: cdn::avatar(id, user.avatar.get()),
        server_avatar_url: cdn::member_avatar(server_id, id, member.avatar.get()),
        role_ids: member
            .roles
            .0
            .iter()
            .filter_map(|role| role.get())
            .filter(|&role| role != server_id)
            .map(str::to_owned)
            .collect(),
        is_bot: user.bot.0.unwrap_or(false),
        joined_at: time(member.joined_at.get()),
    })
}

pub(crate) fn emoji(emoji: &WireEmoji) -> Option<EmojiSnapshot> {
    let id = emoji.id.get()?;
    let animated = emoji.animated.0.unwrap_or(false);
    Some(EmojiSnapshot {
        id: id.to_owned(),
        name: emoji.name.get().unwrap_or("").to_owned(),
        animated,
        available: emoji.available.0.unwrap_or(true),
        image_url: cdn::emoji(id, animated),
    })
}

pub(crate) fn sticker(sticker: &WireSticker) -> Option<StickerSnapshot> {
    let id = sticker.id.get()?;
    let format = StickerFormat::from_number(sticker.format_type.0);
    Some(StickerSnapshot {
        id: id.to_owned(),
        name: sticker.name.get().unwrap_or("").to_owned(),
        description: sticker.description.owned(),
        format,
        image_url: cdn::sticker(id, format),
    })
}

pub(crate) fn thread(thread: &WireChannel) -> Option<ThreadSnapshot> {
    let id = thread.id.get()?;
    let parent = thread.parent_id.get()?;
    if !matches!(thread.kind.0, Some(10..=12)) {
        return None;
    }
    let private = thread.kind.0 == Some(12);
    let metadata = thread.thread_metadata.0.as_ref();
    Some(ThreadSnapshot {
        id: id.to_owned(),
        parent_channel_id: parent.to_owned(),
        name: thread.name.get().unwrap_or("").to_owned(),
        // Discord gives a public thread the id of the message it was started from.
        starter_message_id: if private { None } else { Some(id.to_owned()) },
        archived: metadata.and_then(|m| m.archived.0).unwrap_or(false),
        locked: metadata.and_then(|m| m.locked.0).unwrap_or(false),
        private,
        created_at: time(metadata.and_then(|m| m.create_timestamp.get())),
    })
}

pub(crate) fn message(message: &WireMessage) -> Option<MessageSnapshot> {
    let id = message.id.get()?;
    let channel_id = message.channel_id.get()?;
    let kind = message.kind.0.unwrap_or(0);
    let reference = message.message_reference.0.as_ref();
    let forwarded = reference
        .filter(|reference| reference.kind.0 == Some(FORWARD_REFERENCE))
        .and_then(|_| message.message_snapshots.0.first())
        .and_then(Option::as_ref)
        .and_then(|snapshot| snapshot.message.0.as_ref())
        .map(|copy| ForwardedSnapshot {
            content: copy.content.get().unwrap_or("").to_owned(),
            attachments: attachments(&copy.attachments),
        });

    Some(MessageSnapshot {
        id: id.to_owned(),
        channel_id: channel_id.to_owned(),
        author: author(message.author.0.as_ref(), message.webhook_id.get()),
        content: message.content.get().unwrap_or("").to_owned(),
        created_at: time(message.timestamp.get())
            .or_else(|| snowflake_time(id))
            .unwrap_or(Timestamp::UNIX_EPOCH),
        edited_at: time(message.edited_timestamp.get()),
        reply_to_message_id: if kind == REPLY_TYPE
            && reference.and_then(|r| r.kind.0).unwrap_or(0) == 0
        {
            reference.and_then(|r| r.message_id.owned())
        } else {
            None
        },
        pinned: message.pinned.0.unwrap_or(false),
        kind,
        is_system_notice: !SPOKEN_TYPES.contains(&kind),
        attachments: attachments(&message.attachments),
        embeds: message
            .embeds
            .present()
            .map(|embed| EmbedSummary {
                kind: embed.kind.owned(),
                url: embed.url.owned(),
                title: embed.title.owned(),
                description: embed.description.owned(),
                image_url: embed
                    .image
                    .0
                    .as_ref()
                    .and_then(|image| image.url.owned())
                    .or_else(|| {
                        embed
                            .thumbnail
                            .0
                            .as_ref()
                            .and_then(|thumb| thumb.url.owned())
                    }),
            })
            .collect(),
        stickers: message
            .sticker_items
            .present()
            .filter_map(|sticker| {
                let sticker_id = sticker.id.get()?;
                let format = StickerFormat::from_number(sticker.format_type.0);
                Some(StickerReference {
                    id: sticker_id.to_owned(),
                    name: sticker.name.get().unwrap_or("").to_owned(),
                    format,
                    image_url: cdn::sticker(sticker_id, format),
                })
            })
            .collect(),
        reactions: message
            .reactions
            .present()
            .filter_map(|reaction| {
                let emoji = reaction.emoji.0.as_ref()?;
                Some(ReactionSnapshot {
                    emoji: ReactionEmoji {
                        id: emoji.id.owned(),
                        name: emoji.name.get().unwrap_or("").to_owned(),
                        animated: emoji.animated.0.unwrap_or(false),
                    },
                    count: reaction.count.0.unwrap_or(0),
                    burst_count: reaction
                        .count_details
                        .0
                        .as_ref()
                        .and_then(|details| details.burst.0)
                        .unwrap_or(0),
                })
            })
            .collect(),
        started_thread_id: message
            .thread
            .0
            .as_ref()
            .and_then(|thread| thread.id.owned())
            .or_else(|| {
                (message.flags.0.unwrap_or(0) & HAS_THREAD_FLAG != 0).then(|| id.to_owned())
            }),
        forwarded,
    })
}

pub(crate) fn reaction_user(user: &WireUser) -> Option<ReactionUser> {
    Some(ReactionUser {
        id: user.id.get()?.to_owned(),
        username: user.username.get().unwrap_or("").to_owned(),
        display_name: user.global_name.owned(),
        is_bot: user.bot.0.unwrap_or(false),
    })
}

fn author(author: Option<&WireUser>, webhook_id: Option<&str>) -> AuthorSnapshot {
    let id = author
        .and_then(|a| a.id.get())
        .or(webhook_id)
        .unwrap_or("")
        .to_owned();
    AuthorSnapshot {
        username: author
            .and_then(|a| a.username.get())
            .unwrap_or("")
            .to_owned(),
        display_name: author.and_then(|a| a.global_name.owned()),
        avatar_url: if id.is_empty() {
            None
        } else {
            cdn::avatar(&id, author.and_then(|a| a.avatar.get()))
        },
        is_bot: author.and_then(|a| a.bot.0).unwrap_or(false),
        webhook_id: webhook_id.map(str::to_owned),
        id,
    }
}

fn attachments(attachments: &List<WireAttachment>) -> Vec<AttachmentSnapshot> {
    attachments
        .present()
        .filter_map(|attachment| {
            let id = attachment.id.get()?;
            let url = attachment.url.get()?;
            Some(AttachmentSnapshot {
                id: id.to_owned(),
                file_name: attachment.filename.get().unwrap_or(id).to_owned(),
                size: attachment.size.0.unwrap_or(0),
                content_type: attachment.content_type.owned(),
                url: url.to_owned(),
                width: attachment.width.0,
                height: attachment.height.0,
                duration_seconds: attachment.duration_secs.0,
            })
        })
        .collect()
}

pub(crate) fn time(text: Option<&str>) -> Option<Timestamp> {
    text.and_then(parse_timestamp)
}
