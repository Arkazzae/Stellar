//! Hostile input never panics: every prefix and every single-byte change of
//! valid answers, and pseudo-random bytes, go through the verifiers.

mod support;

use stellarr_webauthn::cbor;
use stellarr_webauthn::cose::PublicKey;
use stellarr_webauthn::{
    Algorithm, AssertionRequest, StoredCredential, b64, verify_assertion, verify_registration,
};
use support::*;

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn changed_and_cut_registrations_never_panic() {
    let relying_party = relying_party();
    let response = RegistrationSpec::new(&CHALLENGE).build(key(Algorithm::Es256));
    let object = b64::decode(&response.response.attestation_object).unwrap();
    let client = b64::decode(&response.response.client_data_json).unwrap();
    for i in 0..object.len() {
        let mut cut = response.clone();
        cut.response.attestation_object = b64::encode(&object[..i]);
        assert!(verify_registration(&relying_party, &CHALLENGE, &cut).is_err());
        for flip in [0x01, 0x80, 0xff] {
            let mut changed = object.clone();
            changed[i] ^= flip;
            let mut r = response.clone();
            r.response.attestation_object = b64::encode(&changed);
            let _ = verify_registration(&relying_party, &CHALLENGE, &r);
        }
    }
    for i in 0..client.len() {
        let mut changed = client.clone();
        changed[i] ^= 0x01;
        let mut r = response.clone();
        r.response.client_data_json = b64::encode(&changed);
        let _ = verify_registration(&relying_party, &CHALLENGE, &r);
    }
}

#[test]
fn changed_and_cut_assertions_never_panic() {
    let registration = enroll(Algorithm::Es256);
    let relying_party = relying_party();
    let request = AssertionRequest {
        relying_party: &relying_party,
        challenge: &[0x3C; 32],
        allowed_credentials: &[],
        require_user_handle: false,
    };
    let stored = StoredCredential {
        credential_id: &registration.credential_id,
        public_key: &registration.public_key,
        user_handle: &USER_HANDLE,
        sign_count: 0,
        backup_eligible: false,
    };
    let response =
        AssertionSpec::new(&registration.credential_id, &[0x3C; 32]).build(key(Algorithm::Es256));
    assert!(verify_assertion(&request, &stored, &response).is_ok());
    let auth = b64::decode(&response.response.authenticator_data).unwrap();
    let signature = b64::decode(&response.response.signature).unwrap();
    for i in 0..auth.len() {
        let mut cut = response.clone();
        cut.response.authenticator_data = b64::encode(&auth[..i]);
        assert!(verify_assertion(&request, &stored, &cut).is_err());
        let mut changed = auth.clone();
        changed[i] ^= 0x01;
        let mut r = response.clone();
        r.response.authenticator_data = b64::encode(&changed);
        // Any change to what was signed is refused (the flags and counter
        // included), by the signature if by nothing earlier.
        assert!(verify_assertion(&request, &stored, &r).is_err(), "byte {i}");
    }
    for i in 0..signature.len() {
        let mut changed = signature.clone();
        changed[i] ^= 0x01;
        let mut r = response.clone();
        r.response.signature = b64::encode(&changed);
        assert!(
            verify_assertion(&request, &stored, &r).is_err(),
            "signature byte {i}"
        );
    }
}

#[test]
fn random_bytes_never_panic_the_readers() {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let seeds: Vec<Vec<u8>> = Algorithm::OFFERED
        .iter()
        .map(|a| key(*a).cose.clone())
        .collect();
    for round in 0..20_000 {
        let len = (xorshift(&mut state) % 300) as usize;
        let random: Vec<u8> = (0..len).map(|_| xorshift(&mut state) as u8).collect();
        let _ = cbor::decode_exact(&random);
        let _ = PublicKey::from_cose(&random);

        // A valid key with a few bytes replaced reaches the deeper checks.
        let mut near = seeds[round % seeds.len()].clone();
        for _ in 0..3 {
            let at = (xorshift(&mut state) as usize) % near.len();
            near[at] = xorshift(&mut state) as u8;
        }
        let _ = PublicKey::from_cose(&near);
    }
}
