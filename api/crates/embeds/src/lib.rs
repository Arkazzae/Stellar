//! Link previews: finding the links in a message and fetching what a card
//! for each needs.
//!
//! The top of the crate is [`Fetcher::preview`]: given a link and the
//! settings, it fetches the page, reads its title, description and picture,
//! downloads the picture, and returns what a card is made of, or the reason
//! there is none. Under it are the parts that need no network and are
//! tested without one: [`extract_links`], [`parse_video_link`],
//! [`parse_giphy_link`], [`parse_open_graph`], [`charset`] and
//! [`ImageKind::sniff`].
//!
//! The page being fetched is chosen by whoever posted the link, so
//! everything it sends is hostile: the address, the redirects, the size, the
//! encoding and the HTML. Each limit is a named constant or a setting with
//! its reason beside it.

pub mod charset;
mod content_type;
mod entity_table;
mod extract;
mod fetch;
mod giphy;
mod image;
mod link;
mod opengraph;
mod settings;
mod text;
mod video;
mod youtube;

pub use content_type::{ContentType, parse_content_type};
pub use extract::extract_links;
pub use fetch::{Failure, FetchedImage, Fetcher, NoCard, Preview, PreviewKind, YOUTUBE_OEMBED};
pub use giphy::parse_giphy_link;
pub use image::ImageKind;
pub use link::{Link, MAX_LINK_CHARS, Scheme};
pub use opengraph::{
    MAX_DESCRIPTION, MAX_TITLE, MAX_TITLE_ELEMENT, PageCard, gif_animation_url, parse_open_graph,
};
pub use settings::EmbedSettings;
pub use text::{clean_text, decode_entities};
pub use video::{MAX_START_SECONDS, VIMEO, VideoLink, YOUTUBE, parse_video_link};
pub use youtube::{MAX_OEMBED_BYTES, MAX_OEMBED_TITLE, MAX_THUMBNAIL_URL, parse_oembed};
