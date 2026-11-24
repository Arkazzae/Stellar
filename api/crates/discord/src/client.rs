//! Reads Discord servers through version 10 of Discord's REST API: a template
//! by its code without signing in, or everything a bot may see with its token.
//!
//! Every address is built here from ids and codes, on Discord's host, and the
//! requests follow no redirect and log nothing that came from them, so
//! neither a pasted link nor an answer can send the token anywhere else.

use std::fmt;
use std::sync::Arc;

use crate::bot::BotReader;
use crate::cdn::escape_segment;
use crate::config::ClientConfig;
use crate::failure::Result;
use crate::mapper;
use crate::rate_limits::Route;
use crate::snapshot::ServerSnapshot;
use crate::template_link::TemplateCode;
use crate::token::BotToken;
use crate::transport::Transport;
use crate::wire::WireTemplate;

/// The entry to Discord: cheap to clone, one per process is enough.
#[derive(Clone)]
pub struct DiscordClient {
    config: Arc<ClientConfig>,
}

impl Default for DiscordClient {
    fn default() -> DiscordClient {
        DiscordClient::new()
    }
}

impl fmt::Debug for DiscordClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscordClient")
            .field("config", &self.config)
            .finish()
    }
}

impl DiscordClient {
    /// The production client: Discord's own host.
    pub fn new() -> DiscordClient {
        DiscordClient::with_config(ClientConfig::default())
    }

    pub fn with_config(config: ClientConfig) -> DiscordClient {
        DiscordClient {
            config: Arc::new(config),
        }
    }

    /// A template's structure: its server's name, roles, sections and
    /// channels. No token is needed.
    pub async fn read_template(&self, code: &TemplateCode) -> Result<ServerSnapshot> {
        let transport = Transport::new(self.config.clone(), None);
        let template: WireTemplate = transport
            .get(Route::new(
                format!("guilds/templates/{}", escape_segment(code.as_str())),
                "guilds/templates/{code}",
                "",
            ))
            .await?;
        Ok(mapper::from_template(&template))
    }

    /// A reader for one bot. Keep it for the whole import: it remembers
    /// Discord's rate limits, which are counted per bot.
    pub fn for_bot(&self, token: BotToken) -> BotReader {
        BotReader::new(Arc::new(Transport::new(self.config.clone(), Some(token))))
    }
}
