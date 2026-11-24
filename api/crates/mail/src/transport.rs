//! Which way a message leaves: SMTP or the Resend service.

use std::fmt;

use stellarr_smtp::{Connection, Security};

/// The default port of an SMTP submission server.
pub const DEFAULT_SMTP_PORT: u16 = 587;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Smtp,
    Resend,
}

impl Provider {
    /// The name the settings and the operator panel use.
    pub fn name(self) -> &'static str {
        match self {
            Provider::Smtp => "smtp",
            Provider::Resend => "resend",
        }
    }
}

/// The operator's mail settings as the database holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailSettings {
    pub smtp_host: String,
    pub smtp_port: u16,
    /// `starttls`, `tls` or `none`; anything else counts as `starttls`.
    pub smtp_security: String,
    pub smtp_username: String,
    /// `None` when the operator never chose.
    pub resend_enabled: Option<bool>,
}

impl Default for MailSettings {
    fn default() -> MailSettings {
        MailSettings {
            smtp_host: String::new(),
            smtp_port: DEFAULT_SMTP_PORT,
            smtp_security: "starttls".into(),
            smtp_username: String::new(),
            resend_enabled: None,
        }
    }
}

/// What outgoing mail goes through. `ready` is false while the chosen
/// provider lacks what it needs: an SMTP host, or the Resend key.
#[derive(Clone, PartialEq, Eq)]
pub struct MailTransport {
    pub provider: Provider,
    /// Present for SMTP.
    pub smtp: Option<Connection>,
    pub ready: bool,
    /// Empty unless the provider is Resend.
    pub resend_api_key: String,
}

// Printing a transport must never print a secret.
impl fmt::Debug for MailTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MailTransport")
            .field("provider", &self.provider)
            .field("smtp", &self.smtp)
            .field("ready", &self.ready)
            .finish_non_exhaustive()
    }
}

impl MailTransport {
    /// SMTP is the main transport; Resend replaces it when the operator turns
    /// it on. An installation that never chose (no flag) keeps sending through
    /// Resend when it has a Resend key and no SMTP host, as it did before
    /// SMTP existed.
    ///
    /// `settings` is `None` before the operator has saved any.
    pub fn resolve(
        settings: Option<&MailSettings>,
        resend_api_key: &str,
        smtp_password: &str,
    ) -> MailTransport {
        let host = settings.map_or("", |s| s.smtp_host.trim());
        let has_key = !resend_api_key.trim().is_empty();
        let provider = match settings.and_then(|s| s.resend_enabled) {
            Some(true) => Provider::Resend,
            Some(false) => Provider::Smtp,
            None if has_key && host.is_empty() => Provider::Resend,
            None => Provider::Smtp,
        };

        if provider == Provider::Resend {
            return MailTransport {
                provider,
                smtp: None,
                ready: has_key,
                resend_api_key: resend_api_key.trim().to_owned(),
            };
        }

        let connection = Connection {
            host: host.to_owned(),
            port: settings.map_or(DEFAULT_SMTP_PORT, |s| s.smtp_port),
            security: Security::parse(settings.map(|s| s.smtp_security.as_str())),
            username: settings.map_or("", |s| s.smtp_username.trim()).to_owned(),
            password: smtp_password.to_owned(),
        };
        MailTransport {
            provider,
            smtp: Some(connection),
            ready: !host.is_empty(),
            resend_api_key: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(host: &str, resend: Option<bool>) -> MailSettings {
        MailSettings {
            smtp_host: host.into(),
            smtp_port: 2525,
            smtp_security: " TLS ".into(),
            smtp_username: " user ".into(),
            resend_enabled: resend,
        }
    }

    #[test]
    fn smtp_is_ready_with_a_host_and_carries_the_trimmed_settings() {
        let transport =
            MailTransport::resolve(Some(&settings(" mail.chat.test ", None)), "", "secret");

        assert_eq!(transport.provider, Provider::Smtp);
        assert!(transport.ready);
        let smtp = transport.smtp.unwrap();
        assert_eq!(
            (
                smtp.host.as_str(),
                smtp.port,
                smtp.security,
                smtp.username.as_str(),
                smtp.password.as_str()
            ),
            ("mail.chat.test", 2525, Security::Tls, "user", "secret")
        );
    }

    #[test]
    fn smtp_without_a_host_is_not_ready() {
        let transport = MailTransport::resolve(Some(&settings("  ", Some(false))), "re_key", "");

        assert_eq!(transport.provider, Provider::Smtp);
        assert!(!transport.ready);
        assert_eq!(transport.resend_api_key, "");
    }

    #[test]
    fn resend_replaces_smtp_when_the_operator_turns_it_on() {
        let transport = MailTransport::resolve(
            Some(&settings("mail.chat.test", Some(true))),
            " re_key ",
            "p",
        );

        assert_eq!(transport.provider, Provider::Resend);
        assert!(transport.ready);
        assert_eq!(transport.resend_api_key, "re_key");
        assert_eq!(transport.smtp, None);
    }

    #[test]
    fn resend_turned_on_without_a_key_is_not_ready() {
        let transport = MailTransport::resolve(Some(&settings("", Some(true))), "  ", "");

        assert_eq!(transport.provider, Provider::Resend);
        assert!(!transport.ready);
    }

    #[test]
    fn an_installation_that_never_chose_follows_what_it_has() {
        let key_only = MailTransport::resolve(Some(&settings("", None)), "re_key", "");
        let both = MailTransport::resolve(Some(&settings("mail.chat.test", None)), "re_key", "");
        let nothing = MailTransport::resolve(Some(&settings("", None)), "", "");

        assert_eq!(key_only.provider, Provider::Resend);
        assert!(key_only.ready);
        assert_eq!(both.provider, Provider::Smtp);
        assert_eq!(nothing.provider, Provider::Smtp);
        assert!(!nothing.ready);
    }

    #[test]
    fn without_any_settings_the_defaults_apply() {
        let transport = MailTransport::resolve(None, "", "");
        let with_key = MailTransport::resolve(None, "re_key", "");

        let smtp = transport.smtp.unwrap();
        assert_eq!((smtp.port, smtp.security), (587, Security::StartTls));
        assert!(!transport.ready);
        assert_eq!(with_key.provider, Provider::Resend);
    }

    #[test]
    fn smtp_is_the_main_transport() {
        let with_host = MailSettings {
            smtp_host: " smtp.example.net ".into(),
            smtp_port: 465,
            smtp_security: "tls".into(),
            smtp_username: "mailer".into(),
            resend_enabled: None,
        };

        let transport = MailTransport::resolve(Some(&with_host), "re_x", "secret");

        assert_eq!(
            (transport.provider, transport.ready),
            (Provider::Smtp, true)
        );
        assert_eq!(
            transport.smtp,
            Some(Connection {
                host: "smtp.example.net".into(),
                port: 465,
                security: Security::Tls,
                username: "mailer".into(),
                password: "secret".into(),
            })
        );
        assert!(!MailTransport::resolve(Some(&MailSettings::default()), "", "").ready);
        let host_only = MailSettings {
            smtp_host: "h".into(),
            ..MailSettings::default()
        };
        let connection = MailTransport::resolve(Some(&host_only), "", "")
            .smtp
            .unwrap();
        assert_eq!(connection.security, Security::StartTls);
    }

    #[test]
    fn resend_replaces_smtp_when_chosen_or_when_it_is_all_there_is() {
        for (resend, key, host, provider, ready) in [
            (None, "re_x", "", Provider::Resend, true),
            (None, "re_x", "smtp.example.net", Provider::Smtp, true),
            (None, "", "", Provider::Smtp, false),
            (Some(false), "re_x", "", Provider::Smtp, false),
            (Some(true), "", "smtp.example.net", Provider::Resend, false),
            (
                Some(true),
                "re_x",
                "smtp.example.net",
                Provider::Resend,
                true,
            ),
        ] {
            let settings = MailSettings {
                resend_enabled: resend,
                smtp_host: host.into(),
                ..MailSettings::default()
            };

            let transport = MailTransport::resolve(Some(&settings), key, "");

            assert_eq!(
                (transport.provider, transport.ready),
                (provider, ready),
                "{resend:?} {key:?} {host:?}"
            );
        }
    }

    #[test]
    fn printing_a_transport_leaves_the_secrets_out() {
        let smtp = MailTransport::resolve(
            Some(&settings("mail.chat.test", Some(false))),
            "",
            "hunter2",
        );
        let resend = MailTransport::resolve(Some(&settings("", Some(true))), "re_secret", "");

        assert!(!format!("{smtp:?}").contains("hunter2"));
        assert!(!format!("{resend:?}").contains("re_secret"));
        assert_eq!(Provider::Resend.name(), "resend");
        assert_eq!(Provider::Smtp.name(), "smtp");
    }
}
