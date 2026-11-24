//! ID token and key set checks against tokens signed by Node's crypto (see
//! fixtures/generate.mjs): the code under test never signed any of them.

use serde_json::Value;
use stellarr_oidc::idtoken::{CLOCK_SKEW, Expect, IdTokenError as E, verify};
use stellarr_oidc::jwks::{self, JwksError, KeySet};

struct Fixtures(Value);

impl Fixtures {
    fn load() -> Self {
        Self(serde_json::from_str(include_str!("fixtures/fixtures.json")).unwrap())
    }

    fn now(&self) -> i64 {
        self.0["now"].as_i64().unwrap()
    }

    fn token(&self, name: &str) -> &str {
        self.0["tokens"][name]
            .as_str()
            .unwrap_or_else(|| panic!("no token {name}"))
    }

    fn keys(&self, name: &str) -> KeySet {
        jwks::parse(&serde_json::to_vec(&self.0[name]).unwrap()).unwrap()
    }

    fn expect(&self) -> Expect<'_> {
        Expect {
            issuer: self.0["issuer"].as_str().unwrap(),
            client_id: self.0["client_id"].as_str().unwrap(),
            nonce: self.0["nonce"].as_str().unwrap(),
            now: self.now(),
            fresh_within: None,
            access_token: None,
        }
    }
}

fn check(f: &Fixtures, name: &str) -> Result<(), E> {
    verify(f.token(name), &f.keys("jwks"), &f.expect()).map(|_| ())
}

#[test]
fn accepts_every_supported_algorithm_signed_by_the_providers_key() {
    let f = Fixtures::load();
    for name in [
        "rs256",
        "rs384",
        "rs512",
        "ps256",
        "ps384",
        "ps512",
        "es256",
        "es384",
        "es512",
        // Without a key id the keys that fit the algorithm are tried.
        "no_kid",
        "no_kid_es256",
    ] {
        assert_eq!(check(&f, name), Ok(()), "{name}");
    }
}

#[test]
fn accepts_the_edges_of_the_allowed_times_and_the_claims_it_does_not_require() {
    let f = Fixtures::load();
    for name in [
        "expired_edge_ok",
        "not_yet_valid_edge_ok",
        "expiry_fraction",
        "audience_list_no_azp",
        "empty_azp",
        "audience_list_empty_azp",
        "unknown_kid_right_key",
        "no_at_hash",
        "padded_subject",
        "long_subject_ok",
        "audience_list",
        "single_audience_right_azp",
        "stale_auth_time",
    ] {
        assert_eq!(check(&f, name), Ok(()), "{name}");
    }
}

#[test]
fn returns_the_claims_the_application_reads() {
    let f = Fixtures::load();
    let token = verify(f.token("padded_subject"), &f.keys("jwks"), &f.expect()).unwrap();
    assert_eq!(token.subject, "subject-1");
    assert_eq!(token.issuer, "https://identity.test");
    assert_eq!(token.stored_issuer(), "https://identity.test");
    assert_eq!(token.amr(), ["pwd", "mfa"]);
    assert_eq!(token.auth_time().unwrap(), (f.now() - 30).to_string());
    let profile = stellarr_oidc::profile::Profile::from_claims(&token.claims);
    assert_eq!(profile.verified_email(), Some("ola@example.test"));
    assert_eq!(profile.name.as_deref(), Some("Ola Łęcka"));
}

#[test]
fn refuses_a_token_the_provider_did_not_sign() {
    let f = Fixtures::load();
    for (name, expected) in [
        // Signed with another key that claims the provider's key id.
        ("other_key_same_kid", E::BadSignature),
        ("no_kid_other_key", E::BadSignature),
        // Payload changed after signing, and signatures cut or emptied.
        ("tampered_payload", E::BadSignature),
        ("empty_signature", E::BadSignature),
        // A key the provider never published.
        ("other_key_own_kid", E::UnknownKeyId),
        ("weak_key", E::UnknownKeyId),
        // A key of the wrong kind for the algorithm the header names.
        ("rsa_alg_on_ec_key", E::NoUsableKey),
        ("es256_on_rsa_key", E::NoUsableKey),
    ] {
        assert_eq!(check(&f, name), Err(expected), "{name}");
    }
    assert!(matches!(
        check(&f, "truncated_signature"),
        Err(E::Malformed | E::BadSignature)
    ));
}

#[test]
fn refuses_a_token_without_a_signature_algorithm_or_with_the_wrong_kind() {
    let f = Fixtures::load();
    for (name, expected) in [
        ("alg_none", E::Unsigned),
        ("alg_none_with_signature", E::Unsigned),
        ("alg_none_upper", E::Unsigned),
        ("alg_missing", E::MissingAlgorithm),
        ("alg_not_text", E::MissingAlgorithm),
        // Symmetric: the public key as the MAC secret, the client secret, and no key id.
        ("hs256_with_public_key", E::UnsupportedAlgorithm),
        ("hs256_with_client_secret", E::UnsupportedAlgorithm),
        ("hs256_no_kid", E::UnsupportedAlgorithm),
        ("crit_header", E::CriticalHeader),
    ] {
        assert_eq!(check(&f, name), Err(expected), "{name}");
    }
}

#[test]
fn a_key_that_names_its_algorithm_is_used_for_it_alone() {
    let f = Fixtures::load();
    let pinned = f.keys("jwks_pinned");
    assert_eq!(
        verify(f.token("rs256"), &pinned, &f.expect()).map(|_| ()),
        Ok(())
    );
    assert_eq!(
        verify(f.token("ps256"), &pinned, &f.expect()).map(|_| ()),
        Err(E::NoUsableKey)
    );
}

#[test]
fn refuses_wrong_issuer_audience_and_authorized_party() {
    let f = Fixtures::load();
    for (name, expected) in [
        ("wrong_issuer", E::WrongIssuer),
        // Compared exactly with the discovery document's, slash included.
        ("issuer_with_slash", E::WrongIssuer),
        ("no_issuer", E::WrongIssuer),
        ("wrong_audience", E::WrongAudience),
        ("no_audience", E::WrongAudience),
        ("audience_not_text", E::WrongAudience),
        ("audience_list_wrong_azp", E::WrongAuthorizedParty),
        ("single_audience_wrong_azp", E::WrongAuthorizedParty),
    ] {
        assert_eq!(check(&f, name), Err(expected), "{name}");
    }
}

#[test]
fn refuses_the_wrong_nonce() {
    let f = Fixtures::load();
    for (name, expected) in [
        ("wrong_nonce", E::WrongNonce),
        ("no_nonce", E::MissingNonce),
        ("nonce_not_text", E::MissingNonce),
    ] {
        assert_eq!(check(&f, name), Err(expected), "{name}");
    }
    // The same token under another sign-in's nonce.
    let mut other = f.expect();
    other.nonce = "n-other";
    assert_eq!(
        verify(f.token("rs256"), &f.keys("jwks"), &other).map(|_| ()),
        Err(E::WrongNonce)
    );
}

#[test]
fn refuses_an_expired_token_and_one_not_yet_valid() {
    let f = Fixtures::load();
    for (name, expected) in [
        ("expired", E::Expired),
        ("no_expiry", E::MissingExpiry),
        ("no_issued_at", E::MissingIssuedAt),
        // A time of the wrong type is a malformed token, not a late one.
        ("expiry_not_a_number", E::Malformed),
        ("exp_null", E::Malformed),
        ("nbf_text", E::Malformed),
        ("iat_text", E::Malformed),
        ("auth_time_text", E::Malformed),
        ("auth_time_bool", E::Malformed),
        ("not_yet_valid", E::NotYetValid),
        ("not_before_after_expiry", E::InvalidLifetime),
    ] {
        assert_eq!(check(&f, name), Err(expected), "{name}");
    }
    // The clock decides, and the skew is exact: one second either side of it flips the answer.
    let mut late = f.expect();
    late.now = f.now() + 300 + CLOCK_SKEW + 1;
    assert_eq!(
        verify(f.token("rs256"), &f.keys("jwks"), &late).map(|_| ()),
        Err(E::Expired)
    );
    late.now -= 1;
    assert_eq!(
        verify(f.token("rs256"), &f.keys("jwks"), &late).map(|_| ()),
        Ok(())
    );
    let mut early = f.expect();
    early.now = f.now();
    assert_eq!(
        verify(f.token("not_yet_valid"), &f.keys("jwks"), &early).map(|_| ()),
        Err(E::NotYetValid)
    );
    early.now += 1;
    assert_eq!(
        verify(f.token("not_yet_valid"), &f.keys("jwks"), &early).map(|_| ()),
        Ok(())
    );
}

#[test]
fn refuses_an_identity_the_application_cannot_store() {
    let f = Fixtures::load();
    for name in [
        "no_subject",
        "empty_subject",
        "long_subject",
        "subject_not_text",
    ] {
        assert_eq!(check(&f, name), Err(E::InvalidIdentity), "{name}");
    }
}

#[test]
fn a_demanded_fresh_sign_in_needs_a_recent_auth_time() {
    let f = Fixtures::load();
    let mut demand = f.expect();
    demand.fresh_within = Some(300);
    let check = |name: &str| verify(f.token(name), &f.keys("jwks"), &demand).map(|_| ());
    assert_eq!(check("rs256"), Ok(()));
    assert_eq!(check("auth_time_edge_ok"), Ok(()));
    for name in [
        "stale_auth_time",
        "auth_time_just_stale",
        "future_auth_time",
        "no_auth_time",
    ] {
        assert_eq!(check(name), Err(E::NotFreshSignIn), "{name}");
    }
}

#[test]
fn at_hash_must_match_the_access_token_when_the_token_has_one() {
    let f = Fixtures::load();
    let mut with_access = f.expect();
    with_access.access_token = Some(f.0["access_token"].as_str().unwrap());
    let run = |name: &str, e: &Expect<'_>| verify(f.token(name), &f.keys("jwks"), e).map(|_| ());
    assert_eq!(run("rs256", &with_access), Ok(()));
    assert_eq!(run("no_at_hash", &with_access), Ok(()));
    assert_eq!(
        run("wrong_at_hash", &with_access),
        Err(E::AccessTokenHashMismatch)
    );
    with_access.access_token = Some("another-access-token");
    assert_eq!(run("rs256", &with_access), Err(E::AccessTokenHashMismatch));
    // No access token to compare with: nothing to check.
    assert_eq!(run("wrong_at_hash", &f.expect()), Ok(()));
}

#[test]
fn an_unknown_key_id_is_a_distinct_outcome_that_a_refetched_key_set_can_answer() {
    let f = Fixtures::load();
    let token = f.token("other_key_own_kid");
    assert_eq!(
        verify(token, &f.keys("jwks"), &f.expect()).map(|_| ()),
        Err(E::UnknownKeyId)
    );
    // The provider rotated its keys: the set fetched again has the new one.
    assert_eq!(
        verify(token, &f.keys("jwks_rotated"), &f.expect()).map(|_| ()),
        Ok(())
    );
    // A rotated set still refuses what the old key did not sign.
    assert_eq!(
        verify(
            f.token("other_key_same_kid"),
            &f.keys("jwks_rotated"),
            &f.expect()
        )
        .map(|_| ()),
        Err(E::BadSignature)
    );
    // And a set that holds the key id but another key does not sign for it.
    assert_eq!(
        verify(f.token("rs256"), &f.keys("jwks_other"), &f.expect()).map(|_| ()),
        Err(E::BadSignature)
    );
}

#[test]
fn refuses_malformed_and_oversized_tokens() {
    let f = Fixtures::load();
    let good = f.token("rs256");
    let parts: Vec<&str> = good.split('.').collect();
    let keys = f.keys("jwks");
    let run = |token: &str| verify(token, &keys, &f.expect()).map(|_| ());
    let (h, p, sig) = (parts[0], parts[1], parts[2]);
    for (token, expected) in [
        (String::new(), E::Malformed),
        (".".into(), E::Malformed),
        ("..".into(), E::Malformed),
        ("a.b".into(), E::Malformed),
        ("a.b.c.d".into(), E::Malformed),
        (format!("{h}.{p}"), E::Malformed),
        (format!("{good}.extra"), E::Malformed),
        // A header that is not base64url, not JSON, or not an object.
        (format!("!!.{p}.{sig}"), E::Malformed),
        (format!("{}.{p}.{sig}", b64("[]")), E::Malformed),
        (format!("{}.{p}.{sig}", b64("null")), E::Malformed),
        (format!("{}.{p}.{sig}", b64("{")), E::Malformed),
        (format!("{}.{p}.{sig}", b64("\u{0}\u{1}")), E::Malformed),
        // A signature that is not base64url.
        (format!("{h}.{p}.!!"), E::Malformed),
        // A payload changed after signing fails the signature, whatever it holds.
        (format!("{h}.!!.{sig}"), E::BadSignature),
        (format!("{h}.{}.{sig}", b64("[1]")), E::BadSignature),
        (format!("{h}.{}.{sig}", b64("{")), E::BadSignature),
    ] {
        assert_eq!(run(&token), Err(expected), "{token:.60}");
    }
    // A payload that is signed but not an object.
    for name in ["payload_array", "payload_string"] {
        assert_eq!(check(&f, name), Err(E::Malformed), "{name}");
    }
    assert_eq!(run(&"a".repeat(64 * 1024 + 1)), Err(E::TooLarge));
    let big_header = format!("{}.{}.{}", "a".repeat(4 * 1024 + 1), parts[1], parts[2]);
    assert_eq!(run(&big_header), Err(E::TooLarge));
}

fn b64(text: &str) -> String {
    stellarr_oidc::b64::encode(text.as_bytes())
}

#[test]
fn a_weak_rsa_key_is_not_used() {
    let f = Fixtures::load();
    let keys = f.keys("jwks_weak");
    assert_eq!(keys.keys().len(), 1);
    assert_eq!(keys.skipped(), 1);
    assert_eq!(
        verify(f.token("weak_key"), &keys, &f.expect()).map(|_| ()),
        Err(E::UnknownKeyId)
    );
    let only_weak = serde_json::json!({"keys": [f.0["jwks_weak"]["keys"][0].clone()]});
    assert_eq!(
        jwks::parse(&serde_json::to_vec(&only_weak).unwrap()).map(|_| ()),
        Err(JwksError::NoUsableKeys)
    );
}

#[test]
fn key_set_skips_what_cannot_verify_and_refuses_what_is_unreadable() {
    let f = Fixtures::load();
    let rsa = f.0["rsa1_jwk"].clone();
    let ec = f.0["p256_jwk"].clone();
    let edit = |base: &Value, key: &str, value: Value| {
        let mut copy = base.clone();
        if value.is_null() {
            copy.as_object_mut().unwrap().remove(key);
        } else {
            copy[key] = value;
        }
        copy
    };
    let unusable = [
        edit(&rsa, "use", "enc".into()),
        edit(&rsa, "use", 1.into()),
        edit(&rsa, "key_ops", serde_json::json!(["encrypt"])),
        edit(&rsa, "key_ops", "verify".into()),
        edit(&rsa, "kty", "oct".into()),
        edit(&rsa, "kty", Value::Null),
        edit(&rsa, "n", "!!".into()),
        edit(&rsa, "n", Value::Null),
        edit(&rsa, "n", "AQAB".into()),
        edit(&rsa, "e", "".into()),
        edit(&rsa, "e", "AAAAAAAAAAAAAAAA".into()),
        edit(&rsa, "alg", "HS256".into()),
        edit(&rsa, "alg", "ES256".into()),
        edit(&rsa, "alg", 5.into()),
        edit(&rsa, "kid", 5.into()),
        edit(&rsa, "kid", "k".repeat(257).into()),
        edit(&ec, "crv", "P-192".into()),
        edit(&ec, "crv", "secp256k1".into()),
        edit(&ec, "x", "AQ".into()),
        edit(&ec, "y", Value::Null),
        edit(&ec, "alg", "ES384".into()),
        edit(&ec, "alg", "RS256".into()),
        serde_json::json!({"kty": "OKP", "crv": "Ed25519", "x": "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo"}),
        serde_json::json!("a string"),
        serde_json::json!(null),
        serde_json::json!([]),
    ];
    for entry in &unusable {
        let doc = serde_json::json!({ "keys": [entry.clone()] });
        assert_eq!(
            jwks::parse(&serde_json::to_vec(&doc).unwrap()).map(|_| ()),
            Err(JwksError::NoUsableKeys),
            "{entry}"
        );
        // Beside a good key it is left out and the good one still works.
        let doc = serde_json::json!({ "keys": [entry.clone(), rsa.clone()] });
        let set = jwks::parse(&serde_json::to_vec(&doc).unwrap()).unwrap();
        assert_eq!((set.keys().len(), set.skipped()), (1, 1), "{entry}");
    }
    // Leading zeros in the modulus change nothing: the key still verifies.
    let mut zeros = vec![0u8, 0u8];
    zeros.extend(stellarr_oidc::b64::decode(rsa["n"].as_str().unwrap()).unwrap());
    let padded = edit(&rsa, "n", stellarr_oidc::b64::encode(&zeros).into());
    let set = jwks::parse(&serde_json::to_vec(&serde_json::json!({ "keys": [padded] })).unwrap())
        .unwrap();
    assert_eq!(
        verify(f.token("rs256"), &set, &f.expect()).map(|_| ()),
        Ok(())
    );
    for body in [
        &b""[..],
        b"[]",
        b"null",
        b"{",
        b"{}",
        br#"{"keys": 5}"#,
        br#"{"keys": {}}"#,
        b"\xff",
    ] {
        assert_eq!(
            jwks::parse(body).map(|_| ()),
            Err(JwksError::NotJson),
            "{body:?}"
        );
    }
    assert_eq!(
        jwks::parse(br#"{"keys": []}"#).map(|_| ()),
        Err(JwksError::NoUsableKeys)
    );
    let many = serde_json::json!({ "keys": vec![rsa.clone(); 65] });
    assert_eq!(
        jwks::parse(&serde_json::to_vec(&many).unwrap()).map(|_| ()),
        Err(JwksError::TooManyKeys)
    );
    let huge = vec![b' '; stellarr_oidc::MAX_DOCUMENT + 1];
    assert_eq!(jwks::parse(&huge).map(|_| ()), Err(JwksError::TooLarge));
}

#[test]
fn a_key_set_may_hold_several_keys_under_one_id() {
    let f = Fixtures::load();
    // Two keys under rsa-1: the token is good when any of them verifies.
    let mut doc = f.0["jwks_other"].clone();
    doc["keys"]
        .as_array_mut()
        .unwrap()
        .push(f.0["rsa1_jwk"].clone());
    let set = jwks::parse(&serde_json::to_vec(&doc).unwrap()).unwrap();
    assert_eq!(set.keys().len(), 2);
    assert_eq!(
        verify(f.token("rs256"), &set, &f.expect()).map(|_| ()),
        Ok(())
    );
    assert_eq!(
        verify(f.token("other_key_same_kid"), &set, &f.expect()).map(|_| ()),
        Ok(())
    );
    assert_eq!(
        verify(f.token("tampered_payload"), &set, &f.expect()).map(|_| ()),
        Err(E::BadSignature)
    );
}

#[test]
fn at_hash_uses_the_hash_of_the_tokens_own_algorithm_family() {
    let f = Fixtures::load();
    let mut with_access = f.expect();
    with_access.access_token = Some(f.0["access_token"].as_str().unwrap());
    let run = |name: &str| verify(f.token(name), &f.keys("jwks"), &with_access).map(|_| ());
    for name in ["at_rs384", "at_rs512", "at_ps512", "at_es384", "at_es512"] {
        assert_eq!(run(name), Ok(()), "{name}");
    }
    // Each with the hash of another family, as the base token's SHA-256 is.
    for name in [
        "at_rs384_wrong",
        "at_rs512_wrong",
        "at_es384_wrong",
        "at_es512_wrong",
        "rs384",
        "es512",
    ] {
        assert_eq!(run(name), Err(E::AccessTokenHashMismatch), "{name}");
    }
}

#[test]
fn a_key_id_must_be_text() {
    let f = Fixtures::load();
    for name in ["kid_number", "kid_null"] {
        assert_eq!(check(&f, name), Err(E::Malformed), "{name}");
    }
}

#[test]
fn a_key_id_nobody_has_falls_back_to_every_fitting_key_and_then_asks_for_a_refetch() {
    let f = Fixtures::load();
    // Signed by a published key under a name the set does not know: accepted.
    assert_eq!(check(&f, "unknown_kid_right_key"), Ok(()));
    // Signed by nobody the set knows: the set may be stale.
    assert_eq!(check(&f, "other_key_own_kid"), Err(E::UnknownKeyId));
    // Where the key id matches a key, there is no fallback to the others.
    let mut doc = f.0["jwks"].clone();
    doc["keys"]
        .as_array_mut()
        .unwrap()
        .push(f.0["rsa2_jwk"].clone());
    let set = jwks::parse(&serde_json::to_vec(&doc).unwrap()).unwrap();
    assert_eq!(
        verify(f.token("other_key_same_kid"), &set, &f.expect()).map(|_| ()),
        Err(E::BadSignature)
    );
    assert_eq!(
        verify(f.token("other_key_own_kid"), &set, &f.expect()).map(|_| ()),
        Ok(())
    );
}
