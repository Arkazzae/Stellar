//! Everything an import reads with a bot's token, one request at a time.
//!
//! Lists Discord pages are read to the end, each page asked for with the
//! largest size the documentation allows and the cursor it documents, and a
//! page that does not move the cursor on ends the read rather than repeating
//! it. Long lists come as pagers, so the import can store as it reads and
//! stop at any point: ask a pager for its next page, write it, ask again.
//! A pager keeps no connection, so it can be dropped at any time and picked
//! up later from the cursor it reports.
//!
//! Ids go into paths, so they must be snowflakes: digits only. Anything else
//! is refused before a request is made.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use crate::cdn::{self, escape_segment};
use crate::failure::{DiscordError, FailureKind, Result};
use crate::mapper;
use crate::permissions::compare_ids;
use crate::rate_limits::Route;
use crate::snapshot::{
    Application, BotIdentity, ChannelKind, ChannelList, DiscordChannelType, EmojiSnapshot,
    MemberSnapshot, MessageSnapshot, PinSnapshot, ReactionEmoji, ReactionUser, RoleSnapshot,
    ServerInfo, ServerSnapshot, ServerSummary, SnapshotGap, SnapshotSource, StickerSnapshot,
    ThreadSnapshot,
};
use crate::transport::Transport;
use crate::wire::{
    List, WireApplication, WireChannel, WireEmoji, WireGuild, WireMember, WireMessage, WirePinList,
    WireRole, WireSticker, WireThreadList, WireUser,
};

const GATEWAY_GUILD_MEMBERS: i64 = 1 << 14;
const GATEWAY_GUILD_MEMBERS_LIMITED: i64 = 1 << 15;
const GATEWAY_MESSAGE_CONTENT: i64 = 1 << 18;
const GATEWAY_MESSAGE_CONTENT_LIMITED: i64 = 1 << 19;

const SERVER_PAGE: usize = 200;
const MEMBER_PAGE: usize = 1000;
const MESSAGE_PAGE: usize = 100;
const PIN_PAGE: usize = 50;
const REACTION_PAGE: usize = 100;

/// A reader for one bot.
#[derive(Clone)]
pub struct BotReader {
    transport: Arc<Transport>,
}

/// Shows nothing of the token it holds.
impl fmt::Debug for BotReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BotReader")
    }
}

impl BotReader {
    pub(crate) fn new(transport: Arc<Transport>) -> BotReader {
        BotReader { transport }
    }

    async fn get<T>(&self, path: String, shape: &str, major: &str) -> Result<T>
    where
        T: for<'de> serde::Deserialize<'de>,
    {
        self.transport.get(Route::new(path, shape, major)).await
    }

    /// Who the token belongs to; the cheapest way to check that a token works.
    pub async fn current_user(&self) -> Result<BotIdentity> {
        let user: WireUser = self.get("users/@me".into(), "users/@me", "").await?;
        Ok(BotIdentity {
            id: user.id.get().unwrap_or("").to_owned(),
            username: user.username.get().unwrap_or("").to_owned(),
            is_bot: user.bot.0.unwrap_or(false),
        })
    }

    /// The bot's app and its privileged intents, read from the app's flags:
    /// Discord answers a read that lacks one with an empty field rather than
    /// an error, so this is the only place to learn of it beforehand.
    pub async fn current_application(&self) -> Result<Application> {
        let application: WireApplication = self
            .get("applications/@me".into(), "applications/@me", "")
            .await?;
        let flags = application.flags.0.unwrap_or(0);
        Ok(Application {
            id: application.id.get().unwrap_or("").to_owned(),
            name: application.name.get().unwrap_or("").to_owned(),
            server_members_intent: flags & (GATEWAY_GUILD_MEMBERS | GATEWAY_GUILD_MEMBERS_LIMITED)
                != 0,
            message_content_intent: flags
                & (GATEWAY_MESSAGE_CONTENT | GATEWAY_MESSAGE_CONTENT_LIMITED)
                != 0,
        })
    }

    /// The servers the bot is in, 200 to a page.
    pub async fn list_servers(&self) -> Result<Vec<ServerSummary>> {
        let mut servers = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let page: List<WireGuild> = self
                .get(
                    format!(
                        "users/@me/guilds?limit={SERVER_PAGE}&with_counts=true{}",
                        cursor_query("after", after.as_deref())
                    ),
                    "users/@me/guilds",
                    "",
                )
                .await?;
            let mut highest: Option<String> = None;
            for guild in page.0.iter().flatten() {
                let Some(id) = guild.id.get() else { continue };
                highest = Some(later(highest, id));
                servers.push(ServerSummary {
                    id: id.to_owned(),
                    name: guild.name.get().unwrap_or("").to_owned(),
                    icon_url: cdn::server_icon(id, guild.icon.get()),
                    approximate_member_count: guild.approximate_member_count.0,
                });
            }

            if !moved(
                page.0.len(),
                SERVER_PAGE,
                highest.as_deref(),
                after.as_deref(),
                true,
            ) {
                return Ok(servers);
            }
            after = highest;
        }
    }

    /// A server's own fields and Discord's estimate of its size.
    pub async fn server(&self, server_id: &str) -> Result<ServerInfo> {
        let id = snowflake(server_id, "the server id")?;
        let guild: WireGuild = self
            .get(
                format!("guilds/{id}?with_counts=true"),
                "guilds/{guild}",
                id,
            )
            .await?;
        Ok(ServerInfo {
            id: id.to_owned(),
            name: guild.name.get().unwrap_or("").to_owned(),
            description: guild.description.owned(),
            icon_url: cdn::server_icon(id, guild.icon.get()),
            owner_id: guild.owner_id.owned(),
            approximate_member_count: guild.approximate_member_count.0,
            approximate_presence_count: guild.approximate_presence_count.0,
        })
    }

    /// The server's categories and channels, without threads; only those the
    /// bot may see.
    pub async fn channels(&self, server_id: &str) -> Result<ChannelList> {
        let id = snowflake(server_id, "the server id")?;
        let channels: List<WireChannel> = self
            .get(
                format!("guilds/{id}/channels"),
                "guilds/{guild}/channels",
                id,
            )
            .await?;
        let (sections, channels) = mapper::structure(&channels);
        Ok(ChannelList { sections, channels })
    }

    pub async fn roles(&self, server_id: &str) -> Result<Vec<RoleSnapshot>> {
        let id = snowflake(server_id, "the server id")?;
        let roles: List<WireRole> = self
            .get(format!("guilds/{id}/roles"), "guilds/{guild}/roles", id)
            .await?;
        // The role every member has carries the server's own id.
        Ok(mapper::roles(&roles, id))
    }

    pub async fn emoji(&self, server_id: &str) -> Result<Vec<EmojiSnapshot>> {
        let id = snowflake(server_id, "the server id")?;
        let emoji: List<WireEmoji> = self
            .get(format!("guilds/{id}/emojis"), "guilds/{guild}/emojis", id)
            .await?;
        Ok(emoji.present().filter_map(mapper::emoji).collect())
    }

    pub async fn stickers(&self, server_id: &str) -> Result<Vec<StickerSnapshot>> {
        let id = snowflake(server_id, "the server id")?;
        let stickers: List<WireSticker> = self
            .get(
                format!("guilds/{id}/stickers"),
                "guilds/{guild}/stickers",
                id,
            )
            .await?;
        Ok(stickers.present().filter_map(mapper::sticker).collect())
    }

    /// Every member, a thousand to a page in order of id. It needs the Server
    /// Members intent. The documentation names no error for a read without
    /// it, so when Discord refuses the list the app's flags are read once:
    /// with the intent off, the refusal is [`FailureKind::MissingIntent`].
    pub fn members(&self, server_id: &str) -> Result<MemberPages> {
        Ok(MemberPages {
            reader: self.clone(),
            server: snowflake(server_id, "the server id")?.to_owned(),
            after: None,
            finished: false,
        })
    }

    /// The server's active threads, public and private, that the bot may see.
    pub async fn active_threads(&self, server_id: &str) -> Result<Vec<ThreadSnapshot>> {
        let id = snowflake(server_id, "the server id")?;
        let list: WireThreadList = self
            .get(
                format!("guilds/{id}/threads/active"),
                "guilds/{guild}/threads/active",
                id,
            )
            .await?;
        Ok(list.threads.present().filter_map(mapper::thread).collect())
    }

    /// One channel's archived threads, newest archived first. Public ones
    /// need Read Message History in the channel; private ones Manage Threads
    /// as well.
    pub fn archived_threads(
        &self,
        channel_id: &str,
        private_threads: bool,
    ) -> Result<ArchivedThreadPages> {
        Ok(ArchivedThreadPages {
            reader: self.clone(),
            channel: snowflake(channel_id, "the channel id")?.to_owned(),
            private: private_threads,
            before: None,
            finished: false,
        })
    }

    /// A channel's or thread's messages, newest first, a hundred to a page.
    /// Start from `before` to go on from where an earlier read stopped.
    /// Without Read Message History the channel reads as empty.
    pub fn messages(&self, channel_id: &str, before: Option<&str>) -> Result<MessagePages> {
        let channel = snowflake(channel_id, "the channel id")?.to_owned();
        let cursor = before
            .map(|id| snowflake(id, "the starting message id").map(str::to_owned))
            .transpose()?;
        Ok(MessagePages {
            reader: self.clone(),
            channel,
            cursor,
            finished: false,
        })
    }

    /// A channel's pinned messages with when each was pinned, latest pin
    /// first.
    pub fn pins(&self, channel_id: &str) -> Result<PinPages> {
        Ok(PinPages {
            reader: self.clone(),
            channel: snowflake(channel_id, "the channel id")?.to_owned(),
            before: None,
            finished: false,
        })
    }

    /// Who reacted to a message with one emoji, a hundred to a page. Super
    /// reactions are a list of their own: ask again with `burst` when
    /// [`crate::ReactionSnapshot::burst_count`] is above zero.
    pub fn reaction_users(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &ReactionEmoji,
        burst: bool,
    ) -> Result<ReactionUserPages> {
        let channel = snowflake(channel_id, "the channel id")?;
        let message = snowflake(message_id, "the message id")?;
        // A custom emoji is name:id; a deleted one may have lost its name,
        // and the id is what counts.
        let segment = match &emoji.id {
            None => escape_segment(&emoji.name),
            Some(id) => {
                let name = if emoji.name.is_empty() {
                    "_"
                } else {
                    &emoji.name
                };
                escape_segment(&format!("{name}:{}", snowflake(id, "the emoji id")?))
            }
        };
        Ok(ReactionUserPages {
            reader: self.clone(),
            path: format!("channels/{channel}/messages/{message}/reactions/{segment}"),
            major: channel.to_owned(),
            burst,
            after: None,
            finished: false,
        })
    }

    /// A server's whole structure and its people, in one snapshot: what a
    /// template has, plus emoji, stickers, members and threads.
    ///
    /// A part the bot is refused (the members without the intent, one
    /// channel's archived threads without the permission) is listed in
    /// [`ServerSnapshot::gaps`] and the rest is still read. Anything else
    /// that fails, a bad token or Discord out of reach, fails the read. It
    /// costs seven requests, one more per thousand members (and one to read
    /// the app's flags if the list is refused), and for each text channel one
    /// request per page of archived threads, public and, under an ordinary
    /// text channel, private.
    pub async fn read_server(&self, server_id: &str) -> Result<ServerSnapshot> {
        let info = self.server(server_id).await?;
        let structure = self.channels(&info.id).await?;
        let roles = self.roles(&info.id).await?;
        let emoji = self.emoji(&info.id).await?;
        let stickers = self.stickers(&info.id).await?;
        let mut gaps: Vec<SnapshotGap> = Vec::new();

        let mut members = Vec::new();
        refusable(&mut gaps, "members", None, async {
            let mut pages = self.members(&info.id)?;
            while let Some(page) = pages.next_page().await? {
                members.extend(page);
            }
            Ok(())
        })
        .await?;

        let mut threads: Vec<ThreadSnapshot> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        refusable(&mut gaps, "active threads", None, async {
            for thread in self.active_threads(&info.id).await? {
                if seen.insert(thread.id.clone()) {
                    threads.push(thread);
                }
            }
            Ok(())
        })
        .await?;

        for channel in structure
            .channels
            .iter()
            .filter(|c| c.kind == ChannelKind::Text)
        {
            // Private threads exist only under ordinary text channels.
            let lists: &[bool] = if channel.discord_type == DiscordChannelType::Conversation {
                &[false, true]
            } else {
                &[false]
            };
            for &private in lists {
                let what = if private {
                    "private archived threads"
                } else {
                    "archived threads"
                };
                refusable(&mut gaps, what, Some(&channel.id), async {
                    let mut pages = self.archived_threads(&channel.id, private)?;
                    while let Some(page) = pages.next_page().await? {
                        for thread in page {
                            if seen.insert(thread.id.clone()) {
                                threads.push(thread);
                            }
                        }
                    }
                    Ok(())
                })
                .await?;
            }
        }

        Ok(ServerSnapshot {
            source: SnapshotSource::Bot,
            source_id: info.id,
            name: info.name,
            description: info.description,
            icon_url: info.icon_url,
            sections: structure.sections,
            channels: structure.channels,
            roles,
            emoji,
            stickers,
            members,
            threads,
            gaps,
        })
    }
}

/// Runs a read whose refusal is a gap in the snapshot, not a failure of it.
async fn refusable(
    gaps: &mut Vec<SnapshotGap>,
    what: &str,
    channel_id: Option<&str>,
    read: impl std::future::Future<Output = Result<()>>,
) -> Result<()> {
    match read.await {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                FailureKind::MissingAccess | FailureKind::MissingIntent
            ) =>
        {
            gaps.push(SnapshotGap {
                what: what.to_owned(),
                channel_id: channel_id.map(str::to_owned),
                kind: error.kind(),
                discord_code: error.discord_code(),
            });
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// The server's members, a page at a time.
#[derive(Debug)]
pub struct MemberPages {
    reader: BotReader,
    server: String,
    after: Option<String>,
    finished: bool,
}

impl MemberPages {
    /// The next page, or `None` once the last one was given.
    pub async fn next_page(&mut self) -> Result<Option<Vec<MemberSnapshot>>> {
        if self.finished {
            return Ok(None);
        }
        let id = self.server.clone();
        let page: List<WireMember> = match self
            .reader
            .get(
                format!(
                    "guilds/{id}/members?limit={MEMBER_PAGE}{}",
                    cursor_query("after", self.after.as_deref())
                ),
                "guilds/{guild}/members",
                &id,
            )
            .await
        {
            Ok(page) => page,
            Err(refused) if refused.kind() == FailureKind::MissingAccess => {
                return Err(self.explain(refused).await);
            }
            Err(error) => return Err(error),
        };

        let mut highest: Option<String> = None;
        let mut members = Vec::new();
        for wire in page.0.iter().flatten() {
            if let Some(member) = mapper::member(wire, &id) {
                highest = Some(later(highest, &member.id));
                members.push(member);
            }
        }

        if moved(
            page.0.len(),
            MEMBER_PAGE,
            highest.as_deref(),
            self.after.as_deref(),
            true,
        ) {
            self.after = highest;
        } else {
            self.finished = true;
        }
        Ok(Some(members))
    }

    /// Turns a refused member list into a missing intent when the app's flags
    /// say the intent is off.
    async fn explain(&self, refused: DiscordError) -> DiscordError {
        let Ok(application) = self.reader.current_application().await else {
            return refused;
        };
        if application.server_members_intent {
            return refused;
        }
        let message = format!(
            "Discord refused {}, and the bot's app has the Server Members intent off",
            refused.route()
        );
        refused.explained(FailureKind::MissingIntent, message)
    }
}

/// A channel's archived threads, a page at a time.
#[derive(Debug)]
pub struct ArchivedThreadPages {
    reader: BotReader,
    channel: String,
    private: bool,
    before: Option<String>,
    finished: bool,
}

impl ArchivedThreadPages {
    pub async fn next_page(&mut self) -> Result<Option<Vec<ThreadSnapshot>>> {
        if self.finished {
            return Ok(None);
        }
        let kind = if self.private { "private" } else { "public" };
        let id = self.channel.clone();
        // The documentation names no range for limit, so Discord's own page
        // size is used.
        let page: WireThreadList = self
            .reader
            .get(
                format!(
                    "channels/{id}/threads/archived/{kind}{}",
                    cursor_query_first("before", self.before.as_deref())
                ),
                &format!("channels/{{channel}}/threads/archived/{kind}"),
                &id,
            )
            .await?;
        let mut oldest: Option<String> = None;
        let mut threads = Vec::new();
        for wire in page.threads.present() {
            if let Some(stamp) = wire
                .thread_metadata
                .0
                .as_ref()
                .and_then(|m| m.archive_timestamp.owned())
            {
                oldest = Some(stamp);
            }
            if let Some(thread) = mapper::thread(wire) {
                threads.push(thread);
            }
        }

        if page.has_more.0 != Some(true) || oldest.is_none() || oldest == self.before {
            self.finished = true;
        } else {
            self.before = oldest;
        }
        Ok(Some(threads))
    }
}

/// A channel's or thread's messages, newest first, a page at a time.
#[derive(Debug)]
pub struct MessagePages {
    reader: BotReader,
    channel: String,
    cursor: Option<String>,
    finished: bool,
}

impl MessagePages {
    /// The next page of up to a hundred messages, newest first, or `None`
    /// once the history is read to its start.
    pub async fn next_page(&mut self) -> Result<Option<Vec<MessageSnapshot>>> {
        if self.finished {
            return Ok(None);
        }
        let id = self.channel.clone();
        let page: List<WireMessage> = self
            .reader
            .get(
                format!(
                    "channels/{id}/messages?limit={MESSAGE_PAGE}{}",
                    cursor_query("before", self.cursor.as_deref())
                ),
                "channels/{channel}/messages",
                &id,
            )
            .await?;
        let mut oldest: Option<String> = None;
        let mut messages = Vec::new();
        for wire in page.0.iter().flatten() {
            if let Some(message) = mapper::message(wire) {
                oldest = Some(earlier(oldest, &message.id));
                messages.push(message);
            }
        }

        if moved(
            page.0.len(),
            MESSAGE_PAGE,
            oldest.as_deref(),
            self.cursor.as_deref(),
            false,
        ) {
            self.cursor = oldest;
        } else {
            self.finished = true;
        }
        Ok(Some(messages))
    }

    /// Where the read would go on from: the id of the oldest message read so
    /// far, to hand to [`BotReader::messages`] as `before` to resume after a
    /// stop. `None` before the first page.
    pub fn resume_before(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Whether the history has been read to its start.
    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

/// A channel's pinned messages, a page at a time.
#[derive(Debug)]
pub struct PinPages {
    reader: BotReader,
    channel: String,
    before: Option<String>,
    finished: bool,
}

impl PinPages {
    pub async fn next_page(&mut self) -> Result<Option<Vec<PinSnapshot>>> {
        if self.finished {
            return Ok(None);
        }
        let id = self.channel.clone();
        let page: WirePinList = self
            .reader
            .get(
                format!(
                    "channels/{id}/messages/pins?limit={PIN_PAGE}{}",
                    cursor_query("before", self.before.as_deref())
                ),
                "channels/{channel}/messages/pins",
                &id,
            )
            .await?;
        let mut oldest: Option<String> = None;
        let mut pins = Vec::new();
        for pin in page.items.present() {
            let (Some(wire), Some(pinned_at)) = (pin.message.0.as_ref(), pin.pinned_at.get())
            else {
                continue;
            };
            oldest = Some(pinned_at.to_owned());
            if let (Some(message), Some(pinned_at)) =
                (mapper::message(wire), mapper::time(Some(pinned_at)))
            {
                pins.push(PinSnapshot { message, pinned_at });
            }
        }

        if page.has_more.0 != Some(true) || oldest.is_none() || oldest == self.before {
            self.finished = true;
        } else {
            self.before = oldest;
        }
        Ok(Some(pins))
    }
}

/// The people who reacted with one emoji, a page at a time.
#[derive(Debug)]
pub struct ReactionUserPages {
    reader: BotReader,
    path: String,
    major: String,
    burst: bool,
    after: Option<String>,
    finished: bool,
}

impl ReactionUserPages {
    pub async fn next_page(&mut self) -> Result<Option<Vec<ReactionUser>>> {
        if self.finished {
            return Ok(None);
        }
        let page: List<WireUser> = self
            .reader
            .get(
                format!(
                    "{}?type={}&limit={REACTION_PAGE}{}",
                    self.path,
                    u8::from(self.burst),
                    cursor_query("after", self.after.as_deref())
                ),
                "channels/{channel}/messages/{message}/reactions/{emoji}",
                &self.major,
            )
            .await?;
        let mut highest: Option<String> = None;
        let mut users = Vec::new();
        for wire in page.0.iter().flatten() {
            if let Some(user) = mapper::reaction_user(wire) {
                highest = Some(later(highest, &user.id));
                users.push(user);
            }
        }

        if moved(
            page.0.len(),
            REACTION_PAGE,
            highest.as_deref(),
            self.after.as_deref(),
            true,
        ) {
            self.after = highest;
        } else {
            self.finished = true;
        }
        Ok(Some(users))
    }
}

macro_rules! collect_all {
    ($($pages:ty => $item:ty),* $(,)?) => {
        $(
            impl $pages {
                /// Every remaining item, page after page.
                pub async fn collect_all(&mut self) -> Result<Vec<$item>> {
                    let mut all = Vec::new();
                    while let Some(page) = self.next_page().await? {
                        all.extend(page);
                    }
                    Ok(all)
                }
            }
        )*
    };
}

collect_all!(
    MemberPages => MemberSnapshot,
    ArchivedThreadPages => ThreadSnapshot,
    MessagePages => MessageSnapshot,
    PinPages => PinSnapshot,
    ReactionUserPages => ReactionUser,
);

/// `&name=value` for a cursor, with the value escaped: it is text from
/// Discord, and text from Discord does not get to add to the query.
fn cursor_query(name: &str, value: Option<&str>) -> String {
    value
        .map(|v| format!("&{name}={}", escape_segment(v)))
        .unwrap_or_default()
}

/// `?name=value` for the first parameter of a query.
fn cursor_query_first(name: &str, value: Option<&str>) -> String {
    value
        .map(|v| format!("?{name}={}", escape_segment(v)))
        .unwrap_or_default()
}

/// Whether to ask for another page: the last one was full, and its cursor
/// moved past the one it was asked with, up for ids read upwards and down for
/// history read backwards.
fn moved(
    count: usize,
    page_size: usize,
    next: Option<&str>,
    previous: Option<&str>,
    forward: bool,
) -> bool {
    let Some(next) = next else { return false };
    count >= page_size
        && previous.is_none_or(|previous| {
            let order = compare_ids(next, previous);
            if forward {
                order == Ordering::Greater
            } else {
                order == Ordering::Less
            }
        })
}

fn later(current: Option<String>, id: &str) -> String {
    match current {
        Some(current) if compare_ids(id, &current) != Ordering::Greater => current,
        _ => id.to_owned(),
    }
}

fn earlier(current: Option<String>, id: &str) -> String {
    match current {
        Some(current) if compare_ids(id, &current) != Ordering::Less => current,
        _ => id.to_owned(),
    }
}

/// A snowflake: up to twenty digits.
fn snowflake<'a>(id: &'a str, what: &str) -> Result<&'a str> {
    if id.is_empty() || id.len() > 20 || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(DiscordError::invalid_argument(what));
    }
    Ok(id)
}
