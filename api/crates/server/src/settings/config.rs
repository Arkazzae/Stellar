//! The layered keys every setting is read from.
//!
//! A key is a path of names joined by `:`, such as `Rtc:TurnSharedSecret`.
//! Names are matched without regard to letter case. A value comes from the
//! highest layer that has the key:
//!
//! 1. the built-in defaults of the environment the API runs in,
//! 2. raw environment variables, where `__` stands for `:` (`Rtc__MediaPort`),
//! 3. the `STELLARR_*` variables, which win over the raw form of the same key.
//!
//! A raw variable that is set to nothing is a value, an empty one. A
//! `STELLARR_*` variable that is empty or only whitespace counts as not set,
//! because that is how a deployment file passes on a variable it never filled.

use std::collections::HashMap;

use super::SettingError;
use super::defaults;
use super::variables::VARIABLES;

#[derive(Debug, Clone, Default)]
pub struct Config {
    /// By lower-cased key; the key is kept as first written, for names that
    /// become something case-sensitive, such as a log target.
    values: HashMap<String, (String, String)>,
}

impl Config {
    /// Builds the layers for `environment` from the process environment, given
    /// as `(name, value)` pairs.
    pub fn layered(environment: &str, variables: &[(String, String)]) -> Config {
        let mut config = Config::default();
        for (key, value) in defaults::BASE {
            config.set(key, value);
        }
        let overlay = if environment.eq_ignore_ascii_case("Development") {
            defaults::DEVELOPMENT
        } else if environment.eq_ignore_ascii_case("Production") {
            defaults::PRODUCTION
        } else {
            &[]
        };
        for (key, value) in overlay {
            config.set(key, value);
        }
        for (name, value) in variables {
            config.set(&name.replace("__", ":"), value);
        }
        for variable in VARIABLES {
            let Some(key) = variable.key else { continue };
            let named = variables
                .iter()
                .find(|(name, _)| name == variable.name)
                .map(|(_, value)| value);
            if let Some(value) = named.filter(|value| !value.trim().is_empty()) {
                config.set(key, value);
            }
        }
        config
    }

    /// Sets one key. Used by the layers above, and by tests that need a value
    /// the environment does not carry.
    pub fn set(&mut self, key: &str, value: &str) {
        self.values.insert(
            key.to_ascii_lowercase(),
            (key.to_string(), value.to_string()),
        );
    }

    /// The value as written, possibly empty.
    pub fn raw(&self, key: &str) -> Option<&str> {
        self.values
            .get(&key.to_ascii_lowercase())
            .map(|(_, value)| value.as_str())
    }

    /// The value without surrounding whitespace; none when it is not set or
    /// blank.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.raw(key)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    /// Whether the key is set to `true` or `false`, in any letter case.
    pub fn flag(&self, key: &str) -> Result<Option<bool>, SettingError> {
        match self.text(key) {
            None => Ok(None),
            Some(value) if value.eq_ignore_ascii_case("true") => Ok(Some(true)),
            Some(value) if value.eq_ignore_ascii_case("false") => Ok(Some(false)),
            Some(value) => Err(SettingError::new(format!(
                "{key} must be \"true\" or \"false\", but is \"{value}\"."
            ))),
        }
    }

    /// A whole number, or none when the key is not set or blank.
    pub fn number(&self, key: &str) -> Result<Option<i64>, SettingError> {
        match self.text(key) {
            None => Ok(None),
            Some(value) => value.parse::<i64>().map(Some).map_err(|_| {
                SettingError::new(format!("{key} must be a whole number, but is \"{value}\"."))
            }),
        }
    }

    /// Every `(name, value)` below a prefix such as `Logging:LogLevel:`, with
    /// the prefix removed and the name's letter case as first written. Only
    /// names that are one level deep are returned.
    pub fn prefixed(&self, prefix: &str) -> Vec<(String, String)> {
        let lowered = prefix.to_ascii_lowercase();
        self.values
            .iter()
            .filter(|(key, _)| key.starts_with(&lowered))
            .map(|(_, (key, value))| (key[prefix.len()..].to_string(), value.clone()))
            .collect()
    }

    /// The items of a list key, written `Key:0`, `Key:1` and so on. Items that
    /// are blank are left out.
    pub fn list(&self, key: &str) -> Vec<String> {
        let mut items = Vec::new();
        let mut index = 0;
        while let Some(value) = self.raw(&format!("{key}:{index}")) {
            if !value.trim().is_empty() {
                items.push(value.trim().to_string());
            }
            index += 1;
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn a_variable_lands_on_its_key_and_nowhere_else() {
        let config = Config::layered("Production", &env(&[("STELLARR_GIPHY_API_KEY", "abc")]));

        assert_eq!(config.raw("Giphy:ApiKey"), Some("abc"));
        assert_eq!(config.raw("giphy:apikey"), Some("abc"));
    }

    #[test]
    fn a_blank_variable_counts_as_not_set() {
        for blank in ["", "   ", " \t\r\n"] {
            let config = Config::layered(
                "Production",
                &env(&[
                    ("STELLARR_MAIL_FROM_NAME", blank),
                    ("STELLARR_MAIL_FROM", blank),
                    ("Mail__FromAddress", "raw@example.test"),
                ]),
            );

            assert_eq!(config.raw("Mail:FromName"), Some("Stellarr"));
            assert_eq!(config.raw("Mail:FromAddress"), Some("raw@example.test"));
        }
    }

    #[test]
    fn a_variable_wins_over_the_raw_key_and_keeps_its_spaces() {
        let config = Config::layered(
            "Production",
            &env(&[
                ("Mail__SmtpPassword", "raw"),
                ("STELLARR_SMTP_PASSWORD", " named with spaces "),
                ("Mail__FromName", "raw"),
            ]),
        );

        assert_eq!(config.raw("Mail:SmtpPassword"), Some(" named with spaces "));
        assert_eq!(config.raw("Mail:FromName"), Some("raw"));
    }

    #[test]
    fn a_raw_variable_set_to_nothing_is_an_empty_value_not_a_missing_one() {
        let config = Config::layered("Production", &env(&[("Clients__DownloadPath", "")]));

        assert_eq!(config.raw("Clients:DownloadPath"), Some(""));
        assert_eq!(config.text("Clients:DownloadPath"), None);
    }

    #[test]
    fn the_environment_picks_its_own_defaults() {
        let production = Config::layered("Production", &[]);
        let development = Config::layered("development", &[]);
        let testing = Config::layered("Testing", &[]);

        assert_eq!(production.raw("Mail:Enabled"), Some("true"));
        assert_eq!(development.raw("Mail:Enabled"), Some("false"));
        assert_eq!(testing.raw("Mail:Enabled"), Some("false"));
        assert_eq!(production.raw("Backups:Enabled"), Some("true"));
        assert_eq!(development.raw("Rtc:PublicHost"), Some("localhost"));
        assert_eq!(production.raw("Rtc:PublicHost"), None);
    }

    #[test]
    fn typed_readers_refuse_what_they_cannot_read() {
        let config = Config::layered(
            "Production",
            &env(&[("Rtc__MediaPort", "many"), ("Mail__Enabled", "maybe")]),
        );

        assert!(
            config
                .number("Rtc:MediaPort")
                .unwrap_err()
                .to_string()
                .contains("Rtc:MediaPort")
        );
        assert!(
            config
                .flag("Mail:Enabled")
                .unwrap_err()
                .to_string()
                .contains("Mail:Enabled")
        );
        assert_eq!(config.number("Rtc:Nothing").unwrap(), None);
    }

    #[test]
    fn a_list_collects_numbered_items() {
        let config = Config::layered(
            "Production",
            &env(&[
                ("Clients__Allowed__0", "web"),
                ("Clients__Allowed__1", "kiosk"),
            ]),
        );

        assert_eq!(config.list("Clients:Allowed"), ["web", "kiosk"]);
    }
}
