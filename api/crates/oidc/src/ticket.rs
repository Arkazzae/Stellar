//! The ticket a signed-in account gets to start a link or a fresh sign-in at
//! a provider: bound to the account, its session, its security version and
//! the provider, short-lived, and good only for the kind of round trip it was
//! made for. The browser carries it to the start address, which checks it
//! against the session presenting it.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::flow::Actor;
use crate::sealed::{SealingKey, open, seal};

/// How long a ticket may wait, in seconds.
pub const TICKET_LIFETIME: i64 = 5 * 60;

const PURPOSE: &str = "stellarr.oidc.ticket.v1";
const MAX_TICKET: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketKind {
    Link,
    Reauthenticate,
}

impl TicketKind {
    fn name(self) -> &'static str {
        match self {
            Self::Link => "link",
            Self::Reauthenticate => "reauthenticate",
        }
    }
}

/// What a ticket says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ticket {
    pub actor: Actor,
    pub provider_id: String,
    expires_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketError {
    /// Not a ticket this server made for this kind of round trip.
    Unreadable,
    Expired,
}

impl fmt::Display for TicketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the ticket is refused: {self:?}")
    }
}

impl std::error::Error for TicketError {}

pub fn issue(
    key: &SealingKey,
    kind: TicketKind,
    actor: Actor,
    provider_id: &str,
    now: i64,
) -> Option<String> {
    let ticket = Ticket {
        actor,
        provider_id: provider_id.to_string(),
        expires_at: now + TICKET_LIFETIME,
    };
    seal(
        key,
        PURPOSE,
        kind.name(),
        &serde_json::to_vec(&ticket).ok()?,
    )
}

/// A ticket made for `kind`. One made for the other kind does not open: the
/// ticket for a fresh sign-in needs no proof, so it must never start a link.
pub fn read(
    key: &SealingKey,
    kind: TicketKind,
    text: &str,
    now: i64,
) -> Result<Ticket, TicketError> {
    if text.len() > MAX_TICKET {
        return Err(TicketError::Unreadable);
    }
    let json = open(key, PURPOSE, kind.name(), text).ok_or(TicketError::Unreadable)?;
    let ticket: Ticket = serde_json::from_slice(&json).map_err(|_| TicketError::Unreadable)?;
    if ticket.expires_at <= now {
        return Err(TicketError::Expired);
    }
    Ok(ticket)
}
