//! Authentication: every step of the ceremony, with one thing wrong at a
//! time. Each answer is built complete and correctly signed, then changed in
//! one way, and the exact error is asserted.

mod support;

use stellarr_webauthn::cbor::CborError;
use stellarr_webauthn::{
    Algorithm, Assertion, AssertionRequest, AssertionResponse, Error, Registration,
    StoredCredential, b64, verify_assertion,
};
use support::*;

const ASSERTION_CHALLENGE: [u8; 32] = [0x3C; 32];

struct Fixture {
    algorithm: Algorithm,
    registration: Registration,
    sign_count: u32,
    allowed: Vec<Vec<u8>>,
    require_user_handle: bool,
    handle: Vec<u8>,
}

impl Fixture {
    fn new(algorithm: Algorithm) -> Fixture {
        let registration = enroll(algorithm);
        Fixture {
            algorithm,
            registration,
            sign_count: 0,
            allowed: vec![],
            require_user_handle: false,
            handle: USER_HANDLE.to_vec(),
        }
    }

    fn spec(&self) -> AssertionSpec {
        AssertionSpec::new(&self.registration.credential_id, &ASSERTION_CHALLENGE)
    }

    fn verify(&self, response: &AssertionResponse) -> Result<Assertion, Error> {
        let relying_party = relying_party();
        let allowed: Vec<&[u8]> = self.allowed.iter().map(Vec::as_slice).collect();
        let request = AssertionRequest {
            relying_party: &relying_party,
            challenge: &ASSERTION_CHALLENGE,
            allowed_credentials: &allowed,
            require_user_handle: self.require_user_handle,
        };
        let stored = StoredCredential {
            credential_id: &self.registration.credential_id,
            public_key: &self.registration.public_key,
            user_handle: &self.handle,
            sign_count: self.sign_count,
        };
        verify_assertion(&request, &stored, response)
    }

    fn accepts(&self, spec: &AssertionSpec) -> Assertion {
        self.verify(&spec.build(key(self.algorithm))).unwrap()
    }

    fn refuses(&self, spec: &AssertionSpec, expected: Error) {
        assert_eq!(
            self.verify(&spec.build(key(self.algorithm))).err(),
            Some(expected)
        );
    }
}

fn es256() -> Fixture {
    Fixture::new(Algorithm::Es256)
}

#[test]
fn every_offered_algorithm_signs_in() {
    for algorithm in Algorithm::OFFERED {
        let fixture = Fixture::new(algorithm);
        let result = fixture.accepts(&fixture.spec());
        assert_eq!(
            result,
            Assertion {
                sign_count: 1,
                backed_up: false,
                backup_eligible: false
            },
            "{algorithm:?}"
        );
    }
}

#[test]
fn the_new_counter_and_backup_state_are_reported() {
    let mut fixture = es256();
    fixture.sign_count = 4;
    let mut spec = fixture.spec();
    spec.sign_count = 77;
    spec.flags = UP | UV | BE | BS;
    assert_eq!(
        fixture.accepts(&spec),
        Assertion {
            sign_count: 77,
            backed_up: true,
            backup_eligible: true
        }
    );
    spec.flags = UP | UV | BE;
    assert_eq!(
        fixture.accepts(&spec),
        Assertion {
            sign_count: 77,
            backed_up: false,
            backup_eligible: true
        }
    );
}

#[test]
fn a_signature_from_another_key_is_refused() {
    let fixture = es256();
    let response = fixture.spec().build(other_key(Algorithm::Es256));
    assert_eq!(
        fixture.verify(&response).err(),
        Some(Error::SignatureInvalid)
    );
}

#[test]
fn a_signature_that_does_not_cover_the_data_is_refused() {
    let fixture = es256();
    // Signed over a different challenge, then sent with this one.
    let mut other = fixture.spec();
    other.client.challenge = vec![1; 32];
    let mut response = fixture.spec().build(key(Algorithm::Es256));
    response.response.signature = other.build(key(Algorithm::Es256)).response.signature;
    assert_eq!(
        fixture.verify(&response).err(),
        Some(Error::SignatureInvalid)
    );

    // Signed over other authenticator data (another counter).
    let mut other = fixture.spec();
    other.sign_count = 2;
    let mut response = fixture.spec().build(key(Algorithm::Es256));
    response.response.signature = other.build(key(Algorithm::Es256)).response.signature;
    assert_eq!(
        fixture.verify(&response).err(),
        Some(Error::SignatureInvalid)
    );
}

#[test]
fn damaged_signatures_are_refused() {
    let fixture = es256();
    let good = fixture.spec().build(key(Algorithm::Es256));
    let raw = b64::decode(&good.response.signature).unwrap();
    let mut flipped = raw.clone();
    let last = flipped.len() - 1;
    flipped[last] ^= 1;
    for bad in [
        vec![],
        raw[..raw.len() - 1].to_vec(),
        [raw.clone(), vec![0]].concat(),
        flipped,
        vec![0; 64],
    ] {
        let mut spec = fixture.spec();
        spec.signature = Some(bad);
        fixture.refuses(&spec, Error::SignatureInvalid);
    }
    let mut huge = fixture.spec();
    huge.signature = Some(vec![1; 2000]);
    fixture.refuses(&huge, Error::FieldTooLarge("signature"));
}

#[test]
fn a_signature_made_for_another_algorithm_is_refused() {
    // The stored key is ES256; an Ed25519 and an RSA signature must not pass.
    let fixture = es256();
    for algorithm in [Algorithm::EdDsa, Algorithm::Rs256, Algorithm::Es384] {
        let response = fixture.spec().build(key(algorithm));
        assert_eq!(
            fixture.verify(&response).err(),
            Some(Error::SignatureInvalid),
            "{algorithm:?}"
        );
    }
}

#[test]
fn the_type_is_checked() {
    let fixture = es256();
    let mut spec = fixture.spec();
    spec.client.kind = "webauthn.create".into();
    fixture.refuses(&spec, Error::ClientDataTypeMismatch);
}

#[test]
fn the_challenge_is_checked() {
    let fixture = es256();
    let mut spec = fixture.spec();
    spec.client.challenge = vec![0x3D; 32];
    fixture.refuses(&spec, Error::ChallengeMismatch);
    // A registration challenge reused for sign-in is another challenge.
    spec.client.challenge = CHALLENGE.to_vec();
    fixture.refuses(&spec, Error::ChallengeMismatch);
}

#[test]
fn the_origin_is_checked() {
    let fixture = es256();
    for origin in [
        "https://elsewhere.test",
        "http://chat.test",
        "https://chat.test:8443",
        "https://chat.test/",
    ] {
        let mut spec = fixture.spec();
        spec.client.origin = origin.into();
        fixture.refuses(&spec, Error::OriginNotAllowed);
    }
}

#[test]
fn cross_origin_ceremonies_are_refused() {
    let fixture = es256();
    let mut spec = fixture.spec();
    spec.client.cross_origin = Some(true);
    fixture.refuses(&spec, Error::CrossOriginNotAllowed);
    spec.client.cross_origin = Some(false);
    spec.client.top_origin = Some("https://chat.test".into());
    fixture.refuses(&spec, Error::CrossOriginNotAllowed);
}

#[test]
fn malformed_client_data_is_refused() {
    let fixture = es256();
    let mut spec = fixture.spec();
    spec.client.raw = Some(b"{".to_vec());
    fixture.refuses(&spec, Error::MalformedClientData);
}

#[test]
fn the_relying_party_id_hash_is_checked() {
    let fixture = es256();
    let mut spec = fixture.spec();
    spec.rp_id = "elsewhere.test".into();
    fixture.refuses(&spec, Error::RpIdHashMismatch);
}

#[test]
fn user_presence_and_verification_are_required() {
    let fixture = es256();
    let mut spec = fixture.spec();
    spec.flags = UV;
    fixture.refuses(&spec, Error::UserNotPresent);
    spec.flags = UP;
    fixture.refuses(&spec, Error::UserNotVerified);
    spec.flags = 0;
    fixture.refuses(&spec, Error::UserNotPresent);
}

#[test]
fn backup_flags_are_checked() {
    let fixture = es256();
    let mut spec = fixture.spec();
    spec.flags = UP | UV | BS;
    fixture.refuses(&spec, Error::BackupStateWithoutEligibility);

    // Eligibility reported differently from registration is not refused, and
    // is reported back for the application to keep or log.
    spec.flags = UP | UV;
    let seen = fixture.accepts(&spec);
    assert!(!seen.backup_eligible && !seen.backed_up);
    spec.flags = UP | UV | BE;
    assert!(fixture.accepts(&spec).backup_eligible);
}

#[test]
fn attested_data_in_an_assertion_is_refused() {
    let fixture = es256();
    let mut spec = fixture.spec();
    spec.flags = UP | UV | AT;
    let mut attested = vec![0u8; 16];
    attested.extend_from_slice(&(fixture.registration.credential_id.len() as u16).to_be_bytes());
    attested.extend_from_slice(&fixture.registration.credential_id);
    attested.extend_from_slice(&fixture.registration.public_key);
    spec.attested = Some(attested);
    fixture.refuses(&spec, Error::AttestedDataUnexpected);
}

#[test]
fn bytes_after_the_authenticator_data_are_refused() {
    let fixture = es256();
    let mut spec = fixture.spec();
    spec.trailing = vec![0];
    fixture.refuses(&spec, Error::AuthenticatorDataTrailingBytes);
    // An extension map without its flag.
    let mut spec = fixture.spec();
    spec.extensions = Some(map(&[(text("credProps"), map(&[]))]));
    fixture.refuses(&spec, Error::AuthenticatorDataTrailingBytes);
}

#[test]
fn extensions_with_their_flag_are_read_and_checked_for_the_end() {
    let fixture = es256();
    let mut spec = fixture.spec();
    spec.flags = UP | UV | ED;
    spec.extensions = Some(map(&[(
        text("credProps"),
        map(&[(text("rk"), vec![0xf5])]),
    )]));
    fixture.accepts(&spec);
    spec.trailing = vec![0];
    fixture.refuses(&spec, Error::AuthenticatorDataTrailingBytes);
    let mut cut = fixture.spec();
    cut.flags = UP | UV | ED;
    cut.extensions = Some(vec![0xa1, 0x61]);
    fixture.refuses(&cut, Error::Cbor(CborError::Truncated));
}

#[test]
fn authenticator_data_that_is_too_short_or_too_long_is_refused() {
    let fixture = es256();
    let mut response = fixture.spec().build(key(Algorithm::Es256));
    response.response.authenticator_data = b64::encode(&[0u8; 36]);
    assert_eq!(
        fixture.verify(&response).err(),
        Some(Error::MalformedAuthenticatorData)
    );
    response.response.authenticator_data = "A".repeat(30_000);
    assert_eq!(
        fixture.verify(&response).err(),
        Some(Error::FieldTooLarge("authenticatorData"))
    );
    response.response.authenticator_data = "*".into();
    assert_eq!(
        fixture.verify(&response).err(),
        Some(Error::BadBase64("authenticatorData"))
    );
}

#[test]
fn the_counter_must_increase_once_either_side_counts() {
    let mut fixture = es256();
    for (stored, received, ok) in [
        (0, 0, true),
        (0, 1, true),
        (5, 6, true),
        (5, u32::MAX, true),
        (5, 5, false),
        (5, 4, false),
        (5, 0, false),
        (u32::MAX, u32::MAX, false),
        (u32::MAX, 0, false),
    ] {
        fixture.sign_count = stored;
        let mut spec = fixture.spec();
        spec.sign_count = received;
        let result = fixture.verify(&spec.build(key(Algorithm::Es256)));
        if ok {
            assert_eq!(
                result.unwrap().sign_count,
                received,
                "{stored} -> {received}"
            );
        } else {
            assert_eq!(
                result.err(),
                Some(Error::CounterNotIncreased),
                "{stored} -> {received}"
            );
        }
    }
}

#[test]
fn the_counter_is_only_looked_at_after_the_signature() {
    let mut fixture = es256();
    fixture.sign_count = 9;
    let mut spec = fixture.spec();
    spec.sign_count = 1;
    let response = spec.build(other_key(Algorithm::Es256));
    assert_eq!(
        fixture.verify(&response).err(),
        Some(Error::SignatureInvalid)
    );
}

#[test]
fn the_answer_must_be_for_the_stored_credential() {
    let fixture = es256();
    let mut spec = fixture.spec();
    spec.raw_id = Some(vec![9; 32]);
    fixture.refuses(&spec, Error::CredentialIdMismatch);
}

#[test]
fn the_allowed_list_is_enforced_when_it_is_not_empty() {
    let mut fixture = es256();
    fixture.allowed = vec![vec![1; 32], vec![2; 16]];
    fixture.refuses(&fixture.spec(), Error::CredentialNotAllowed);
    fixture
        .allowed
        .push(fixture.registration.credential_id.clone());
    fixture.accepts(&fixture.spec());
    fixture.allowed.clear();
    fixture.accepts(&fixture.spec());
}

#[test]
fn the_user_handle_is_checked() {
    let mut fixture = es256();
    let mut spec = fixture.spec();

    // Not sent: fine for a second step, refused for a sign-in without a name.
    fixture.accepts(&spec);
    fixture.require_user_handle = true;
    fixture.refuses(&spec, Error::UserHandleMissing);

    spec.user_handle = Some(b64::encode(&USER_HANDLE));
    fixture.accepts(&spec);
    fixture.require_user_handle = false;
    fixture.accepts(&spec);

    // Sent but another account's, empty, or not a handle at all.
    spec.user_handle = Some(b64::encode(&[0x43; 16]));
    fixture.refuses(&spec, Error::UserHandleMismatch);
    spec.user_handle = Some(b64::encode(&USER_HANDLE[..15]));
    fixture.refuses(&spec, Error::UserHandleMismatch);
    spec.user_handle = Some(b64::encode(&[USER_HANDLE.to_vec(), vec![0]].concat()));
    fixture.refuses(&spec, Error::UserHandleMismatch);
    spec.user_handle = Some(String::new());
    fixture.refuses(&spec, Error::UserHandleEmpty);
    spec.user_handle = Some("!".into());
    fixture.refuses(&spec, Error::BadBase64("userHandle"));
    spec.user_handle = Some("A".repeat(200));
    fixture.refuses(&spec, Error::FieldTooLarge("userHandle"));
}

#[test]
fn the_envelope_is_checked() {
    let fixture = es256();
    let mut spec = fixture.spec();
    spec.kind = "password".into();
    fixture.refuses(&spec, Error::CredentialTypeNotPublicKey);
    let mut spec = fixture.spec();
    spec.id = Some(b64::encode(&[1, 2, 3]));
    fixture.refuses(&spec, Error::IdDoesNotMatchRawId);
}

#[test]
fn a_stored_key_that_cannot_be_read_is_an_error_not_a_pass() {
    let mut fixture = es256();
    let good = fixture.registration.public_key.clone();
    for bad in [
        vec![],
        vec![0xa0],
        [good.clone(), vec![0]].concat(),
        good[..good.len() - 1].to_vec(),
    ] {
        fixture.registration.public_key = bad;
        let spec = fixture.spec();
        assert_eq!(
            fixture.verify(&spec.build(key(Algorithm::Es256))).err(),
            Some(Error::StoredKeyInvalid)
        );
    }
}

#[test]
fn the_same_signed_answer_passes_again_only_while_the_counter_allows() {
    // Replay protection is the application's (the challenge is used once);
    // the counter is the second line for authenticators that count.
    let mut fixture = es256();
    let spec = fixture.spec();
    let response = spec.build(key(Algorithm::Es256));
    let first = fixture.verify(&response).unwrap();
    fixture.sign_count = first.sign_count;
    assert_eq!(
        fixture.verify(&response).err(),
        Some(Error::CounterNotIncreased)
    );
}
