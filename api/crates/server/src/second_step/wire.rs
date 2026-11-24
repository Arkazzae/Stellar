//! What a browser's passkey answer looks like on the wire, read as the first
//! server read it.
//!
//! The answers of `navigator.credentials.create` and `.get` are JSON whose
//! binary members are base64url text. The first server bound them with a
//! library whose rules decide what is a malformed request (a 400, before
//! anything is looked at) and what is a bad answer (a 401 that spends an
//! attempt): a member that is not text, text that is not canonical unpadded
//! base64url, a `type` that is not `public-key`, a member the library asks for
//! that is not there, all end in a 400; a member that is `null`, left out or
//! empty is let through and the answer is refused when it is checked. The rules
//! here were found by asking the first server, member by member.

use serde_json::{Map, Value};
use stellarr_webauthn::{
    AssertionBody, AssertionResponse, AttestationBody, RegistrationResponse, Transport, b64,
};

/// A request that cannot be read: a 400 with no body.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Malformed;

/// Text that may be left out or null, and is text when it is there.
fn text(object: &Map<String, Value>, name: &str) -> Result<Option<String>, Malformed> {
    match object.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(Malformed),
    }
}

/// Base64url that may be left out or null; when there it is canonical.
fn bytes(object: &Map<String, Value>, name: &str) -> Result<Option<String>, Malformed> {
    match text(object, name)? {
        Some(text) if b64::decode(&text).is_none() => Err(Malformed),
        other => Ok(other),
    }
}

/// [`bytes`], but a member the library asks for: it must be there, though it
/// may be null.
fn required_bytes(object: &Map<String, Value>, name: &str) -> Result<Option<String>, Malformed> {
    if !object.contains_key(name) {
        return Err(Malformed);
    }
    bytes(object, name)
}

/// `Some` for `public-key`; anything else that is text is not a kind there is,
/// and a number is taken for one that is not that.
fn kind(object: &Map<String, Value>) -> Result<Option<String>, Malformed> {
    match object.get("type") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text == "public-key" => Ok(Some(text.clone())),
        Some(Value::Number(number)) if number.is_i64() || number.is_u64() => Ok(None),
        Some(_) => Err(Malformed),
    }
}

/// The members every answer has, and the inner object where its own are.
struct Envelope<'a> {
    id: Option<String>,
    raw_id: Option<String>,
    kind: Option<String>,
    inner: Option<&'a Map<String, Value>>,
}

fn envelope(value: &Value) -> Result<Envelope<'_>, Malformed> {
    let Value::Object(object) = value else {
        return Err(Malformed);
    };
    // The results of extensions are an object when they are there.
    match object.get("clientExtensionResults") {
        None | Some(Value::Null | Value::Object(_)) => {}
        Some(_) => return Err(Malformed),
    }
    let inner = match object.get("response") {
        None | Some(Value::Null) => None,
        Some(Value::Object(inner)) => Some(inner),
        Some(_) => return Err(Malformed),
    };
    Ok(Envelope {
        id: text(object, "id")?,
        raw_id: bytes(object, "rawId")?,
        kind: kind(object)?,
        inner,
    })
}

/// The answer of a sign-in or of a second step, or `None` for one that is
/// well formed but has what a check cannot do without missing (which is
/// refused as a wrong answer).
pub(super) fn assertion(value: Option<&Value>) -> Result<Option<AssertionResponse>, Malformed> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let envelope = envelope(value)?;
    let Some(inner) = envelope.inner else {
        return Ok(None);
    };
    let authenticator_data = required_bytes(inner, "authenticatorData")?;
    let client_data = required_bytes(inner, "clientDataJSON")?;
    let signature = required_bytes(inner, "signature")?;
    let user_handle = bytes(inner, "userHandle")?;
    let (Some(raw_id), Some(kind), Some(authenticator_data), Some(client_data), Some(signature)) = (
        envelope.raw_id,
        envelope.kind,
        authenticator_data,
        client_data,
        signature,
    ) else {
        return Ok(None);
    };
    Ok(Some(AssertionResponse {
        id: envelope.id.unwrap_or_default(),
        raw_id,
        kind,
        response: AssertionBody {
            authenticator_data,
            client_data_json: client_data,
            signature,
            user_handle,
        },
    }))
}

/// The answer of adding a passkey, as [`assertion`] reads one.
pub(super) fn registration(
    value: Option<&Value>,
) -> Result<Option<RegistrationResponse>, Malformed> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let envelope = envelope(value)?;
    let Some(inner) = envelope.inner else {
        return Ok(None);
    };
    let attestation = required_bytes(inner, "attestationObject")?;
    let client_data = required_bytes(inner, "clientDataJSON")?;
    let transports = match inner.get("transports") {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) => {
            let mut known = Vec::new();
            for item in items {
                match item {
                    Value::String(name) if Transport::parse(name).is_some() => {
                        known.push(name.clone());
                    }
                    // A number names a transport the library knows by its
                    // value; none is kept.
                    Value::Number(number) if number.is_i64() || number.is_u64() => {}
                    _ => return Err(Malformed),
                }
            }
            Some(known)
        }
        Some(_) => return Err(Malformed),
    };
    let (Some(raw_id), Some(kind), Some(attestation_object), Some(client_data_json)) =
        (envelope.raw_id, envelope.kind, attestation, client_data)
    else {
        return Ok(None);
    };
    Ok(Some(RegistrationResponse {
        id: envelope.id.unwrap_or_default(),
        raw_id,
        kind,
        response: AttestationBody {
            attestation_object,
            client_data_json,
            transports,
        },
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn none<T>(result: Result<Option<T>, Malformed>) -> bool {
        matches!(result, Ok(None))
    }

    fn bad<T>(result: Result<Option<T>, Malformed>) -> bool {
        result.is_err()
    }

    fn good_assertion() -> Value {
        json!({"id": "AAAA", "rawId": "AAAA", "type": "public-key", "response": {
            "authenticatorData": "AAAA", "clientDataJSON": "AAAA", "signature": "AAAA", "userHandle": "AAAA"}})
    }

    fn good_registration() -> Value {
        json!({"id": "AAAA", "rawId": "AAAA", "type": "public-key", "response": {
            "attestationObject": "AAAA", "clientDataJSON": "AAAA"}})
    }

    /// Changes one member of an answer: `None` takes it out.
    fn changed(mut value: Value, path: &[&str], to: Option<Value>) -> Value {
        let mut at = &mut value;
        for name in &path[..path.len() - 1] {
            at = at.get_mut(*name).unwrap();
        }
        let object = at.as_object_mut().unwrap();
        match to {
            Some(to) => object.insert(path[path.len() - 1].to_string(), to),
            None => object.remove(path[path.len() - 1]),
        };
        value
    }

    // Every case below was asked of the first server.

    #[test]
    fn a_whole_answer_is_read() {
        assert!(assertion(Some(&good_assertion())).unwrap().is_some());
        assert!(registration(Some(&good_registration())).unwrap().is_some());
    }

    #[test]
    fn an_answer_that_is_not_there_is_a_wrong_answer_and_not_a_bad_request() {
        for value in [None, Some(Value::Null), Some(json!({})), Some(json!({"id": "AAAA"}))] {
            assert!(none(assertion(value.as_ref())));
            assert!(none(registration(value.as_ref())));
        }
        for member in [None, Some(Value::Null)] {
            let value = changed(good_assertion(), &["response"], member.clone());
            assert!(none(assertion(Some(&value))));
            let value = changed(good_registration(), &["response"], member);
            assert!(none(registration(Some(&value))));
        }
    }

    #[test]
    fn members_that_may_be_null_or_gone_make_an_answer_that_is_refused_later() {
        for (path, to) in [
            (&["rawId"][..], None),
            (&["rawId"], Some(Value::Null)),
            (&["type"], None),
            (&["type"], Some(Value::Null)),
            (&["type"], Some(json!(5))),
            (&["id"], None),
            (&["id"], Some(json!(""))),
            (&["id"], Some(json!("!!"))),
        ] {
            let kept_id = path == ["id"];
            let assertion = assertion(Some(&changed(good_assertion(), path, to.clone()))).unwrap();
            // The id is not looked at on the way in: the answer is whole.
            assert_eq!(assertion.is_some(), kept_id, "{path:?} {to:?}");
        }
        for name in ["authenticatorData", "clientDataJSON", "signature"] {
            let value = changed(good_assertion(), &["response", name], Some(Value::Null));
            assert!(none(assertion(Some(&value))), "{name}");
            let value = changed(good_assertion(), &["response", name], Some(json!("")));
            assert!(assertion(Some(&value)).unwrap().is_some(), "{name}");
        }
        for name in ["userHandle"] {
            for to in [None, Some(Value::Null), Some(json!(""))] {
                let value = changed(good_assertion(), &["response", name], to);
                assert!(assertion(Some(&value)).unwrap().is_some(), "{name}");
            }
        }
        for name in ["attestationObject", "clientDataJSON"] {
            let value = changed(good_registration(), &["response", name], Some(Value::Null));
            assert!(none(registration(Some(&value))), "{name}");
        }
    }

    #[test]
    fn what_cannot_be_read_is_a_bad_request() {
        for (path, to) in [
            (&["id"][..], Some(json!(5))),
            (&["rawId"], Some(json!(5))),
            (&["rawId"], Some(json!("!!"))),
            (&["rawId"], Some(json!("AAAA=="))),
            (&["rawId"], Some(json!("+/+/"))),
            (&["rawId"], Some(json!("ABSENT"))),
            (&["type"], Some(json!(""))),
            (&["type"], Some(json!("nonsense"))),
            (&["type"], Some(json!("Public-Key"))),
            (&["response"], Some(json!({}))),
            (&["response"], Some(json!([]))),
            (&["response"], Some(json!("x"))),
            (&["clientExtensionResults"], Some(json!("x"))),
        ] {
            assert!(
                bad(assertion(Some(&changed(good_assertion(), path, to.clone())))),
                "assertion {path:?} {to:?}"
            );
            assert!(
                bad(registration(Some(&changed(good_registration(), path, to.clone())))),
                "registration {path:?} {to:?}"
            );
        }
        for name in ["authenticatorData", "clientDataJSON", "signature"] {
            for to in [None, Some(json!(5)), Some(json!("!!"))] {
                let value = changed(good_assertion(), &["response", name], to.clone());
                assert!(bad(assertion(Some(&value))), "{name} {to:?}");
            }
        }
        for name in ["userHandle"] {
            for to in [Some(json!(5)), Some(json!("!!"))] {
                let value = changed(good_assertion(), &["response", name], to.clone());
                assert!(bad(assertion(Some(&value))), "{name} {to:?}");
            }
        }
        for name in ["attestationObject", "clientDataJSON"] {
            for to in [None, Some(json!(5)), Some(json!("!!"))] {
                let value = changed(good_registration(), &["response", name], to.clone());
                assert!(bad(registration(Some(&value))), "{name} {to:?}");
            }
        }
        assert!(bad(assertion(Some(&json!([])))));
        assert!(bad(registration(Some(&json!("x")))));
    }

    #[test]
    fn a_registration_names_transports_the_library_knows_or_numbers() {
        let with = |transports: Value| {
            registration(Some(&changed(
                good_registration(),
                &["response", "transports"],
                Some(transports),
            )))
        };
        let read = with(json!(["usb", "smart-card", 7])).unwrap().unwrap();
        assert_eq!(read.response.transports, Some(vec!["usb".to_string(), "smart-card".to_string()]));
        assert!(with(Value::Null).unwrap().is_some());
        for wrong in [json!("x"), json!(["bogus"]), json!(["INTERNAL"]), json!(["cable"]), json!([null])] {
            assert!(bad(with(wrong.clone())), "{wrong}");
        }
    }
}
