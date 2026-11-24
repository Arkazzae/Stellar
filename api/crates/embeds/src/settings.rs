//! The settings link previews run on.

/// Settings of link previews. The defaults are what an instance runs with
/// when nothing is configured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbedSettings {
    /// Switches outbound fetching off. YouTube and Vimeo links still become
    /// players, because recognising them takes no request, but no page is
    /// read and no card is built.
    pub enabled: bool,

    /// How many links of one message get a card; the rest stay plain links.
    pub max_per_message: usize,

    /// How long a host may take for one complete request, including the body
    /// of a page, a preview picture or a YouTube oEmbed answer. A value of
    /// zero is read as one second.
    pub timeout_seconds: u64,

    /// How much of a page is read. The tags that make a card sit in the
    /// head, so a page that has not said it by then is not going to.
    pub max_html_bytes: usize,

    /// The largest preview picture kept. A bigger one is dropped, not cut.
    pub max_image_bytes: usize,

    /// How many redirects are followed. At least one is always allowed.
    pub max_redirects: u32,

    /// Lets the fetcher connect to loopback, link-local and private
    /// addresses.
    ///
    /// Off is the security boundary. A message body is written by whoever can
    /// post, so a fetcher that follows it anywhere lets that person make this
    /// server open connections to everything else it can reach. Turn it on
    /// only to preview pages on your own network, and only if everyone who
    /// can post may probe that network.
    pub allow_private_networks: bool,

    /// Sent as `User-Agent`, so that the other end can recognise and block us.
    pub user_agent: String,
}

impl Default for EmbedSettings {
    fn default() -> EmbedSettings {
        EmbedSettings {
            enabled: true,
            max_per_message: 2,
            timeout_seconds: 6,
            max_html_bytes: 512 * 1024,
            max_image_bytes: 2 * 1024 * 1024,
            max_redirects: 3,
            allow_private_networks: false,
            user_agent: "StellarrBot/1.0 (+https://github.com/arkazz/stellarr)".to_owned(),
        }
    }
}
