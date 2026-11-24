//! The bot token, which prints as nothing but its type.

use std::fmt;

/// A Discord bot token. It opens every server the bot is in, so no log line,
/// error or debug view may show it: it prints as its type, and the only way
/// to read it back is inside this crate, to put it in a request header.
#[derive(Clone)]
pub struct BotToken {
    value: String,
}

/// Why a text is not a token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidToken;

impl fmt::Display for InvalidToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("A Discord bot token is one word of printable characters.")
    }
}

impl std::error::Error for InvalidToken {}

impl BotToken {
    /// Takes the token as the Developer Portal shows it. A pasted header is
    /// forgiven: the `Bot ` prefix and the spaces around are dropped.
    ///
    /// # Errors
    /// Fails when nothing is left, or what is left has a space or a control
    /// character in it. The error does not repeat the text.
    pub fn new(pasted: &str) -> Result<BotToken, InvalidToken> {
        let mut value = pasted.trim();
        let prefix = value
            .get(..4)
            .filter(|head| head.eq_ignore_ascii_case("Bot "));
        if prefix.is_some() {
            value = value[4..].trim();
        } else if value.eq_ignore_ascii_case("Bot") {
            value = "";
        }

        if value.is_empty() || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(InvalidToken);
        }
        Ok(BotToken {
            value: value.to_owned(),
        })
    }

    pub(crate) fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Debug for BotToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BotToken")
    }
}

impl fmt::Display for BotToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BotToken")
    }
}
