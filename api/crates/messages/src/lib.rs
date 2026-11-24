//! The catalogue of what the server says to people.
//!
//! Each key has one text per language. The language is a property of the
//! instance, not of a person or a request. Texts are plain strings with named
//! placeholders such as `{limit}`.

mod english;
mod key;
mod polish;

use std::fmt;

pub use key::MessageKey;

/// A language the catalogue has texts for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    English,
    Polish,
}

impl Language {
    /// The code used in settings and in the page's `lang` attribute.
    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Polish => "pl",
        }
    }

    /// Reads a setting value. A blank value means English: that is how a
    /// variable that a deployment file passes on without setting it arrives.
    pub fn parse(value: &str) -> Result<Language, UnsupportedLanguage> {
        match value {
            "en" => Ok(Language::English),
            "pl" => Ok(Language::Polish),
            blank if blank.trim().is_empty() => Ok(Language::English),
            other => Err(UnsupportedLanguage(other.to_string())),
        }
    }
}

/// The setting names a language the catalogue has no texts for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedLanguage(pub String);

impl fmt::Display for UnsupportedLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "language is \"{}\" but must be \"en\" or \"pl\"", self.0)
    }
}

impl std::error::Error for UnsupportedLanguage {}

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
        match self.language {
            Language::English => english::text(key),
            Language::Polish => polish::text(key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LANGUAGES: [Language; 2] = [Language::English, Language::Polish];

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
        let language = Language::parse("").unwrap();

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
        assert_eq!(Language::parse("  ").unwrap(), Language::English);
        assert_eq!(Language::parse("\t").unwrap(), Language::English);
    }

    #[test]
    fn a_language_selects_its_own_texts() {
        for (code, expected) in [
            ("pl", "Nieprawidłowy login lub hasło."),
            ("en", "Invalid username or password."),
        ] {
            let catalogue = Messages::new(Language::parse(code).unwrap());
            assert_eq!(
                catalogue.get(MessageKey::SignInFailed, &[]).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn an_unknown_language_is_refused() {
        let refusal = Language::parse("de").unwrap_err();

        assert_eq!(
            refusal.to_string(),
            "language is \"de\" but must be \"en\" or \"pl\""
        );
        assert!(Language::parse("EN").is_err());
        assert!(Language::parse(" en").is_err());
    }

    #[test]
    fn both_languages_tell_the_same_thing() {
        let english = Messages::new(Language::English);
        let polish = Messages::new(Language::Polish);

        for &key in MessageKey::ALL {
            assert!(
                !english.template(key).trim().is_empty(),
                "{key:?} has no English text"
            );
            assert!(
                !polish.template(key).trim().is_empty(),
                "{key:?} has no Polish text"
            );
            assert_eq!(
                placeholders_of(english.template(key)),
                placeholders_of(polish.template(key)),
                "{key:?} has different placeholders in English and in Polish"
            );
        }
    }

    #[test]
    fn the_catalogue_holds_every_key_the_server_uses() {
        assert_eq!(MessageKey::ALL.len(), 367);
    }

    #[test]
    fn a_placeholder_takes_the_value_of_its_name() {
        for language in LANGUAGES {
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
        for language in LANGUAGES {
            let refusal = Messages::new(language)
                .get(MessageKey::ClientOutdated, &[("reported", &"1.2.3")])
                .unwrap_err();

            assert_eq!(refusal.key, MessageKey::ClientOutdated);
            assert_eq!(refusal.placeholder, "minimum");
        }
    }

    #[test]
    fn every_text_can_be_filled_with_values_for_its_placeholders() {
        for language in LANGUAGES {
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
