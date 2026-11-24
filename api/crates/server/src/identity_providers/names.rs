//! What a provider is called, which the first server did in two ways.

use stellarr_messages::{MessageKey, Messages, MissingValue};
use stellarr_oidc::options::Provider;

use crate::state::AppState;

/// What the provider's button says, and what the security page and the
/// sign-in page show: the operator's name for it, else the instance's word
/// for the account that comes with it, in its language.
pub fn shown_name(provider: &Provider, messages: &Messages) -> Result<String, MissingValue> {
    display_name_or(&provider.settings.display_name, messages)
}

/// The same for a name that may belong to no provider that works.
pub fn display_name_or(name: &str, messages: &Messages) -> Result<String, MissingValue> {
    if name.trim().is_empty() {
        messages.get(MessageKey::OidcDefaultName, &[])
    } else {
        Ok(name.to_string())
    }
}

/// What the history of the account says about the provider an identity was
/// linked through: the operator's name for it, else its id. A name that is
/// only a word of the language would change with the instance's language,
/// which a record must not.
pub fn recorded_name(provider: &Provider) -> String {
    if provider.settings.display_name.trim().is_empty() {
        provider.id.clone()
    } else {
        provider.settings.display_name.clone()
    }
}

/// [`recorded_name`] of the provider with this id, when the instance has such
/// a provider set up. A link to name in the account's history is made only
/// for one that has.
pub fn recorded_name_of(state: &AppState, id: &str) -> Option<String> {
    state.settings().oidc.find(id).map(recorded_name)
}

/// [`shown_name`] of the provider with this id, when the instance has one.
pub fn shown_name_of(state: &AppState, id: &str) -> Option<Result<String, MissingValue>> {
    state
        .settings()
        .oidc
        .find(id)
        .map(|provider| shown_name(provider, state.messages()))
}
