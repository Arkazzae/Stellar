//! Answers made by other implementations: Node's crypto (OpenSSL) for all ten
//! algorithms and the `openssl` command line tool for six, each with its own
//! hand-written CBOR. They are checked in; the scripts that wrote them are
//! next to them in `fixtures/`. A bug shared by this crate and aws-lc-rs
//! cannot make these pass.

mod support;

use serde::Deserialize;
use stellarr_webauthn::{
    Algorithm, AssertionRequest, AssertionResponse, Error, RegistrationResponse, RelyingParty,
    StoredCredential, b64, verify_assertion, verify_registration,
};

#[derive(Deserialize)]
struct Fixture {
    algorithm: String,
    cose_alg: i64,
    rp_id: String,
    origin: String,
    registration_challenge: String,
    registration: RegistrationResponse,
    stored: Stored,
    assertion_challenge: String,
    assertion: AssertionResponse,
}

#[derive(Deserialize)]
struct Stored {
    public_key: String,
    user_handle: String,
    sign_count: u32,
    backup_eligible: bool,
}

fn load(json: &str) -> Vec<Fixture> {
    serde_json::from_str(json).unwrap()
}

fn run(fixtures: &[Fixture], expected_algorithms: &[&str]) {
    let names: Vec<&str> = fixtures.iter().map(|f| f.algorithm.as_str()).collect();
    assert_eq!(names, expected_algorithms);
    for fixture in fixtures {
        let name = &fixture.algorithm;
        let relying_party =
            RelyingParty::new(&fixture.rp_id, "Stellarr", &[&fixture.origin]).unwrap();

        // Registration: the key stored is the one in the fixture, byte for byte.
        let challenge = b64::decode(&fixture.registration_challenge).unwrap();
        let registered = verify_registration(&relying_party, &challenge, &fixture.registration)
            .unwrap_or_else(|e| panic!("{name}: registration: {e}"));
        assert_eq!(registered.algorithm.cose_id(), fixture.cose_alg, "{name}");
        assert_eq!(
            b64::encode(&registered.public_key),
            fixture.stored.public_key,
            "{name}"
        );

        // Assertion against the stored key.
        let public_key = b64::decode(&fixture.stored.public_key).unwrap();
        let handle = b64::decode(&fixture.stored.user_handle).unwrap();
        let challenge = b64::decode(&fixture.assertion_challenge).unwrap();
        let stored = StoredCredential {
            credential_id: &registered.credential_id,
            public_key: &public_key,
            user_handle: &handle,
            sign_count: fixture.stored.sign_count,
            backup_eligible: fixture.stored.backup_eligible,
        };
        let request = AssertionRequest {
            relying_party: &relying_party,
            challenge: &challenge,
            allowed_credentials: &[],
            require_user_handle: true,
        };
        let verified = verify_assertion(&request, &stored, &fixture.assertion)
            .unwrap_or_else(|e| panic!("{name}: assertion: {e}"));
        assert!(verified.sign_count > fixture.stored.sign_count, "{name}");

        // One bit of the signature, one bit of the signed data: refused.
        let mut flipped = fixture.assertion.clone();
        let mut signature = b64::decode(&flipped.response.signature).unwrap();
        let last = signature.len() - 1;
        signature[last] ^= 0x01;
        flipped.response.signature = b64::encode(&signature);
        assert_eq!(
            verify_assertion(&request, &stored, &flipped).err(),
            Some(Error::SignatureInvalid),
            "{name}"
        );

        let mut other_data = fixture.assertion.clone();
        let mut auth = b64::decode(&other_data.response.authenticator_data).unwrap();
        auth[36] ^= 0x80;
        other_data.response.authenticator_data = b64::encode(&auth);
        assert_eq!(
            verify_assertion(&request, &stored, &other_data).err(),
            Some(Error::SignatureInvalid),
            "{name}"
        );

        // The registration signature too.
        let mut bad = fixture.registration.clone();
        let mut object = b64::decode(&bad.response.attestation_object).unwrap();
        let at = object.len() - 1;
        object[at] ^= 0x01;
        bad.response.attestation_object = b64::encode(&object);
        let challenge = b64::decode(&fixture.registration_challenge).unwrap();
        assert!(
            verify_registration(&relying_party, &challenge, &bad).is_err(),
            "{name}"
        );
    }
}

#[test]
fn node_crypto_signatures_verify_for_every_offered_algorithm() {
    let fixtures = load(include_str!("fixtures/node_fixtures.json"));
    // Every offered algorithm has a fixture, none is left out or invented.
    let mut ids: Vec<i64> = fixtures.iter().map(|f| f.cose_alg).collect();
    let mut offered: Vec<i64> = Algorithm::OFFERED.iter().map(|a| a.cose_id()).collect();
    ids.sort_unstable();
    offered.sort_unstable();
    assert_eq!(ids, offered);
    run(
        &fixtures,
        &[
            "EdDSA", "ES256", "ES384", "ES512", "RS256", "RS384", "RS512", "PS256", "PS384",
            "PS512",
        ],
    );
}

#[test]
fn openssl_command_line_signatures_verify() {
    let fixtures = load(include_str!("fixtures/openssl_fixtures.json"));
    run(
        &fixtures,
        &["ES256", "ES384", "ES512", "EdDSA", "RS256", "PS256"],
    );
}
