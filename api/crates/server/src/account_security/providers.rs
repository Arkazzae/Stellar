//! The identity providers an account can sign in through.
//!
//! They are the ones the settings name (see `identity_providers`). An identity
//! whose provider is no longer set up is named by the host of its issuer and
//! counts as a way in for nobody.

use crate::state::AppState;

/// A provider the instance is set up with.
pub struct Provider {
    pub id: String,
    pub display_name: String,
}

/// The providers this instance is set up with, in the order the sign-in
/// page offers them, named as the person sees them.
pub fn configured(state: &AppState) -> Vec<Provider> {
    state
        .settings()
        .oidc
        .providers
        .iter()
        .map(|provider| Provider {
            id: provider.id.clone(),
            display_name: crate::identity_providers::shown_name(provider, state.messages()).unwrap_or_default(),
        })
        .collect()
}
