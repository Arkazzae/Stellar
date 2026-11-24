//! Reads a Discord server into a snapshot in Stellarr's own terms, from a
//! template link or with a bot token, and rewrites the text of its messages.
//!
//! This is the reading half of the Discord import: it talks to Discord,
//! checks what comes back, and says what it found. It touches no database.
//!
//! ```no_run
//! # async fn demo() -> stellarr_discord::Result<()> {
//! use stellarr_discord::{BotToken, DiscordClient};
//!
//! let client = DiscordClient::new();
//! let bot = client.for_bot(BotToken::new("the token").unwrap());
//! let snapshot = bot.read_server("41771983423143936").await?;
//!
//! // Messages are too many to hold: read a page, store it, ask for the next.
//! let mut pages = bot.messages("290926798999357250", None)?;
//! while let Some(page) = pages.next_page().await? {
//!     // ... write `page`; remember `pages.resume_before()` to pick up later
//!     # let _ = (&snapshot, page);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Everything Discord sends is input: lengths, counts and nesting are bounded,
//! ids that go into an address are checked or escaped, and an answer that does
//! not fit fails the read with a [`DiscordError`] and nothing else. The bot
//! token appears in no error, no debug print and no log line.

mod bot;
mod cdn;
mod client;
mod config;
mod content;
mod failure;
mod mapper;
mod mentions;
mod permissions;
mod rate_limits;
mod snapshot;
mod template_link;
mod time;
mod token;
mod transport;
mod unicode;
mod unicode_tables;
mod wire;

pub use bot::{
    ArchivedThreadPages, BotReader, MemberPages, MessagePages, PinPages, ReactionUserPages,
};
pub use cdn::{
    CDN_HOST, MEDIA_HOST, StickerFormat, avatar, emoji as emoji_url, is_cdn_url, member_avatar,
    server_icon, sticker as sticker_url,
};
pub use client::DiscordClient;
pub use config::{API_BASE, ClientConfig, RequestPolicy};
pub use content::{MentionText, ReferenceResolver, escape, rewrite};
pub use failure::{DiscordError, FailureKind, Result};
pub use mentions::extract_normalized_usernames;
pub use permissions::{VIEW_CHANNEL, compare_ids, hidden_from_everyone, permission_bits};
pub use snapshot::*;
pub use template_link::{TemplateCode, TemplateLinkRefusal, parse_template_link};
pub use time::{Clock, SystemClock, Timestamp, parse_timestamp, snowflake_time};
pub use token::{BotToken, InvalidToken};
