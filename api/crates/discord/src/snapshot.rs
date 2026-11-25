//! What an import reads from Discord, named as Stellarr names it where it has
//! the thing. Ids are Discord's own, as text: snowflakes for a server the bot
//! read, the template's placeholder numbers ("0", "1", ...) for a template.
//! Nothing here is Stellarr's yet; turning it into Stellarr's records is the
//! import's job, and permission bits stay Discord's raw bit sets until then.

use crate::cdn::StickerFormat;
use crate::failure::FailureKind;
use crate::time::Timestamp;

/// Where a snapshot was read from, which says how much of it there can be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotSource {
    /// A template link: roles, channels and their permissions only.
    Template,
    /// A bot in the server: everything the bot may see.
    Bot,
}

/// One Discord server's structure and people. Messages are not part of it:
/// there can be millions, so they are read channel by channel as a stream.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerSnapshot {
    pub source: SnapshotSource,
    /// The server's id; for a template, that of the server it was made from.
    pub source_id: String,
    pub name: String,
    pub description: Option<String>,
    /// The icon on Discord's CDN, or `None` when the server has none.
    pub icon_url: Option<String>,
    /// Discord's categories.
    pub sections: Vec<SectionSnapshot>,
    pub channels: Vec<ChannelSnapshot>,
    pub roles: Vec<RoleSnapshot>,
    pub emoji: Vec<EmojiSnapshot>,
    pub stickers: Vec<StickerSnapshot>,
    pub members: Vec<MemberSnapshot>,
    pub threads: Vec<ThreadSnapshot>,
    /// What the bot could not read, and why. Everything else in the snapshot
    /// is complete; a part listed here is missing, not empty.
    pub gaps: Vec<SnapshotGap>,
}

/// A part of a server the reader was refused, such as one channel's private
/// threads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotGap {
    pub what: String,
    pub channel_id: Option<String>,
    pub kind: FailureKind,
    pub discord_code: Option<i32>,
}

/// A Discord category: a named group of channels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionSnapshot {
    pub id: String,
    pub name: String,
    pub position: i32,
    pub overwrites: Vec<PermissionOverwriteSnapshot>,
}

/// What Stellarr makes of a channel: it has text and voice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    Text,
    Voice,
}

/// What kind of Discord channel a channel was. Stellarr has text and voice;
/// this says which of Discord's flavours of them it came from, so the import
/// can say what it simplified.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiscordChannelType {
    /// An ordinary text or voice channel.
    Conversation,
    /// A text channel other servers can follow.
    Announcements,
    /// A text channel that holds only threads, as posts.
    Forum,
    /// A forum laid out for images and video.
    Media,
    /// A voice channel with speakers and an audience.
    Stage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelSnapshot {
    pub id: String,
    pub name: String,
    pub kind: ChannelKind,
    pub discord_type: DiscordChannelType,
    pub topic: Option<String>,
    pub slow_mode_seconds: i32,
    /// For a voice channel, how many may be in it at once; zero for no limit.
    pub user_limit: i32,
    /// Discord's position, which is not unique: its client sorts a section's
    /// text channels by position and then id, and its voice channels after
    /// them.
    pub position: i32,
    /// The section this channel sits under, or `None` when it has none.
    pub section_id: Option<String>,
    pub overwrites: Vec<PermissionOverwriteSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverwriteTarget {
    Role,
    Member,
}

/// One allow and deny pair on a channel or section, for one role or one
/// member. The two sets are Discord's permission bits as decimal text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionOverwriteSnapshot {
    pub target: OverwriteTarget,
    pub target_id: String,
    pub allow: String,
    pub deny: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleSnapshot {
    pub id: String,
    pub name: String,
    /// The colour as 0xRRGGBB, or `None` for a role without one.
    pub color: Option<i32>,
    /// Discord's position: higher is more senior, and @everyone is 0.
    pub position: i32,
    /// Whether members with it are listed apart.
    pub hoisted: bool,
    pub mentionable: bool,
    /// A role a bot or integration owns, such as a bot's own role or the
    /// boosters'.
    pub managed: bool,
    /// Discord's permission bits as decimal text.
    pub permissions: String,
    /// The role every member has. Discord gives it the server's id, and a
    /// template the placeholder 0.
    pub is_everyone: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmojiSnapshot {
    pub id: String,
    pub name: String,
    pub animated: bool,
    pub available: bool,
    pub image_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StickerSnapshot {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub format: StickerFormat,
    pub image_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberSnapshot {
    pub id: String,
    pub username: String,
    /// The name the person shows everywhere on Discord, or `None` when they
    /// kept their username.
    pub display_name: Option<String>,
    /// The name they use on this server only.
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    /// The avatar they set for this server only.
    pub server_avatar_url: Option<String>,
    /// Their roles, without @everyone, which every member has.
    pub role_ids: Vec<String>,
    pub is_bot: bool,
    pub joined_at: Option<Timestamp>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadSnapshot {
    pub id: String,
    pub parent_channel_id: String,
    pub name: String,
    /// The message the thread grew from, which has the thread's own id, or
    /// `None` for a private thread, which grows from none. Under a text or
    /// announcement channel it is in the parent channel, and may have been
    /// deleted since. For a forum or media post it is taken to be the post's
    /// opening message, inside the thread: Discord documents the shared id
    /// for threads started from a message, not for posts, so check before
    /// relying on it.
    pub starter_message_id: Option<String>,
    pub archived: bool,
    pub locked: bool,
    /// Visible only to those invited and to moderators.
    pub private: bool,
    /// When it was made; Discord only knows this for threads made after
    /// 2022-01-09.
    pub created_at: Option<Timestamp>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageSnapshot {
    pub id: String,
    /// The channel or thread the message is in.
    pub channel_id: String,
    pub author: AuthorSnapshot,
    /// As Discord stores it, tokens and all; [`crate::rewrite`] makes
    /// Stellarr text of it. Empty for every message but the bot's own when
    /// the app lacks the Message Content intent.
    pub content: String,
    pub mentions: Vec<MentionedUser>,
    pub created_at: Timestamp,
    pub edited_at: Option<Timestamp>,
    /// The message this one answers, if it is a reply.
    pub reply_to_message_id: Option<String>,
    pub pinned: bool,
    /// Discord's message type: 0 is ordinary, 19 a reply, most others notices.
    pub kind: i32,
    /// A notice Discord wrote, such as a member joining or a pin, rather than
    /// something somebody said.
    pub is_system_notice: bool,
    pub attachments: Vec<AttachmentSnapshot>,
    pub embeds: Vec<EmbedSummary>,
    pub stickers: Vec<StickerReference>,
    pub reactions: Vec<ReactionSnapshot>,
    /// The thread started from this message, which has the message's id.
    pub started_thread_id: Option<String>,
    /// The copy of another message this one forwards; its content is empty
    /// then.
    pub forwarded: Option<ForwardedSnapshot>,
}

/// Who a message shows as written by. For a webhook message, that is the name
/// and avatar the webhook gave this one message, and `id` is the webhook's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorSnapshot {
    pub id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub is_bot: bool,
    /// The webhook that posted the message, or `None` for a person or bot.
    pub webhook_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MentionedUser {
    pub id: String,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub nickname: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttachmentSnapshot {
    pub id: String,
    pub file_name: String,
    pub size: i64,
    pub content_type: Option<String>,
    /// A signed address that expires: download it while the import runs, or
    /// read the message again. Check it with [`crate::is_cdn_url`] first.
    pub url: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub duration_seconds: Option<f64>,
}

/// An embed reduced to what a link preview shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbedSummary {
    pub kind: Option<String>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub image_proxy_url: Option<String>,
    pub video_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StickerReference {
    pub id: String,
    pub name: String,
    pub format: StickerFormat,
    pub image_url: String,
}

/// One emoji's reactions on a message. Who reacted is a separate request per
/// emoji (and per kind, for super reactions), so only the counts come with
/// the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactionSnapshot {
    pub emoji: ReactionEmoji,
    pub count: i32,
    pub burst_count: i32,
}

/// A reaction's emoji: Unicode when `id` is `None`, a custom emoji otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactionEmoji {
    pub id: Option<String>,
    pub name: String,
    pub animated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ForwardedSnapshot {
    pub content: String,
    pub attachments: Vec<AttachmentSnapshot>,
}

/// A pinned message and when it was pinned, which the message itself does not
/// say.
#[derive(Clone, Debug, PartialEq)]
pub struct PinSnapshot {
    pub message: MessageSnapshot,
    pub pinned_at: Timestamp,
}

/// Somebody who reacted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactionUser {
    pub id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub is_bot: bool,
}

/// The bot a token belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BotIdentity {
    pub id: String,
    pub username: String,
    pub is_bot: bool,
}

/// The app behind a bot, and whether the two privileged intents the import
/// needs are on. Without Server Members the member list cannot be read;
/// without Message Content every message reads as empty, with no error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub id: String,
    pub name: String,
    pub server_members_intent: bool,
    pub message_content_intent: bool,
}

/// A server the bot is in, as the list of them gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerSummary {
    pub id: String,
    pub name: String,
    pub icon_url: Option<String>,
    pub approximate_member_count: Option<i32>,
}

/// One server's own fields, with Discord's estimate of how many people are in
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub owner_id: Option<String>,
    pub approximate_member_count: Option<i32>,
    pub approximate_presence_count: Option<i32>,
}

/// A server's channel list, split into Discord's categories and the channels
/// in them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelList {
    pub sections: Vec<SectionSnapshot>,
    pub channels: Vec<ChannelSnapshot>,
}
