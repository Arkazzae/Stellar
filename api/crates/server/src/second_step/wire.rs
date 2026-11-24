//! What a browser's passkey answer looks like on the wire.
//!
//! The answers of `navigator.credentials.create` and `.get` are JSON whose
//! binary members are base64url text. What is not shaped like that is a
//! malformed request (a 400, before anything is looked at): a member that is
//! not text, text that is not canonical unpadded base64url, a `type` that is
//! not `public-key`, a transport nobody knows, a member named twice. A member
//! that is `null`, left out or empty is let through, and the answer is refused
//! when it is checked (a 401 that spends an attempt).

use serde::de::{self, Deserializer};
use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;
use stellarr_webauthn::{
    AssertionBody, AssertionResponse, AttestationBody, RegistrationResponse, Transport, b64,
};

/// A request that cannot be read: a 400 with no body.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Malformed;

/// Canonical unpadded base64url text, kept as text.
struct Bytes(String);

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Bytes, D::Error> {
        let text = String::deserialize(deserializer)?;
        if b64::decode(&text).is_none() {
            return Err(de::Error::custom("not base64url"));
        }
        Ok(Bytes(text))
    }
}

/// The only kind of credential there is.
struct PublicKey(String);

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<PublicKey, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text != "public-key" {
            return Err(de::Error::custom("not a public key"));
        }
        Ok(PublicKey(text))
    }
}

/// A transport the library knows by name.
struct KnownTransport(String);

impl<'de> Deserialize<'de> for KnownTransport {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<KnownTransport, D::Error> {
        let text = String::deserialize(deserializer)?;
        if Transport::parse(&text).is_none() {
            return Err(de::Error::custom("not a transport"));
        }
        Ok(KnownTransport(text))
    }
}

/// The members every answer has, and the inner object where its own are.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<Inner> {
    id: Option<String>,
    raw_id: Option<Bytes>,
    #[serde(rename = "type")]
    kind: Option<PublicKey>,
    response: Option<Inner>,
    /// The results of extensions are an object when they are there.
    #[serde(rename = "clientExtensionResults")]
    _extensions: Option<serde_json::Map<String, Value>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssertionInner {
    authenticator_data: Option<Bytes>,
    #[serde(rename = "clientDataJSON")]
    client_data_json: Option<Bytes>,
    signature: Option<Bytes>,
    user_handle: Option<Bytes>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttestationInner {
    attestation_object: Option<Bytes>,
    #[serde(rename = "clientDataJSON")]
    client_data_json: Option<Bytes>,
    transports: Option<Vec<KnownTransport>>,
}

/// The raw text of the `response` member a request carried, which is read
/// when the route gets to it so that its refusal comes in the order it always
/// did.
pub(super) type Raw = Option<Box<RawValue>>;

fn read<Inner: for<'de> Deserialize<'de>>(
    raw: Option<&RawValue>,
) -> Result<Option<Envelope<Inner>>, Malformed> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    serde_json::from_str::<Option<Envelope<Inner>>>(raw.get()).map_err(|_| Malformed)
}

/// The answer of a sign-in or of a second step, or `None` for one that is
/// well formed but has what a check cannot do without missing (which is
/// refused as a wrong answer).
pub(super) fn assertion(raw: Option<&RawValue>) -> Result<Option<AssertionResponse>, Malformed> {
    let Some(envelope) = read::<AssertionInner>(raw)? else {
        return Ok(None);
    };
    let Some(inner) = envelope.response else {
        return Ok(None);
    };
    let (Some(raw_id), Some(kind), Some(authenticator_data), Some(client_data), Some(signature)) = (
        envelope.raw_id,
        envelope.kind,
        inner.authenticator_data,
        inner.client_data_json,
        inner.signature,
    ) else {
        return Ok(None);
    };
    Ok(Some(AssertionResponse {
        id: envelope.id.unwrap_or_default(),
        raw_id: raw_id.0,
        kind: kind.0,
        response: AssertionBody {
            authenticator_data: authenticator_data.0,
            client_data_json: client_data.0,
            signature: signature.0,
            user_handle: inner.user_handle.map(|handle| handle.0),
        },
    }))
}

/// The answer of adding a passkey, as [`assertion`] reads one.
pub(super) fn registration(
    raw: Option<&RawValue>,
) -> Result<Option<RegistrationResponse>, Malformed> {
    let Some(envelope) = read::<AttestationInner>(raw)? else {
        return Ok(None);
    };
    let Some(inner) = envelope.response else {
        return Ok(None);
    };
    let (Some(raw_id), Some(kind), Some(attestation_object), Some(client_data_json)) = (
        envelope.raw_id,
        envelope.kind,
        inner.attestation_object,
        inner.client_data_json,
    ) else {
        return Ok(None);
    };
    Ok(Some(RegistrationResponse {
        id: envelope.id.unwrap_or_default(),
        raw_id: raw_id.0,
        kind: kind.0,
        response: AttestationBody {
            attestation_object: attestation_object.0,
            client_data_json: client_data_json.0,
            transports: inner
                .transports
                .map(|names| names.into_iter().map(|name| name.0).collect()),
        },
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn raw(value: &Value) -> Box<RawValue> {
        RawValue::from_string(value.to_string()).unwrap()
    }

    fn assert_with(value: &Value) -> Result<Option<AssertionResponse>, Malformed> {
        assertion(Some(&raw(value)))
    }

    fn register_with(value: &Value) -> Result<Option<RegistrationResponse>, Malformed> {
        registration(Some(&raw(value)))
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

    #[test]
    fn a_whole_answer_is_read() {
        assert!(assert_with(&good_assertion()).unwrap().is_some());
        assert!(register_with(&good_registration()).unwrap().is_some());
    }

    #[test]
    fn an_answer_that_is_not_there_is_a_wrong_answer_and_not_a_bad_request() {
        assert!(matches!(assertion(None), Ok(None)));
        assert!(matches!(registration(None), Ok(None)));
        for value in [Value::Null, json!({}), json!({"id": "AAAA"})] {
            assert!(matches!(assert_with(&value), Ok(None)));
            assert!(matches!(register_with(&value), Ok(None)));
        }
        for member in [None, Some(Value::Null)] {
            let value = changed(good_assertion(), &["response"], member.clone());
            assert!(matches!(assert_with(&value), Ok(None)));
            let value = changed(good_registration(), &["response"], member);
            assert!(matches!(register_with(&value), Ok(None)));
        }
    }

    #[test]
    fn members_that_may_be_null_or_gone_make_an_answer_that_is_refused_later() {
        for (path, to) in [
            (&["rawId"][..], None),
            (&["rawId"], Some(Value::Null)),
            (&["type"], None),
            (&["type"], Some(Value::Null)),
        ] {
            let read = assert_with(&changed(good_assertion(), path, to.clone())).unwrap();
            assert!(read.is_none(), "{path:?} {to:?}");
        }
        // The id is not looked at on the way in: the answer is whole.
        for to in [None, Some(json!("")), Some(json!("!!"))] {
            let read = assert_with(&changed(good_assertion(), &["id"], to.clone())).unwrap();
            assert!(read.is_some(), "{to:?}");
        }
        for name in ["authenticatorData", "clientDataJSON", "signature"] {
            for to in [None, Some(Value::Null)] {
                let value = changed(good_assertion(), &["response", name], to);
                assert!(assert_with(&value).unwrap().is_none(), "{name}");
            }
            let value = changed(good_assertion(), &["response", name], Some(json!("")));
            assert!(assert_with(&value).unwrap().is_some(), "{name}");
        }
        for to in [None, Some(Value::Null), Some(json!(""))] {
            let value = changed(good_assertion(), &["response", "userHandle"], to);
            assert!(assert_with(&value).unwrap().is_some());
        }
        for name in ["attestationObject", "clientDataJSON"] {
            for to in [None, Some(Value::Null)] {
                let value = changed(good_registration(), &["response", name], to);
                assert!(register_with(&value).unwrap().is_none(), "{name}");
            }
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
            (&["type"], Some(json!(5))),
            (&["type"], Some(json!("nonsense"))),
            (&["type"], Some(json!("Public-Key"))),
            (&["response"], Some(json!([]))),
            (&["response"], Some(json!("x"))),
            (&["clientExtensionResults"], Some(json!("x"))),
        ] {
            assert!(
                assert_with(&changed(good_assertion(), path, to.clone())).is_err(),
                "assertion {path:?} {to:?}"
            );
            assert!(
                register_with(&changed(good_registration(), path, to.clone())).is_err(),
                "registration {path:?} {to:?}"
            );
        }
        for name in ["authenticatorData", "clientDataJSON", "signature", "userHandle"] {
            for to in [Some(json!(5)), Some(json!("!!"))] {
                let value = changed(good_assertion(), &["response", name], to.clone());
                assert!(assert_with(&value).is_err(), "{name} {to:?}");
            }
        }
        for name in ["attestationObject", "clientDataJSON"] {
            for to in [Some(json!(5)), Some(json!("!!"))] {
                let value = changed(good_registration(), &["response", name], to.clone());
                assert!(register_with(&value).is_err(), "{name} {to:?}");
            }
        }
        assert!(assert_with(&json!([])).is_err());
        assert!(register_with(&json!("x")).is_err());
    }

    #[test]
    fn a_member_named_twice_or_in_another_case_is_not_read_loosely() {
        let twice = RawValue::from_string(
            "{\"rawId\":\"AAAA\",\"rawId\":\"BBBB\",\"type\":\"public-key\"}".to_string(),
        )
        .unwrap();
        assert!(assertion(Some(&twice)).is_err());
        // `RawID` is a member nobody asked for, so the answer has no raw id.
        let cased = changed(good_assertion(), &["rawId"], None);
        let cased = changed(cased, &["RawId"], Some(json!("AAAA")));
        assert!(assert_with(&cased).unwrap().is_none());
    }

    #[test]
    fn a_registration_names_transports_the_library_knows() {
        let with = |transports: Value| {
            register_with(&changed(
                good_registration(),
                &["response", "transports"],
                Some(transports),
            ))
        };
        let read = with(json!(["usb", "smart-card"])).unwrap().unwrap();
        assert_eq!(read.response.transports, Some(vec!["usb".to_string(), "smart-card".to_string()]));
        assert!(with(Value::Null).unwrap().is_some());
        for wrong in [json!("x"), json!(["bogus"]), json!(["INTERNAL"]), json!(["cable"]), json!([null]), json!([7])] {
            assert!(with(wrong.clone()).is_err(), "{wrong}");
        }
    }
}
