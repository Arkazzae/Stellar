//! The identity providers the operator named: the one at the top of the `Oidc`
//! section and the ones under `Oidc:Providers:<id>`.
//!
//! Which of them can really work, and under what id, is decided by the
//! `stellarr-oidc` crate; this reads the keys, resolves the secret files once
//! and refuses a start that names a provider no route could serve.

use std::collections::BTreeMap;

use stellarr_oidc::options::{DEFAULT_CALLBACK_PATH, Provider, ProviderSettings, Settings};

use super::{Config, SettingError};

/// The providers that can be used, the original one first and then the named
/// ones in the order of their ids.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OidcSettings {
    pub providers: Vec<Provider>,
    /// What the original provider's button is called as the operator wrote it,
    /// which the sign-in page shows while no provider can be used.
    pub display_name: String,
}

impl OidcSettings {
    pub(super) fn load(config: &Config) -> Result<OidcSettings, SettingError> {
        // The route that receives the provider's answer is fixed when the
        // routes are built, so the original provider's callback cannot move.
        if let Some(path) = config.text("Oidc:CallbackPath")
            && path != DEFAULT_CALLBACK_PATH
        {
            return Err(SettingError::new(format!(
                "Oidc:CallbackPath is \"{path}\" but must be \"{DEFAULT_CALLBACK_PATH}\": this server receives the provider's answer there only."
            )));
        }
        let mut settings = Settings {
            original: ProviderSettings {
                enabled: false,
                ..ProviderSettings::default()
            },
            ..Settings::default()
        };
        read_provider(config, "Oidc:", &mut settings.original)?;

        let mut named: BTreeMap<String, ProviderSettings> = BTreeMap::new();
        for (name, _) in config.prefixed("Oidc:Providers:") {
            let Some((id, _)) = name.split_once(':') else {
                continue;
            };
            if id.is_empty() || named.contains_key(id) {
                continue;
            }
            let mut provider = ProviderSettings::default();
            read_provider(config, &format!("Oidc:Providers:{id}:"), &mut provider)?;
            named.insert(id.to_string(), provider);
        }
        settings.named = named;

        let providers = settings
            .configured_providers(read_secret_file)
            .map_err(|error| SettingError::new(format!("Oidc:Providers: {error}.")))?;
        Ok(OidcSettings {
            providers,
            display_name: settings.original.display_name.clone(),
        })
    }

    pub fn find(&self, id: &str) -> Option<&Provider> {
        self.providers.iter().find(|provider| provider.id == id)
    }
}

/// What a secret file holds, or nothing when it cannot be read.
fn read_secret_file(path: &str) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

fn read_provider(
    config: &Config,
    prefix: &str,
    provider: &mut ProviderSettings,
) -> Result<(), SettingError> {
    let key = |name: &str| format!("{prefix}{name}");
    if let Some(enabled) = config.flag(&key("Enabled"))? {
        provider.enabled = enabled;
    }
    if let Some(trust) = config.flag(&key("TrustProviderMfa"))? {
        provider.trust_provider_mfa = trust;
    }
    // Text keeps what was written, an empty value included.
    let text = |name: &str, slot: &mut String| {
        if let Some(value) = config.raw(&key(name)) {
            *slot = value.to_string();
        }
    };
    text("Authority", &mut provider.authority);
    text("ClientId", &mut provider.client_id);
    text("ClientSecret", &mut provider.client_secret);
    text("ClientSecretFile", &mut provider.client_secret_file);
    text("DisplayName", &mut provider.display_name);
    text("ProviderMfaAmr", &mut provider.provider_mfa_amr);
    Ok(())
}
