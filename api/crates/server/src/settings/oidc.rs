//! The `Oidc` section: the identity providers people may sign in through.
//!
//! The provider written at the top of the section keeps the id `default` and
//! the original callback; more are written under `Oidc:Providers:{id}`, each
//! with a callback of its own. A provider counts only when it can really work:
//! it is enabled, its authority is an HTTPS address, and it has a client id and
//! a secret. One that is half written is left out, not refused, so that an
//! operator can keep a provider's settings while it is switched off.

use super::SettingError;
use super::config::Config;

/// The id of the provider written at the top of the section.
pub const DEFAULT_PROVIDER_ID: &str = "default";
/// Words a named provider cannot take: its own routes use them.
const RESERVED_IDS: [&str; 6] = [
    DEFAULT_PROVIDER_ID,
    "login",
    "callback",
    "link",
    "reauthenticate",
    "pending",
];
const DEFAULT_CALLBACK: &str = "/api/auth/oidc/callback";
const DEFAULT_MFA_AMR: &str = "mfa";

/// The providers an instance is set up with, the default one first and then
/// the named ones in the order of their ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcSettings {
    providers: Vec<Provider>,
}

/// One identity provider, as it is written, usable or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    pub id: String,
    pub enabled: bool,
    pub authority: String,
    pub client_id: String,
    pub client_secret: String,
    /// A file that holds the secret, for deployments that mount secrets.
    pub client_secret_file: String,
    pub display_name: String,
    pub trust_provider_mfa: bool,
    pub provider_mfa_amr: String,
    pub callback_path: String,
}

impl OidcSettings {
    pub(super) fn load(config: &Config) -> Result<OidcSettings, SettingError> {
        let mut providers = vec![Provider::read(
            config,
            "Oidc:",
            DEFAULT_PROVIDER_ID,
            false,
            config.text("Oidc:CallbackPath").unwrap_or(DEFAULT_CALLBACK),
        )?];
        let mut ids: Vec<String> = Vec::new();
        for (rest, _) in config.prefixed("Oidc:Providers:") {
            if let Some((id, _)) = rest.split_once(':')
                && !ids.iter().any(|known| known == id)
            {
                ids.push(id.to_string());
            }
        }
        ids.sort();
        for id in ids {
            if !valid_id(&id) {
                return Err(SettingError::new(format!(
                    "Oidc:Providers:{id} is not a valid provider id: use 1 to 32 lower-case \
                     letters, digits or hyphens, not {}.",
                    RESERVED_IDS.join(", ")
                )));
            }
            let callback = format!("/api/auth/oidc/{id}/callback");
            providers.push(Provider::read(
                config,
                &format!("Oidc:Providers:{id}:"),
                &id,
                true,
                &callback,
            )?);
        }
        Ok(OidcSettings { providers })
    }

    /// Every provider that can really be used, the default one first. A
    /// secret kept in a file is read now, so this does disk work.
    pub fn configured(&self) -> Vec<&Provider> {
        self.providers
            .iter()
            .filter(|provider| provider.is_configured())
            .collect()
    }
}

impl Provider {
    fn read(
        config: &Config,
        prefix: &str,
        id: &str,
        enabled_by_default: bool,
        callback_path: &str,
    ) -> Result<Provider, SettingError> {
        let key = |name: &str| format!("{prefix}{name}");
        let text = |name: &str| config.raw(&key(name)).unwrap_or("").to_string();
        Ok(Provider {
            id: id.to_string(),
            enabled: config.flag(&key("Enabled"))?.unwrap_or(enabled_by_default),
            authority: text("Authority"),
            client_id: text("ClientId"),
            client_secret: text("ClientSecret"),
            client_secret_file: text("ClientSecretFile"),
            display_name: text("DisplayName"),
            trust_provider_mfa: config.flag(&key("TrustProviderMfa"))?.unwrap_or(false),
            provider_mfa_amr: config
                .raw(&key("ProviderMfaAmr"))
                .unwrap_or(DEFAULT_MFA_AMR)
                .to_string(),
            callback_path: callback_path.to_string(),
        })
    }

    /// Where the sign-in button sends people.
    pub fn login_path(&self) -> String {
        if self.id == DEFAULT_PROVIDER_ID {
            "/api/auth/oidc/login".to_string()
        } else {
            format!("/api/auth/oidc/{}/login", self.id)
        }
    }

    /// The inline secret, else the content of the secret file, else nothing.
    pub fn resolve_client_secret(&self) -> String {
        let inline = self.client_secret.trim();
        let path = self.client_secret_file.trim();
        if !inline.is_empty() || path.is_empty() {
            return inline.to_string();
        }
        std::fs::read_to_string(path)
            .map(|content| content.trim().to_string())
            .unwrap_or_default()
    }

    /// Whether the provider can really work.
    pub fn is_configured(&self) -> bool {
        self.enabled
            && is_https_address(&self.authority)
            && !self.client_id.trim().is_empty()
            && !self.resolve_client_secret().is_empty()
            && self.callback_path.starts_with('/')
    }
}

fn valid_id(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !RESERVED_IDS.contains(&id)
}

/// Whether the text is an absolute address with the scheme `https` and a host:
/// what the provider's authority must be. White space around it does not
/// count, as for any address.
fn is_https_address(text: &str) -> bool {
    let text = text.trim();
    let Some((scheme, rest)) = text.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let authority = rest.split(['/', '\\', '?', '#']).next().unwrap_or("");
    let host_and_port = authority.rsplit('@').next().unwrap_or("");
    let (host, port) = if let Some(inside) = host_and_port.strip_prefix('[') {
        let Some((host, after)) = inside.split_once(']') else {
            return false;
        };
        match after {
            "" => (host, None),
            after => match after.strip_prefix(':') {
                Some(port) => (host, Some(port)),
                None => return false,
            },
        }
    } else {
        match host_and_port.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_and_port, None),
        }
    };
    let port_fine = port.is_none_or(|port| {
        port.is_empty()
            || (port.bytes().all(|b| b.is_ascii_digit())
                && port.parse::<u32>().is_ok_and(|n| n <= 65535))
    });
    let host_fine = !host.is_empty()
        && host
            .chars()
            .all(|c| !c.is_whitespace() && !c.is_control() && !"<>\"{}|^`%".contains(c));
    port_fine && host_fine
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(pairs: &[(&str, &str)]) -> Config {
        let mut config = Config::default();
        for (key, value) in pairs {
            config.set(key, value);
        }
        config
    }

    fn working() -> Vec<(&'static str, &'static str)> {
        vec![
            ("Oidc:Enabled", "true"),
            ("Oidc:Authority", "https://identity.test"),
            ("Oidc:ClientId", "stellarr"),
            ("Oidc:ClientSecret", "secret"),
        ]
    }

    #[test]
    fn nothing_is_configured_until_the_provider_is_enabled_and_complete() {
        assert!(
            OidcSettings::load(&config(&[]))
                .unwrap()
                .configured()
                .is_empty()
        );
        for missing in [
            "Oidc:Enabled",
            "Oidc:Authority",
            "Oidc:ClientId",
            "Oidc:ClientSecret",
        ] {
            let pairs: Vec<_> = working()
                .into_iter()
                .filter(|(key, _)| *key != missing)
                .collect();
            let settings = OidcSettings::load(&config(&pairs)).unwrap();
            assert!(settings.configured().is_empty(), "{missing}");
        }
        let settings = OidcSettings::load(&config(&working())).unwrap();
        let found = settings.configured();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "default");
        assert_eq!(found[0].login_path(), "/api/auth/oidc/login");
        assert_eq!(found[0].callback_path, "/api/auth/oidc/callback");
    }

    #[test]
    fn the_authority_must_be_an_https_address() {
        for good in [
            "https://identity.test",
            " HTTPS://id.test:8443/realm ",
            "https://[::1]:8443/x",
            "https://u:p@id.test",
        ] {
            assert!(is_https_address(good), "{good}");
        }
        for bad in [
            "http://identity.test",
            "identity.test",
            "https://",
            "https:///x",
            "https://id.test:99999",
            "https://bad host",
            "ftp://id.test",
            "",
        ] {
            assert!(!is_https_address(bad), "{bad}");
        }
    }

    #[test]
    fn a_secret_may_come_from_a_file_and_the_inline_one_wins() {
        let folder = std::env::temp_dir().join(format!("stellarr-oidc-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("secret");
        std::fs::write(&file, "  from-file \n").unwrap();
        let path = file.to_str().unwrap();
        let without_inline = [
            ("Oidc:Enabled", "true"),
            ("Oidc:Authority", "https://identity.test"),
            ("Oidc:ClientId", "stellarr"),
            ("Oidc:ClientSecretFile", path),
        ];
        let settings = OidcSettings::load(&config(&without_inline)).unwrap();
        assert_eq!(
            settings.configured()[0].resolve_client_secret(),
            "from-file"
        );
        let mut both = without_inline.to_vec();
        both.push(("Oidc:ClientSecret", " inline "));
        let settings = OidcSettings::load(&config(&both)).unwrap();
        assert_eq!(settings.configured()[0].resolve_client_secret(), "inline");
        std::fs::remove_dir_all(&folder).unwrap();
        let settings = OidcSettings::load(&config(&without_inline)).unwrap();
        assert!(
            settings.configured().is_empty(),
            "a file that cannot be read is no secret"
        );
    }

    #[test]
    fn named_providers_follow_the_default_one_in_the_order_of_their_ids() {
        let mut pairs = working();
        pairs.extend([
            ("Oidc:Providers:work:Authority", "https://work.test"),
            ("Oidc:Providers:work:ClientId", "w"),
            ("Oidc:Providers:work:ClientSecret", "s"),
            ("Oidc:Providers:home:Authority", "https://home.test"),
            ("Oidc:Providers:home:ClientId", "h"),
            ("Oidc:Providers:home:ClientSecret", "s"),
            ("Oidc:Providers:off:Authority", "https://off.test"),
            ("Oidc:Providers:off:ClientId", "o"),
            ("Oidc:Providers:off:ClientSecret", "s"),
            ("Oidc:Providers:off:Enabled", "false"),
        ]);
        let settings = OidcSettings::load(&config(&pairs)).unwrap();
        let found = settings.configured();

        assert_eq!(
            found.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["default", "home", "work"]
        );
        assert_eq!(found[2].login_path(), "/api/auth/oidc/work/login");
        assert_eq!(found[2].callback_path, "/api/auth/oidc/work/callback");
        // A named provider is on unless it says otherwise; the default one is off.
        assert!(found[1].enabled);
    }

    #[test]
    fn a_provider_id_that_cannot_be_registered_stops_the_start() {
        for bad in [
            "Work",
            "-x",
            "login",
            "default",
            "a_b",
            "a-very-long-id-that-goes-past-thirty-two-characters",
        ] {
            let pairs = [(
                format!("Oidc:Providers:{bad}:Authority"),
                "https://x.test".to_string(),
            )];
            let mut c = Config::default();
            for (k, v) in &pairs {
                c.set(k, v);
            }
            let refused = OidcSettings::load(&c).unwrap_err();
            assert!(refused.to_string().contains(bad), "{refused}");
        }
    }

    #[test]
    fn a_flag_that_is_not_true_or_false_stops_the_start() {
        let refused = OidcSettings::load(&config(&[("Oidc:Enabled", "yes")])).unwrap_err();
        assert!(refused.to_string().contains("Oidc:Enabled"), "{refused}");
    }
}
