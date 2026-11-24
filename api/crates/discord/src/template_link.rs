//! Reads what a person pastes as a Discord template: a discord.new link, a
//! discord.com or discordapp.com /template/ link, with or without the scheme
//! and `www.`, or the bare code.
//!
//! Only the code is taken from the text. The request goes to an address the
//! client builds from the code, never to the text itself, so a link that
//! merely looks like Discord's cannot send the server anywhere. That is also
//! why a query, a fragment, a port or a login part is refused rather than
//! dropped: nothing Discord hands out has one, and a parser that tolerates
//! them has to decide what they mean.

use std::fmt;

/// The code of a Discord server template, checked and nothing else: the only
/// way to make one is [`parse_template_link`], so a value of this type is
/// always safe to put into the path of an API request.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TemplateCode(String);

impl TemplateCode {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TemplateCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a pasted text is not a template link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateLinkRefusal {
    Empty,
    /// An invite (discord.gg, discord.com/invite) joins a server; it does not
    /// describe one.
    InviteLink,
    /// A link to somewhere else, or with more in it than a template link has.
    NotATemplateLink,
    /// The right shape, but the code holds characters no template code has.
    InvalidCode,
}

/// Reads the code out of a pasted template link or code.
pub fn parse_template_link(input: &str) -> Result<TemplateCode, TemplateLinkRefusal> {
    let mut text = input.trim();
    if text.is_empty() {
        return Err(TemplateLinkRefusal::Empty);
    }

    let scheme = text.find("://");
    if let Some(at) = scheme {
        let name = &text[..at];
        if !name.eq_ignore_ascii_case("https") && !name.eq_ignore_ascii_case("http") {
            return Err(TemplateLinkRefusal::NotATemplateLink);
        }
        text = &text[at + 3..];
    }

    if text.contains(['?', '#', '@', '\\', ' ', '\t']) {
        return Err(TemplateLinkRefusal::NotATemplateLink);
    }

    let Some(slash) = text.find('/') else {
        // Without a path the whole text is the code, or a host on its own.
        if scheme.is_none() && is_code(text) {
            return Ok(TemplateCode(text.to_owned()));
        }
        return Err(if text.contains('.') || scheme.is_some() {
            TemplateLinkRefusal::NotATemplateLink
        } else {
            TemplateLinkRefusal::InvalidCode
        });
    };

    let mut host = text[..slash].to_ascii_lowercase();
    if let Some(rest) = host.strip_prefix("www.") {
        host = rest.to_owned();
    }
    let mut path = &text[slash + 1..];
    if let Some(rest) = path.strip_suffix('/') {
        path = rest;
    }

    let candidate = match host.as_str() {
        "discord.new" => Some(path),
        "discord.com" | "discordapp.com" if starts_with_ignore_case(path, "template/") => {
            Some(&path["template/".len()..])
        }
        _ => None,
    };

    let Some(candidate) = candidate else {
        return Err(if is_invite(&host, path) {
            TemplateLinkRefusal::InviteLink
        } else {
            TemplateLinkRefusal::NotATemplateLink
        });
    };

    if candidate.is_empty() || candidate.contains('/') {
        return Err(TemplateLinkRefusal::NotATemplateLink);
    }
    if !is_code(candidate) {
        return Err(TemplateLinkRefusal::InvalidCode);
    }
    Ok(TemplateCode(candidate.to_owned()))
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn is_invite(host: &str, path: &str) -> bool {
    host == "discord.gg"
        || (matches!(host, "discord.com" | "discordapp.com")
            && starts_with_ignore_case(path, "invite/"))
}

/// Codes Discord hands out are a dozen letters and digits; the class is a
/// little wider so that a longer code still reads, and narrow enough that it
/// can never hold a path separator or an escape.
fn is_code(text: &str) -> bool {
    (2..=64).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}
