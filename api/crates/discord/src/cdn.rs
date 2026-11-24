//! Addresses of images on Discord's CDN, and the check every download from it
//! must pass.
//!
//! An import downloads what a Discord server points at: avatars, icons, emoji,
//! stickers and attachments. Those addresses arrive inside API answers, and an
//! attachment's address in particular is a signed URL Discord wrote, so the
//! downloader takes nothing on trust: only https on the two hosts the
//! documentation names is fetched at all.

use stellarr_outbound::Url;

/// The image CDN, also the host of signed attachment URLs.
pub const CDN_HOST: &str = "cdn.discordapp.com";

/// The media proxy: attachments' `proxy_url`, and the only home of GIF
/// stickers.
pub const MEDIA_HOST: &str = "media.discordapp.net";

const CDN_BASE: &str = "https://cdn.discordapp.com/";

/// The file format a sticker is stored in; Discord's `format_type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StickerFormat {
    Png = 1,
    Apng = 2,
    Lottie = 3,
    Gif = 4,
}

impl StickerFormat {
    /// The format for Discord's number; anything unknown reads as PNG.
    pub fn from_number(number: Option<i32>) -> StickerFormat {
        match number {
            Some(2) => StickerFormat::Apng,
            Some(3) => StickerFormat::Lottie,
            Some(4) => StickerFormat::Gif,
            _ => StickerFormat::Png,
        }
    }
}

/// Whether `url` may be downloaded: absolute https, exactly one of the two
/// CDN hosts, no login part and no port but the default.
///
/// The address is read by the same parser the download goes through, so what
/// this approves is what the download connects to: a host that only looks
/// like Discord's to some other reader of the text is not approved.
pub fn is_cdn_url(url: Option<&str>) -> bool {
    let Some(Ok(parsed)) = url.map(Url::parse) else {
        return false;
    };
    parsed.is_https()
        && parsed.port() == 443
        && (parsed.host() == CDN_HOST || parsed.host() == MEDIA_HOST)
}

/// A server's icon; `None` when it has none.
pub fn server_icon(server_id: &str, hash: Option<&str>) -> Option<String> {
    image(&format!("icons/{}", escape_segment(server_id)), hash)
}

/// A user's (or a webhook's) own avatar; `None` for Discord's default one.
pub fn avatar(user_id: &str, hash: Option<&str>) -> Option<String> {
    image(&format!("avatars/{}", escape_segment(user_id)), hash)
}

/// The avatar a member set for one server only.
pub fn member_avatar(server_id: &str, user_id: &str, hash: Option<&str>) -> Option<String> {
    image(
        &format!(
            "guilds/{}/users/{}/avatars",
            escape_segment(server_id),
            escape_segment(user_id)
        ),
        hash,
    )
}

/// A custom emoji, as WebP: Discord serves every emoji as WebP, while one
/// uploaded as WebP or AVIF does not convert cleanly to the other formats.
pub fn emoji(emoji_id: &str, animated: bool) -> String {
    format!(
        "{CDN_BASE}emojis/{}.webp{}",
        escape_segment(emoji_id),
        if animated { "?animated=true" } else { "" }
    )
}

/// A sticker in the one format Discord stores it in: PNG (also APNG), Lottie
/// JSON, or GIF, which lives on the media host instead.
pub fn sticker(sticker_id: &str, format: StickerFormat) -> String {
    match format {
        StickerFormat::Gif => format!(
            "https://{MEDIA_HOST}/stickers/{}.gif",
            escape_segment(sticker_id)
        ),
        StickerFormat::Lottie => format!("{CDN_BASE}stickers/{}.json", escape_segment(sticker_id)),
        _ => format!("{CDN_BASE}stickers/{}.png", escape_segment(sticker_id)),
    }
}

/// An image hash under `path`. A hash that starts with `a_` is animated and is
/// asked for as animated WebP, which Discord recommends and Stellarr accepts;
/// a still one as PNG.
fn image(path: &str, hash: Option<&str>) -> Option<String> {
    let hash = hash.filter(|hash| !hash.is_empty())?;
    Some(if hash.starts_with("a_") {
        format!(
            "{CDN_BASE}{path}/{}.webp?animated=true",
            escape_segment(hash)
        )
    } else {
        format!("{CDN_BASE}{path}/{}.png", escape_segment(hash))
    })
}

/// One path segment, with everything but unreserved characters escaped.
pub(crate) fn escape_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
