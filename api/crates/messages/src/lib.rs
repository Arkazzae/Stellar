//! The catalogue of what the server says to people.
//!
//! Each key has one text per language. The language is a property of the
//! instance, not of a person or a request. Texts are plain strings with named
//! placeholders such as `{limit}`.

use std::fmt;

include!(concat!(env!("OUT_DIR"), "/catalogs.rs"));

impl Language {
    pub fn code(self) -> &'static str {
        CATALOGS[self as usize].0
    }

    /// Unset and unshipped instance languages use the English source catalog.
    pub fn resolve(code: &str) -> Self {
        Self::ALL
            .iter()
            .copied()
            .find(|language| language.code() == code)
            .unwrap_or(Self::English)
    }
}

/// A value for a placeholder: anything that can be written as text. It is
/// `Sync` so that a list of values can be held across an `.await`.
pub type Value<'a> = &'a (dyn fmt::Display + Sync);

/// A text has a placeholder that the caller gave no value for. This is a bug
/// in the calling code, never a fault of the person who sent the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingValue {
    pub key: MessageKey,
    pub placeholder: String,
}

impl fmt::Display for MissingValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} needs a value for {{{}}}",
            self.key, self.placeholder
        )
    }
}

impl std::error::Error for MissingValue {}

/// The catalogue in the language of the instance.
#[derive(Debug, Clone, Copy)]
pub struct Messages {
    language: Language,
}

impl Messages {
    pub fn new(language: Language) -> Messages {
        Messages { language }
    }

    pub fn language(&self) -> Language {
        self.language
    }

    /// The text of `key` with each placeholder replaced by the value of that
    /// name: `messages.get(MessageKey::PasswordTooShort, &[("min", &10)])`.
    /// Values are written the way `Display` writes them. A placeholder with no
    /// value is an error; a value that no placeholder uses is ignored.
    pub fn get(
        &self,
        key: MessageKey,
        values: &[(&str, Value<'_>)],
    ) -> Result<String, MissingValue> {
        let template = self.template(key);
        let mut result = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            let after = &rest[open + 1..];
            let name_len = after
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(after.len());
            let closes = after[name_len..].starts_with('}');
            if name_len == 0 || !closes {
                // Not a placeholder: keep the brace as written.
                result.push_str(&rest[..=open]);
                rest = after;
                continue;
            }
            let name = &after[..name_len];
            let Some((_, value)) = values.iter().find(|(given, _)| *given == name) else {
                return Err(MissingValue {
                    key,
                    placeholder: name.to_string(),
                });
            };
            result.push_str(&rest[..open]);
            result.push_str(&value.to_string());
            rest = &after[name_len + 1..];
        }
        result.push_str(rest);
        Ok(result)
    }

    /// The text with its placeholders still in place.
    pub fn template(&self, key: MessageKey) -> &'static str {
        CATALOGS[self.language as usize].1[key as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholders_of(template: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            let after = &rest[open + 1..];
            if let Some(close) = after.find('}') {
                found.push(after[..close].to_string());
                rest = &after[close + 1..];
            } else {
                break;
            }
        }
        found.sort();
        found
    }

    #[test]
    fn the_instance_speaks_english_when_no_language_is_set() {
        let language = Language::resolve("");

        assert_eq!(language.code(), "en");
        assert_eq!(
            Messages::new(language)
                .get(MessageKey::SignInFailed, &[])
                .unwrap(),
            "Invalid username or password."
        );
    }

    #[test]
    fn a_blank_language_is_the_same_as_none_at_all() {
        assert_eq!(Language::resolve("  "), Language::English);
        assert_eq!(Language::resolve("\t"), Language::English);
    }

    #[test]
    fn a_language_selects_its_own_texts() {
        for (code, expected) in [
            ("pl", "Nieprawidłowy login lub hasło."),
            ("de", "Benutzername oder Passwort ist falsch."),
            ("en", "Invalid username or password."),
        ] {
            let catalogue = Messages::new(Language::resolve(code));
            assert_eq!(
                catalogue.get(MessageKey::SignInFailed, &[]).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn an_unshipped_language_uses_the_english_catalog() {
        assert_eq!(Language::resolve("qaa-Test"), Language::English);
    }

    #[test]
    fn all_catalogs_cover_the_english_keys_and_placeholders() {
        let english = Messages::new(Language::English);
        for &language in Language::ALL {
            let catalog = Messages::new(language);
            for &key in MessageKey::ALL {
                let text = catalog.template(key);
                assert!(!text.trim().is_empty(), "{language:?}.{key:?} is empty");
                assert_eq!(
                    placeholders_of(text),
                    placeholders_of(english.template(key)),
                    "{language:?}.{key:?} has different placeholders from English"
                );
            }
        }
    }

    #[test]
    fn a_placeholder_takes_the_value_of_its_name() {
        for &language in Language::ALL {
            let text = Messages::new(language)
                .get(
                    MessageKey::ClientOutdated,
                    &[("reported", &"1.2.3"), ("minimum", &"2.0")],
                )
                .unwrap();

            assert!(text.contains("1.2.3"));
            assert!(text.contains("2.0"));
            assert!(!text.contains('{'));
        }
    }

    #[test]
    fn a_text_whose_placeholder_has_no_value_is_refused() {
        for &language in Language::ALL {
            let refusal = Messages::new(language)
                .get(MessageKey::ClientOutdated, &[("reported", &"1.2.3")])
                .unwrap_err();

            assert_eq!(refusal.key, MessageKey::ClientOutdated);
            assert_eq!(refusal.placeholder, "minimum");
        }
    }

    #[test]
    fn every_text_can_be_filled_with_values_for_its_placeholders() {
        for &language in Language::ALL {
            let catalogue = Messages::new(language);
            for &key in MessageKey::ALL {
                let names = placeholders_of(catalogue.template(key));
                let values: Vec<(&str, Value<'_>)> = names
                    .iter()
                    .map(|name| (name.as_str(), &1 as Value<'_>))
                    .collect();

                let text = catalogue.get(key, &values).unwrap();

                assert!(
                    !text.contains('{'),
                    "{key:?} still has a placeholder: {text}"
                );
            }
        }
    }

    #[test]
    fn polish_characters_and_dashes_survive_untouched() {
        let catalogue = Messages::new(Language::English);
        assert_eq!(
            catalogue.get(MessageKey::UsernameInvalid, &[]).unwrap(),
            "Username must be 3–32 characters: letters, digits, dot, hyphen or underscore."
        );
    }
}
