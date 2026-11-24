//! Fetching other people's pages on behalf of a message.
//!
//! A timeout, a 404, a login wall and a picture that is really a 40 MB TIFF
//! all end in "no card". A preview is decoration, and decoration must not be
//! able to fail a send or hold one up. The reason is kept all the same, for
//! the log, in [`Failure`].
//!
//! Every request goes through `stellarr-outbound`, which connects only to the
//! address it was allowed to resolve a name to, so a host that answers with a
//! public address the first time and a private one the second never gets a
//! connection, and every hop of a redirect is judged like the first.

use std::fmt;
use std::time::Duration;

use http::HeaderValue;
use http::header::{ACCEPT, ACCEPT_LANGUAGE, CONTENT_LENGTH, CONTENT_TYPE, USER_AGENT};
use stellarr_outbound::{AddressPolicy, Client, Error, Request, Response};

use crate::charset::decode_page;
use crate::content_type::parse_content_type;
use crate::giphy::parse_giphy_link;
use crate::image::ImageKind;
use crate::link::Link;
use crate::opengraph::{PageCard, parse_open_graph};
use crate::settings::EmbedSettings;
use crate::video::{VideoLink, YOUTUBE, parse_video_link};
use crate::youtube::{MAX_OEMBED_BYTES, oembed_address, parse_oembed};

/// What a page is asked for: a page, and failing that a picture.
const ACCEPT_PAGE: &str = "text/html,application/xhtml+xml,image/*;q=0.8";

/// Pages are asked for in Polish first, then English.
const ACCEPT_LANGUAGE_VALUE: &str = "pl,en;q=0.8";

/// What stands in for a user agent that cannot be sent as a header.
const FALLBACK_USER_AGENT: &str = "Stellarr";

/// What an oEmbed answer is asked for.
const ACCEPT_JSON: &str = "application/json";

/// Room above the cap for the head of an answer, which the client counts
/// toward the ceiling on its size together with the body: a body of exactly
/// the cap must still arrive, and one over it is caught by the cap itself.
const HEAD_ALLOWANCE: usize = 64 * 1024;

/// Where YouTube answers for a video.
pub const YOUTUBE_OEMBED: &str = "https://www.youtube.com/oembed";

/// The media types of a page that is read.
const PAGE_TYPES: [&str; 2] = ["text/html", "application/xhtml+xml"];

/// Why there is nothing to show for a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The address policy refused every address of the host, or a redirect.
    Refused,
    /// The host did not answer in time, the body included.
    Timeout,
    /// The answer announced more than the cap, or was over it.
    TooLarge,
    /// The answer was not 2xx: its status.
    Status(u16),
    /// More redirects than allowed, or one to somewhere refused.
    Redirects,
    /// Not a page: its type is not HTML.
    NotAPage,
    /// Not a picture of a kind that is kept: its type says so, or its bytes
    /// are not what its type says.
    NotAnImage,
    /// The answer had no body to keep.
    Empty,
    /// The address cannot be requested.
    BadAddress,
    /// Could not resolve, connect, speak HTTP, or the connection broke.
    Network,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Refused => f.write_str("the address is not allowed"),
            Failure::Timeout => f.write_str("the host took too long"),
            Failure::TooLarge => f.write_str("the answer is over the size cap"),
            Failure::Status(code) => write!(f, "the host answered {code}"),
            Failure::Redirects => f.write_str("the redirects did not lead to an answer"),
            Failure::NotAPage => f.write_str("the answer is not a page"),
            Failure::NotAnImage => f.write_str("the answer is not a picture that is kept"),
            Failure::Empty => f.write_str("the answer has no body"),
            Failure::BadAddress => f.write_str("the address cannot be requested"),
            Failure::Network => f.write_str("the connection failed"),
        }
    }
}

impl From<&Error> for Failure {
    fn from(error: &Error) -> Failure {
        match error {
            Error::AddressRefused { .. } => Failure::Refused,
            Error::Timeout | Error::ConnectTimeout { .. } => Failure::Timeout,
            Error::TooLarge => Failure::TooLarge,
            Error::TooManyRedirects | Error::RedirectRefused(_) => Failure::Redirects,
            Error::InvalidUrl(_) => Failure::BadAddress,
            _ => Failure::Network,
        }
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Failure {
        Failure::from(&error)
    }
}

/// A preview picture, downloaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedImage {
    pub bytes: Vec<u8>,
    pub kind: ImageKind,
}

impl FetchedImage {
    pub fn content_type(&self) -> &'static str {
        self.kind.content_type()
    }

    /// The name the stored copy goes by.
    pub fn file_name(&self) -> String {
        format!("preview{}", self.kind.extension())
    }

    pub fn size(&self) -> usize {
        self.bytes.len()
    }
}

/// The kind of card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewKind {
    Link,
    Video,
}

/// What a card is made of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub kind: PreviewKind,
    /// The address in its normal form, as the card links to it.
    pub url: String,
    /// The host without `www.`.
    pub domain: String,
    /// The site the card names: `youtube` or `vimeo` for a video, otherwise
    /// the domain.
    pub provider: String,
    /// The id of a video at its provider.
    pub provider_id: Option<String>,
    pub start_seconds: Option<u32>,
    pub title: Option<String>,
    pub description: Option<String>,
    /// The picture, already downloaded: the reader's browser is sent to this
    /// server for it, never to the linked host.
    pub thumbnail: Option<FetchedImage>,
}

/// Why a link gets no card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoCard {
    /// A GIPHY page: the viewer's own client resolves it, and it is neither
    /// fetched nor copied.
    Giphy,
    /// Previews are switched off.
    Disabled,
    /// The page could not be read.
    Unreadable(Failure),
    /// The page was read and says nothing a card can show, which is worse
    /// than the link the reader can already see.
    Empty,
}

impl fmt::Display for NoCard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NoCard::Giphy => f.write_str("a GIPHY link is left to the viewer's client"),
            NoCard::Disabled => f.write_str("link previews are switched off"),
            NoCard::Unreadable(failure) => write!(f, "the page could not be read: {failure}"),
            NoCard::Empty => f.write_str("the page says nothing about itself"),
        }
    }
}

/// Fetches pages and pictures under the limits of the settings.
#[derive(Clone)]
pub struct Fetcher {
    client: Client,
    settings: EmbedSettings,
    policy: AddressPolicy,
    oembed_endpoint: String,
}

impl Fetcher {
    /// A fetcher with a client of its own, trusting the system's
    /// certificates.
    pub fn new(settings: EmbedSettings) -> Fetcher {
        Fetcher::with_client(Client::new(), settings)
    }

    /// A fetcher on a client the caller made, to share one with the rest of
    /// the process or to give it a resolver of its own.
    pub fn with_client(client: Client, settings: EmbedSettings) -> Fetcher {
        let policy = if settings.allow_private_networks {
            AddressPolicy::Anywhere
        } else {
            AddressPolicy::PublicOnly
        };
        Fetcher {
            client,
            settings,
            policy,
            oembed_endpoint: YOUTUBE_OEMBED.to_owned(),
        }
    }

    /// Narrows or widens the addresses that may be connected to, whatever
    /// the settings say. The default is public addresses only unless the
    /// settings allow private networks.
    /// Override the address policy in tests.
    #[doc(hidden)]
    pub fn with_address_policy(mut self, policy: AddressPolicy) -> Fetcher {
        self.policy = policy;
        self
    }

    /// Asks another endpoint than YouTube's for oEmbed answers.
    /// Point oEmbed at a test server.
    #[doc(hidden)]
    pub fn with_oembed_endpoint(mut self, endpoint: impl Into<String>) -> Fetcher {
        self.oembed_endpoint = endpoint.into();
        self
    }

    pub fn settings(&self) -> &EmbedSettings {
        &self.settings
    }

    /// Everything a card needs for one link, or why it gets none.
    ///
    /// A video link is a card from the moment it is recognised: whether its
    /// host answers only decides how the card reads. A YouTube video is asked
    /// through its own endpoint and never read as an ordinary page.
    pub async fn preview(&self, link: &Link) -> Result<Preview, NoCard> {
        // GIPHY requires the viewer's own client to talk to it, so the link
        // stays in the message for the client to resolve.
        if parse_giphy_link(link).is_some() {
            return Err(NoCard::Giphy);
        }
        let video = parse_video_link(link);
        let card = match (&video, self.settings.enabled) {
            (_, false) => Err(NoCard::Disabled),
            (Some(video), true) if video.provider == YOUTUBE => {
                self.fetch_youtube(video).await.map_err(NoCard::Unreadable)
            }
            (_, true) => self.fetch_card(link).await.map_err(NoCard::Unreadable),
        };
        let card = match (card, &video) {
            (Ok(card), None) if card.is_empty() => return Err(NoCard::Empty),
            (Ok(card), _) => Some(card),
            (Err(reason), None) => return Err(reason),
            (Err(_), Some(_)) => None,
        };

        let thumbnail = match card.as_ref().and_then(|c| c.image_url.as_deref()) {
            Some(address) => match Link::parse(address) {
                Some(image) => self.fetch_image(&image).await.ok(),
                None => None,
            },
            None => None,
        };
        let card = card.unwrap_or_default();
        let domain = link.domain().to_owned();
        Ok(Preview {
            kind: if video.is_some() {
                PreviewKind::Video
            } else {
                PreviewKind::Link
            },
            url: link.to_string(),
            provider: video
                .as_ref()
                .map_or_else(|| domain.clone(), |v| v.provider.to_owned()),
            provider_id: video.as_ref().map(|v| v.id.clone()),
            start_seconds: video.as_ref().and_then(|v| v.start_seconds),
            domain,
            title: card.title,
            description: card.description,
            thumbnail,
        })
    }

    /// Reads what a page says about itself.
    pub async fn fetch_card(&self, link: &Link) -> Result<PageCard, Failure> {
        let cap = self.settings.max_html_bytes;
        let response = self
            .get(
                &link.fetch_address().ok_or(Failure::BadAddress)?,
                ACCEPT_PAGE,
                true,
                cap,
            )
            .await?;
        let Response {
            status,
            headers,
            url,
            mut body,
        } = response;
        if !status.is_success() {
            return Err(Failure::Status(status.as_u16()));
        }
        let content_type = headers
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_type)
            .filter(|t| PAGE_TYPES.contains(&t.media_type.as_str()))
            .ok_or(Failure::NotAPage)?;

        // A page that outgrows the cap while it streams is still read as far
        // as the cap: what a card needs is in the head, and the head comes
        // first. One that announces a size over the cap is not read at all.
        if announced_length(&headers).is_some_and(|length| length > cap as u64) {
            return Err(Failure::TooLarge);
        }
        let (bytes, _truncated) = body.read_up_to(cap).await?;

        let decoded = decode_page(&bytes, content_type.charset.as_deref());
        if let Some(name) = &decoded.unsupported {
            tracing::debug!(
                charset = name.as_str(),
                "a page declared a character set that is not decoded"
            );
        }
        // Relative addresses resolve against where the redirects ended, not
        // against the link as it was posted.
        let base = Link::parse(&format!(
            "{}://{}{}",
            if url.is_https() { "https" } else { "http" },
            url.host_header(),
            url.path_and_query()
        ))
        .unwrap_or_else(|| link.clone());
        Ok(parse_open_graph(&decoded.text, &base))
    }

    /// Downloads a preview picture, so that this server can serve it itself.
    pub async fn fetch_image(&self, link: &Link) -> Result<FetchedImage, Failure> {
        let cap = self.settings.max_image_bytes;
        let response = self
            .get(
                &link.fetch_address().ok_or(Failure::BadAddress)?,
                ACCEPT_PAGE,
                true,
                cap,
            )
            .await?;
        let Response {
            status,
            headers,
            mut body,
            ..
        } = response;
        if !status.is_success() {
            return Err(Failure::Status(status.as_u16()));
        }
        let declared = headers
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_type)
            .and_then(|t| ImageKind::from_media_type(&t.media_type))
            .ok_or(Failure::NotAnImage)?;
        if announced_length(&headers).is_some_and(|length| length > cap as u64) {
            return Err(Failure::TooLarge);
        }
        // Half a picture is not a picture, so one over the cap is dropped and
        // not kept cut short.
        let (bytes, truncated) = body.read_up_to(cap).await?;
        if truncated {
            return Err(Failure::TooLarge);
        }
        if bytes.is_empty() {
            return Err(Failure::Empty);
        }
        // The bytes also have to be what the header claims: this one comes
        // from a stranger and is about to be served from our origin.
        if ImageKind::sniff(&bytes) != Some(declared) {
            return Err(Failure::NotAnImage);
        }
        Ok(FetchedImage {
            bytes: bytes.to_vec(),
            kind: declared,
        })
    }

    /// Reads what YouTube itself says about a video whose id has already been
    /// validated.
    pub async fn fetch_youtube(&self, video: &VideoLink) -> Result<PageCard, Failure> {
        if video.provider != YOUTUBE {
            return Err(Failure::NotAPage);
        }
        let address = oembed_address(&self.oembed_endpoint, &video.id);
        let response = self
            .get(&address, ACCEPT_JSON, false, MAX_OEMBED_BYTES)
            .await?;
        let Response {
            status,
            headers,
            mut body,
            ..
        } = response;
        if !status.is_success() {
            return Err(Failure::Status(status.as_u16()));
        }
        let is_json = headers
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_type)
            .is_some_and(|t| t.media_type == "application/json");
        if !is_json {
            return Err(Failure::NotAPage);
        }
        if announced_length(&headers).is_some_and(|length| length > MAX_OEMBED_BYTES as u64) {
            return Err(Failure::TooLarge);
        }
        let (bytes, truncated) = body.read_up_to(MAX_OEMBED_BYTES).await?;
        if truncated {
            return Err(Failure::TooLarge);
        }
        parse_oembed(&bytes, &video.id).ok_or(Failure::NotAPage)
    }

    /// One request under the settings' limits. `cap` is how much of the body
    /// the caller will read.
    async fn get(
        &self,
        address: &str,
        accept: &str,
        language: bool,
        cap: usize,
    ) -> Result<Response, Failure> {
        let seconds = self.settings.timeout_seconds.max(1);
        let mut request = Request::get(address)?
            .header(USER_AGENT, self.user_agent())
            .header(
                ACCEPT,
                HeaderValue::from_str(accept).map_err(|_| Failure::BadAddress)?,
            )
            .timeout(Duration::from_secs(seconds))
            .connect_timeout(Duration::from_secs(seconds))
            .max_response_bytes((cap + HEAD_ALLOWANCE) as u64)
            .max_redirects(self.settings.max_redirects.max(1))
            .policy(self.policy);
        if language {
            request = request.header(
                ACCEPT_LANGUAGE,
                HeaderValue::from_static(ACCEPT_LANGUAGE_VALUE),
            );
        }
        match self.client.send(request).await {
            Ok(response) => Ok(response),
            Err(error) => {
                // The kind of failure only: a path or a query can carry a key.
                tracing::debug!(failure = %Failure::from(&error), "a preview request failed");
                Err(error.into())
            }
        }
    }

    fn user_agent(&self) -> HeaderValue {
        HeaderValue::from_str(&self.settings.user_agent)
            .unwrap_or_else(|_| HeaderValue::from_static(FALLBACK_USER_AGENT))
    }
}

fn announced_length(headers: &http::HeaderMap) -> Option<u64> {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
}
