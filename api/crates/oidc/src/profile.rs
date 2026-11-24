//! What the provider says about a person, from the ID token first and then
//! from the userinfo endpoint, whose profile wins where it has one. Values
//! longer than the application's columns are cut, never refused.
//!
//! Lengths count UTF-16 units, as the application's columns do, and a cut
//! never splits a surrogate pair.

use serde_json::{Map, Value};

const MAX_EMAIL: usize = 320;
const MAX_NAME: usize = 64;
const MAX_USERNAME: usize = 64;
const MAX_AMR: usize = 256;
const MAX_AUTH_TIME: usize = 32;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    pub email: Option<String>,
    pub email_verified: bool,
    pub name: Option<String>,
    pub preferred_username: Option<String>,
    /// The `amr` values joined with commas.
    pub amr: String,
    pub auth_time: Option<String>,
}

impl Profile {
    /// The profile an ID token's claims carry.
    pub fn from_claims(claims: &Map<String, Value>) -> Self {
        let claim = |name: &str| claims.get(name).and_then(claim_text);
        let amr = claims.get("amr").map(claim_values).unwrap_or_default();
        Self {
            email: address(claim("email").as_deref()),
            email_verified: claim("email_verified").is_some_and(|v| v.eq_ignore_ascii_case("true")),
            name: clip(
                claim("name")
                    .or_else(|| {
                        join_names(
                            claim("given_name").as_deref(),
                            claim("family_name").as_deref(),
                        )
                    })
                    .as_deref(),
                MAX_NAME,
            ),
            preferred_username: clip(claim("preferred_username").as_deref(), MAX_USERNAME),
            amr: clip(Some(&amr.join(",")), MAX_AMR).unwrap_or_default(),
            auth_time: clip(claim("auth_time").as_deref(), MAX_AUTH_TIME),
        }
    }

    /// Adds what the userinfo endpoint answered with. What it says about the
    /// address counts only where it says it: a repeat of the token's address
    /// that leaves the verification out keeps the token's word for it, and
    /// another address is verified only when userinfo says so.
    pub fn apply_userinfo(&mut self, info: &Map<String, Value>) {
        let text = |name: &str| info.get(name).and_then(Value::as_str);
        if let Some(email) = address(text("email")) {
            let stated = info.get("email_verified").map(|v| {
                *v == Value::Bool(true)
                    || text("email_verified").is_some_and(|s| s.eq_ignore_ascii_case("true"))
            });
            let same = self
                .email
                .as_deref()
                .is_some_and(|own| own.eq_ignore_ascii_case(&email));
            if !same {
                self.email = Some(email);
                self.email_verified = stated == Some(true);
            } else if let Some(stated) = stated {
                self.email_verified = stated;
            }
        }
        let name = text("name")
            .map(str::to_string)
            .or_else(|| join_names(text("given_name"), text("family_name")));
        if let Some(name) = clip(name.as_deref(), MAX_NAME) {
            self.name = Some(name);
        }
        if let Some(username) = clip(text("preferred_username"), MAX_USERNAME) {
            self.preferred_username = Some(username);
        }
    }

    /// The address, only when the provider verified it.
    pub fn verified_email(&self) -> Option<&str> {
        self.email.as_deref().filter(|_| self.email_verified)
    }
}

/// A claim's value as text. A list gives its first element; a number gives
/// its digits; an object gives nothing.
pub(crate) fn claim_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Array(items) => items.first().and_then(claim_text),
        _ => None,
    }
}

/// Every value of a claim that may be one string or a list of them.
pub(crate) fn claim_values(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items.iter().filter_map(claim_text).collect(),
        other => claim_text(other).into_iter().collect(),
    }
}

/// The address when it is plausible: one `@`, a local part and a domain, no
/// spaces, quotes, brackets or list separators, and no longer than 320 units.
/// A narrower reading than the platform's own address parser, on purpose: it
/// accepts only a bare address, never a display name around one.
fn address(value: Option<&str>) -> Option<String> {
    let email = value?.trim();
    let (local, domain) = email.split_once('@')?;
    let plain = |c: char| !c.is_whitespace() && !c.is_control() && !"()<>[]\\,;:\"@".contains(c);
    let good = !email.is_empty()
        && email.encode_utf16().count() <= MAX_EMAIL
        && local.chars().count() <= 64
        && !local.is_empty()
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local.chars().all(plain)
        && !domain.is_empty()
        && !domain.starts_with(['.', '-'])
        && !domain.ends_with(['.', '-'])
        && !domain.contains("..")
        && domain.chars().all(plain);
    good.then(|| email.to_string())
}

fn join_names(given: Option<&str>, family: Option<&str>) -> Option<String> {
    let parts: Vec<&str> = [given, family]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Trims, then cuts to `maximum` UTF-16 units without splitting a pair.
/// `None` when nothing is left.
fn clip(value: Option<&str>, maximum: usize) -> Option<String> {
    let text = value?.trim();
    if text.is_empty() {
        return None;
    }
    let mut units = 0;
    let mut end = 0;
    for (index, c) in text.char_indices() {
        units += c.len_utf16();
        if units > maximum {
            break;
        }
        end = index + c.len_utf8();
    }
    Some(text[..end].trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn claims(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn reads_the_profile_from_the_token() {
        let profile = Profile::from_claims(&claims(json!({
            "name": "Ola Łęcka", "email": "ola@example.test", "email_verified": true,
            "preferred_username": "ola", "amr": ["pwd", "mfa"], "auth_time": 1700000000,
        })));
        assert_eq!(profile.name.as_deref(), Some("Ola Łęcka"));
        assert_eq!(profile.verified_email(), Some("ola@example.test"));
        assert_eq!(profile.amr, "pwd,mfa");
        assert_eq!(profile.auth_time.as_deref(), Some("1700000000"));
    }

    #[test]
    fn email_verified_counts_when_it_reads_true_in_any_case() {
        for (value, expected) in [
            (json!(true), true),
            (json!("true"), true),
            (json!("TRUE"), true),
            (json!("True"), true),
            (json!(false), false),
            (json!("false"), false),
            (json!("yes"), false),
            (json!(1), false),
        ] {
            let profile = Profile::from_claims(&claims(
                json!({"email": "a@b.test", "email_verified": value}),
            ));
            assert_eq!(profile.email_verified, expected, "{value}");
        }
        assert_eq!(
            Profile::from_claims(&claims(json!({"email": "a@b.test"}))).verified_email(),
            None
        );
        assert_eq!(
            Profile::from_claims(&claims(json!({"email_verified": true}))).verified_email(),
            None
        );
    }

    #[test]
    fn name_falls_back_to_given_and_family_names() {
        let joined = Profile::from_claims(&claims(
            json!({"given_name": " Ewa ", "family_name": "Nowak"}),
        ));
        assert_eq!(joined.name.as_deref(), Some("Ewa Nowak"));
        let only_given =
            Profile::from_claims(&claims(json!({"given_name": "Ewa", "family_name": "  "})));
        assert_eq!(only_given.name.as_deref(), Some("Ewa"));
        // An empty name is still a name: the fallback is for a missing one.
        let empty = Profile::from_claims(&claims(json!({"name": "", "given_name": "Ewa"})));
        assert_eq!(empty.name, None);
    }

    #[test]
    fn amr_may_be_one_string() {
        assert_eq!(
            Profile::from_claims(&claims(json!({"amr": "mfa"}))).amr,
            "mfa"
        );
        assert_eq!(Profile::from_claims(&claims(json!({"amr": []}))).amr, "");
    }

    #[test]
    fn lengths_are_utf16_units_and_never_split_a_pair() {
        let long = "a".repeat(70);
        let profile = Profile::from_claims(&claims(
            json!({"name": long, "preferred_username": "x".repeat(64)}),
        ));
        assert_eq!(profile.name.unwrap().len(), 64);
        assert_eq!(profile.preferred_username.unwrap().len(), 64);
        // 63 units, then a pair that would end at 65: cut before the pair.
        let emoji = format!("{}😀", "a".repeat(63));
        let cut = Profile::from_claims(&claims(json!({"name": emoji})))
            .name
            .unwrap();
        assert_eq!(cut, "a".repeat(63));
        let fits = format!("{}😀", "a".repeat(62));
        assert_eq!(
            Profile::from_claims(&claims(json!({"name": fits.clone()})))
                .name
                .unwrap(),
            fits
        );
        // Trailing space left by the cut goes too.
        let spaced = format!("{} {}", "a".repeat(63), "b");
        assert_eq!(
            Profile::from_claims(&claims(json!({"name": spaced})))
                .name
                .unwrap(),
            "a".repeat(63)
        );
        let amr = vec!["m".repeat(100); 4];
        assert_eq!(
            Profile::from_claims(&claims(json!({"amr": amr}))).amr.len(),
            256
        );
        let time = Profile::from_claims(&claims(json!({"auth_time": "1".repeat(40)})))
            .auth_time
            .unwrap();
        assert_eq!(time.len(), 32);
    }

    #[test]
    fn addresses_must_be_plausible() {
        for good in [
            "a@b.test",
            " a@b.test ",
            "first.last+tag@sub.example.test",
            "ala@localhost",
        ] {
            assert!(address(Some(good)).is_some(), "{good}");
        }
        for bad in [
            "",
            "a",
            "@b.test",
            "a@",
            "a@@b",
            "a b@c.test",
            "Name <a@b.test>",
            "a@b.test, c@d.test",
            "a..b@c.test",
            "a@b..test",
            ".a@b.test",
            "a@.b",
            "\"a\"@b.test",
            &format!("{}@b.test", "a".repeat(65)),
            &format!("a@{}.test", "b".repeat(320)),
        ] {
            assert_eq!(address(Some(bad)), None, "{bad}");
        }
        assert_eq!(address(None), None);
        let trimmed = Profile::from_claims(&claims(json!({"email": " a@b.test "})));
        assert_eq!(trimmed.email.as_deref(), Some("a@b.test"));
    }

    #[test]
    fn userinfo_fills_what_the_token_did_not_have() {
        let mut profile = Profile::from_claims(&claims(json!({})));
        profile.apply_userinfo(&claims(json!({
            "name": "Jan Kowalski", "preferred_username": "jkowal",
            "email": "jan@example.test", "email_verified": true,
        })));
        assert_eq!(profile.name.as_deref(), Some("Jan Kowalski"));
        assert_eq!(profile.preferred_username.as_deref(), Some("jkowal"));
        assert_eq!(profile.verified_email(), Some("jan@example.test"));
    }

    #[test]
    fn userinfo_says_only_what_it_states_about_the_address() {
        let token = claims(json!({"email": "ewa@example.test", "email_verified": true}));

        // A repeat of the address that leaves the verification out keeps the token's word.
        let mut repeated = Profile::from_claims(&token);
        repeated.apply_userinfo(&claims(
            json!({"name": "Ewa Nowak", "email": "Ewa@example.test"}),
        ));
        assert!(repeated.email_verified);
        assert_eq!(repeated.email.as_deref(), Some("ewa@example.test"));
        assert_eq!(repeated.name.as_deref(), Some("Ewa Nowak"));

        // Another address is only as verified as userinfo says it is.
        let mut changed = Profile::from_claims(&token);
        changed.apply_userinfo(&claims(json!({"email": "ewa.work@example.test"})));
        assert_eq!(changed.email.as_deref(), Some("ewa.work@example.test"));
        assert!(!changed.email_verified);

        let mut verified = Profile::from_claims(&token);
        verified.apply_userinfo(&claims(
            json!({"email": "other@example.test", "email_verified": "True"}),
        ));
        assert!(verified.email_verified);

        // A stated "false" for the same address withdraws the verification.
        let mut withdrawn = Profile::from_claims(&token);
        withdrawn.apply_userinfo(&claims(
            json!({"email": "ewa@example.test", "email_verified": false}),
        ));
        assert!(!withdrawn.email_verified);

        // An address that is not one changes nothing.
        let mut garbage = Profile::from_claims(&token);
        garbage.apply_userinfo(&claims(
            json!({"email": "not an address", "email_verified": false}),
        ));
        assert!(garbage.email_verified);
        assert_eq!(garbage.email.as_deref(), Some("ewa@example.test"));
    }

    #[test]
    fn userinfo_ignores_values_that_are_not_text() {
        let mut profile = Profile::from_claims(&claims(json!({"name": "Ola"})));
        profile.apply_userinfo(&claims(
            json!({"name": 5, "preferred_username": ["x"], "email": {"a": 1}}),
        ));
        assert_eq!(profile.name.as_deref(), Some("Ola"));
        assert_eq!(profile.preferred_username, None);
        assert_eq!(profile.email, None);
    }
}
