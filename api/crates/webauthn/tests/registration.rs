//! Registration: every step of the ceremony, with one thing wrong at a time.
//! Each refusal is asserted by its exact error, so a test cannot pass because
//! some other check happened to catch the answer first.

mod support;

use stellarr_webauthn::cbor::CborError;
use stellarr_webauthn::{Algorithm, Error, RegistrationResponse, Transport, verify_registration};
use support::*;

fn verify(response: &RegistrationResponse) -> Result<stellarr_webauthn::Registration, Error> {
    verify_registration(&relying_party(), &CHALLENGE, response)
}

fn spec() -> RegistrationSpec {
    RegistrationSpec::new(&CHALLENGE)
}

fn refused(spec: &RegistrationSpec, expected: Error) {
    let result = verify(&spec.build(key(Algorithm::Es256)));
    assert_eq!(result.err(), Some(expected));
}

#[test]
fn every_offered_algorithm_registers_with_packed_self_attestation() {
    for algorithm in Algorithm::OFFERED {
        let key = key(algorithm);
        let registered =
            verify(&spec().build(key)).unwrap_or_else(|e| panic!("{algorithm:?}: {e}"));
        assert_eq!(registered.algorithm, algorithm);
        assert_eq!(registered.public_key, key.cose);
        assert_eq!(registered.credential_id, spec().credential_id);
    }
}

#[test]
fn every_offered_algorithm_registers_with_no_attestation() {
    for algorithm in Algorithm::OFFERED {
        let mut spec = spec();
        spec.statement = Statement::None;
        assert!(verify(&spec.build(key(algorithm))).is_ok(), "{algorithm:?}");
    }
}

#[test]
fn what_the_application_stores_is_reported() {
    let mut spec = spec();
    spec.flags = UP | UV | AT | BE | BS;
    spec.sign_count = 9;
    spec.aaguid = *b"0123456789abcdef";
    spec.transports = Some(vec!["usb".into(), "hybrid".into(), "usb".into()]);
    let registered = verify(&spec.build(key(Algorithm::Es256))).unwrap();
    assert_eq!(registered.sign_count, 9);
    assert!(registered.backup_eligible);
    assert!(registered.backed_up);
    assert_eq!(registered.aaguid, *b"0123456789abcdef");
    assert_eq!(registered.transports, [Transport::Usb, Transport::Hybrid]);

    let mut plain = spec.clone();
    plain.flags = UP | UV | AT;
    plain.transports = None;
    let registered = verify(&plain.build(key(Algorithm::Es256))).unwrap();
    assert!(!registered.backup_eligible && !registered.backed_up);
    assert!(registered.transports.is_empty());
}

#[test]
fn the_type_is_checked() {
    let mut spec = spec();
    spec.client.kind = "webauthn.get".into();
    refused(&spec, Error::ClientDataTypeMismatch);
    spec.client.kind = "webauthn.create ".into();
    refused(&spec, Error::ClientDataTypeMismatch);
}

#[test]
fn the_challenge_is_checked() {
    let mut other = spec();
    other.client.challenge = vec![0xA5; 31];
    refused(&other, Error::ChallengeMismatch);
    other.client.challenge = [vec![0xA5; 31], vec![0xA4]].concat();
    refused(&other, Error::ChallengeMismatch);
    other.client.challenge = vec![];
    refused(&other, Error::ChallengeMismatch);
    other.client.challenge = [CHALLENGE.to_vec(), vec![0]].concat();
    refused(&other, Error::ChallengeMismatch);

    // A challenge that is not canonical base64url never matches.
    let mut raw = spec();
    raw.client.raw = Some(
        br#"{"type":"webauthn.create","challenge":"paWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaU=","origin":"https://chat.test"}"#.to_vec(),
    );
    refused(&raw, Error::ChallengeMismatch);

    // An empty challenge on both sides is not a match.
    let mut empty = spec();
    empty.client.challenge = vec![];
    let response = empty.build(key(Algorithm::Es256));
    assert_eq!(
        verify_registration(&relying_party(), &[], &response).err(),
        Some(Error::ChallengeMismatch)
    );

    // And an empty expectation is refused whatever the answer says.
    let response = spec().build(key(Algorithm::Es256));
    assert_eq!(
        verify_registration(&relying_party(), &[], &response).err(),
        Some(Error::ChallengeMismatch)
    );
    assert_eq!(
        verify_registration(&relying_party(), &[0xA4; 32], &response).err(),
        Some(Error::ChallengeMismatch)
    );
}

#[test]
fn the_origin_is_checked() {
    for origin in [
        "https://elsewhere.test",
        "http://chat.test",
        "https://chat.test:8443",
        "https://chat.test/",
        "https://CHAT.test",
        "https://chat.test.evil.test",
        "https://evil.test/https://chat.test",
        "",
    ] {
        let mut spec = spec();
        spec.client.origin = origin.into();
        refused(&spec, Error::OriginNotAllowed);
    }
}

#[test]
fn cross_origin_ceremonies_are_refused() {
    let mut spec = spec();
    spec.client.cross_origin = Some(true);
    refused(&spec, Error::CrossOriginNotAllowed);
    spec.client.cross_origin = Some(false);
    spec.client.top_origin = Some("https://chat.test".into());
    refused(&spec, Error::CrossOriginNotAllowed);

    // Not saying is the same as false.
    spec.client.cross_origin = None;
    spec.client.top_origin = None;
    assert!(verify(&spec.build(key(Algorithm::Es256))).is_ok());
}

#[test]
fn client_data_that_is_not_the_expected_json_is_refused() {
    for raw in [
        &b"not json"[..],
        b"",
        b"[]",
        b"{}",
        br#"{"type":"webauthn.create","challenge":"pQ","origin":3}"#,
        br#"{"type":"webauthn.create","type":"webauthn.create","challenge":"pQ","origin":"https://chat.test"}"#,
        b"\xff\xfe{}",
    ] {
        let mut spec = spec();
        spec.client.raw = Some(raw.to_vec());
        refused(&spec, Error::MalformedClientData);
    }
}

#[test]
fn the_relying_party_id_hash_is_checked() {
    let mut spec = spec();
    spec.rp_id = "elsewhere.test".into();
    refused(&spec, Error::RpIdHashMismatch);
    spec.rp_id = "CHAT.test".into();
    refused(&spec, Error::RpIdHashMismatch);
}

#[test]
fn user_presence_is_required() {
    let mut spec = spec();
    spec.flags = UV | AT;
    refused(&spec, Error::UserNotPresent);
}

#[test]
fn user_verification_is_required() {
    let mut spec = spec();
    spec.flags = UP | AT;
    refused(&spec, Error::UserNotVerified);
}

#[test]
fn a_backed_up_credential_must_be_backup_eligible() {
    let mut spec = spec();
    spec.flags = UP | UV | AT | BS;
    refused(&spec, Error::BackupStateWithoutEligibility);
}

#[test]
fn attested_credential_data_is_required() {
    let mut header = sha256(RP_ID.as_bytes());
    header.extend_from_slice(&[UP | UV, 0, 0, 0, 0]);
    let object = map(&[
        (text("fmt"), text("none")),
        (text("attStmt"), map(&[])),
        (text("authData"), bytes(&header)),
    ]);
    let mut spec = spec();
    spec.attestation_object = Some(object);
    refused(&spec, Error::AttestedDataMissing);
}

#[test]
fn the_credential_id_is_checked() {
    let mut spec = spec();
    spec.raw_id = Some(vec![9; 32]);
    refused(&spec, Error::CredentialIdMismatch);

    let mut empty = self::spec();
    empty.credential_id = vec![];
    empty.raw_id = Some(vec![1]);
    refused(&empty, Error::CredentialIdLength);

    let mut long = self::spec();
    long.credential_id = vec![1; 1024];
    long.raw_id = Some(vec![1; 1024]);
    refused(&long, Error::FieldTooLarge("rawId"));
    // Past the largest value the length field can hold sensibly: the attested
    // length itself is refused when the browser's own copy was within bounds.
    long.raw_id = Some(vec![1; 32]);
    refused(&long, Error::CredentialIdLength);

    let mut largest = self::spec();
    largest.credential_id = vec![1; 1023];
    assert!(verify(&largest.build(key(Algorithm::Es256))).is_ok());

    let mut none = self::spec();
    none.raw_id = Some(vec![]);
    refused(&none, Error::CredentialIdLength);
}

#[test]
fn the_envelope_is_checked() {
    let mut spec = spec();
    spec.kind = "password".into();
    refused(&spec, Error::CredentialTypeNotPublicKey);

    let mut spec = self::spec();
    spec.id = Some(b64_of(&[1, 2, 3]));
    refused(&spec, Error::IdDoesNotMatchRawId);

    let mut spec = self::spec();
    spec.id = Some("not base64!".into());
    refused(&spec, Error::BadBase64("id"));

    let mut response = self::spec().build(key(Algorithm::Es256));
    response.raw_id = format!("{}=", response.raw_id);
    assert_eq!(verify(&response).err(), Some(Error::BadBase64("rawId")));
}

fn b64_of(bytes: &[u8]) -> String {
    stellarr_webauthn::b64::encode(bytes)
}

#[test]
fn transports_that_are_not_known_are_refused() {
    let mut spec = spec();
    spec.transports = Some(vec!["carrier-pigeon".into()]);
    refused(&spec, Error::UnknownTransport);
}

#[test]
fn oversized_and_undecodable_fields_are_refused_before_use() {
    let mut response = spec().build(key(Algorithm::Es256));
    response.response.attestation_object = "A".repeat(40_000);
    assert_eq!(
        verify(&response).err(),
        Some(Error::FieldTooLarge("attestationObject"))
    );

    let mut response = spec().build(key(Algorithm::Es256));
    response.response.client_data_json = "A".repeat(40_000);
    assert_eq!(
        verify(&response).err(),
        Some(Error::FieldTooLarge("clientDataJSON"))
    );

    let mut response = spec().build(key(Algorithm::Es256));
    response.response.attestation_object = "!!!!".into();
    assert_eq!(
        verify(&response).err(),
        Some(Error::BadBase64("attestationObject"))
    );
}

// ---- the algorithm against what was offered ----

fn cose_ec(alg: i64, crv: i64, x: &[u8], y: &[u8]) -> Vec<u8> {
    map(&[
        (int(1), int(2)),
        (int(3), int(alg)),
        (int(-1), int(crv)),
        (int(-2), bytes(x)),
        (int(-3), bytes(y)),
    ])
}

#[test]
fn an_algorithm_that_was_not_offered_is_refused() {
    // ES256K (secp256k1) and RS1 are known to COSE but were never offered.
    for (cose, label) in [
        (cose_ec(-47, 8, &[1; 32], &[2; 32]), "ES256K"),
        (
            map(&[
                (int(1), int(3)),
                (int(3), int(-65535)),
                (int(-1), bytes(&[0x80; 256])),
                (int(-2), bytes(&[1, 0, 1])),
            ]),
            "RS1",
        ),
        (
            cose_ec(-8, 1, &[1; 32], &[2; 32]),
            "an EdDSA number on an EC key",
        ),
        (cose_ec(0, 1, &[1; 32], &[2; 32]), "zero"),
    ] {
        let mut spec = spec();
        spec.cose = Some(cose);
        let result = verify(&spec.build(key(Algorithm::Es256)));
        assert!(
            matches!(
                result,
                Err(Error::AlgorithmNotOffered | Error::MalformedCoseKey(_))
            ),
            "{label}: {result:?}"
        );
    }
    let mut spec = self::spec();
    spec.cose = Some(cose_ec(-47, 8, &[1; 32], &[2; 32]));
    refused(&spec, Error::AlgorithmNotOffered);
}

#[test]
fn a_key_that_disagrees_with_its_algorithm_is_refused() {
    let k = key(Algorithm::Es256);
    let good = cose_of(&k.cose);
    // Curve P-384 under ES256, a coordinate one byte short, a missing y, an
    // extra label, a point that is not on the curve, an OKP key under ES256.
    for (cose, label) in [
        (cose_ec(-7, 2, &good.0, &good.1), "wrong curve"),
        (cose_ec(-7, 1, &good.0[1..], &good.1), "short x"),
        // The same point cut in another place: aws-lc would read it as valid.
        (
            cose_ec(
                -7,
                1,
                &good.0[..31],
                &[good.0[31..].to_vec(), good.1.clone()].concat(),
            ),
            "coordinates shifted",
        ),
        (
            cose_ec(-7, 1, &good.0, &[good.1.clone(), vec![0]].concat()),
            "long y",
        ),
        (
            map(&[
                (int(1), int(2)),
                (int(3), int(-7)),
                (int(-1), int(1)),
                (int(-2), bytes(&good.0)),
            ]),
            "no y",
        ),
        (cose_ec(-7, 1, &good.0, &[0u8; 32]), "off the curve"),
        (
            map(&[
                (int(1), int(1)),
                (int(3), int(-7)),
                (int(-1), int(1)),
                (int(-2), bytes(&good.0)),
                (int(-3), bytes(&good.1)),
            ]),
            "wrong kty",
        ),
        (
            map(&[
                (int(1), int(2)),
                (int(3), int(-7)),
                (int(-1), int(1)),
                (int(-2), bytes(&good.0)),
                (int(-3), bytes(&good.1)),
                (int(4), bytes(&[1])),
            ]),
            "extra label",
        ),
        (
            map(&[
                (int(1), int(2)),
                (int(-1), int(1)),
                (int(-2), bytes(&good.0)),
                (int(-3), bytes(&good.1)),
            ]),
            "no alg",
        ),
        (bytes(&good.0), "not a map"),
    ] {
        let mut spec = spec();
        spec.cose = Some(cose);
        let result = verify(&spec.build(k));
        assert!(
            matches!(result, Err(Error::MalformedCoseKey(_))),
            "{label}: {result:?}"
        );
    }
}

/// The coordinates of a P-256 COSE key written by the test authenticator.
fn cose_of(cose: &[u8]) -> (Vec<u8>, Vec<u8>) {
    // map(5) 01 02 03 26 20 01 21 58 20 <x> 22 58 20 <y>
    let x = cose[10..42].to_vec();
    let y = cose[45..77].to_vec();
    (x, y)
}

#[test]
fn rsa_keys_outside_the_accepted_size_are_refused() {
    for (n, e, label) in [
        (vec![0x80; 255], vec![1, 0, 1], "modulus under 2048 bits"),
        (vec![0x80; 1025], vec![1, 0, 1], "modulus over 8192 bits"),
        (
            [vec![0], vec![0x80; 255]].concat(),
            vec![1, 0, 1],
            "leading zero in the modulus",
        ),
        (
            [vec![0x01], vec![0xff; 255]].concat(),
            vec![1, 0, 1],
            "modulus of 2041 bits",
        ),
        (vec![0x80; 256], vec![1, 0, 1], "even modulus"),
        (
            [vec![0x80; 255], vec![0x01]].concat(),
            vec![1],
            "exponent 1",
        ),
        (
            [vec![0x80; 255], vec![0x01]].concat(),
            vec![2],
            "exponent 2",
        ),
        (
            [vec![0x80; 255], vec![0x01]].concat(),
            vec![1, 0, 0],
            "even exponent",
        ),
        (vec![0x80; 256], vec![], "no exponent"),
        (
            vec![0x80; 256],
            vec![0, 1, 0, 1],
            "leading zero in the exponent",
        ),
        (vec![0x80; 256], vec![1; 9], "exponent over 8 bytes"),
    ] {
        let mut spec = spec();
        spec.cose = Some(map(&[
            (int(1), int(3)),
            (int(3), int(-257)),
            (int(-1), bytes(&n)),
            (int(-2), bytes(&e)),
        ]));
        let result = verify(&spec.build(key(Algorithm::Rs256)));
        assert!(
            matches!(result, Err(Error::MalformedCoseKey(_))),
            "{label}: {result:?}"
        );
    }
}

// ---- the attestation statement ----

#[test]
fn other_attestation_formats_are_refused_not_accepted_unchecked() {
    for format in [
        "tpm",
        "android-key",
        "android-safetynet",
        "fido-u2f",
        "apple",
        "apple-appattest",
        "",
        "None",
        "packed ",
    ] {
        let mut spec = spec();
        spec.statement = Statement::Custom(format.into(), map(&[]));
        refused(&spec, Error::UnsupportedAttestationFormat);
    }
}

#[test]
fn packed_with_a_certificate_chain_is_refused_not_accepted_unchecked() {
    let mut spec = spec();
    spec.statement = Statement::Custom(
        "packed".into(),
        map(&[
            (text("alg"), int(-7)),
            (text("sig"), bytes(&[1; 70])),
            (text("x5c"), [vec![0x81], bytes(&[1; 10])].concat()),
        ]),
    );
    refused(&spec, Error::UnsupportedAttestationFormat);
    spec.statement = Statement::Custom(
        "packed".into(),
        map(&[
            (text("alg"), int(-7)),
            (text("sig"), bytes(&[1; 70])),
            (text("ecdaaKeyId"), bytes(&[1; 10])),
        ]),
    );
    refused(&spec, Error::UnsupportedAttestationFormat);
}

#[test]
fn no_attestation_means_an_empty_statement() {
    let mut spec = spec();
    spec.statement = Statement::Custom("none".into(), map(&[(text("alg"), int(-7))]));
    refused(&spec, Error::AttestationStatementInvalid);
}

#[test]
fn a_packed_statement_has_exactly_alg_and_sig() {
    for statement in [
        map(&[(text("alg"), int(-7))]),
        map(&[(text("sig"), bytes(&[1; 70]))]),
        map(&[(text("alg"), text("ES256")), (text("sig"), bytes(&[1; 70]))]),
        map(&[(text("alg"), int(-7)), (text("sig"), text("x"))]),
        map(&[
            (text("alg"), int(-7)),
            (text("sig"), bytes(&[1; 70])),
            (text("other"), int(1)),
        ]),
        map(&[]),
    ] {
        let mut spec = spec();
        spec.statement = Statement::Custom("packed".into(), statement);
        refused(&spec, Error::AttestationStatementInvalid);
    }
}

#[test]
fn the_self_attestation_algorithm_must_be_the_credentials() {
    let mut spec = spec();
    spec.attestation_alg = Some(Algorithm::Rs256.cose_id());
    refused(&spec, Error::AttestationAlgorithmMismatch);
}

#[test]
fn the_self_attestation_signature_is_checked() {
    // The statement is signed by another key, the attested key is the first.
    let mut stranger = spec();
    stranger.cose = Some(key(Algorithm::Es256).cose.clone());
    let response = stranger.build(other_key(Algorithm::Es256));
    assert_eq!(
        verify(&response).err(),
        Some(Error::AttestationSignatureInvalid)
    );

    // The signature is the first key's, made over other client data.
    let mut elsewhere = spec();
    elsewhere.client.challenge = vec![1; 32];
    let signed_elsewhere = elsewhere.build(key(Algorithm::Es256));
    let mut forged = spec().build(key(Algorithm::Es256));
    forged.response.attestation_object = signed_elsewhere.response.attestation_object;
    assert_eq!(
        verify(&forged).err(),
        Some(Error::AttestationSignatureInvalid)
    );
}

// ---- the encodings ----

#[test]
fn bytes_after_the_attestation_object_are_refused() {
    let good = spec();
    let mut response = good.build(key(Algorithm::Es256));
    let mut object = stellarr_webauthn::b64::decode(&response.response.attestation_object).unwrap();
    object.push(0);
    response.response.attestation_object = b64_of(&object);
    assert_eq!(
        verify(&response).err(),
        Some(Error::Cbor(CborError::TrailingBytes))
    );
}

#[test]
fn bytes_after_the_authenticator_data_are_refused() {
    let mut spec = spec();
    spec.trailing = vec![0];
    refused(&spec, Error::AuthenticatorDataTrailingBytes);
    // Extension bytes without the flag that announces them are the same.
    let mut spec = self::spec();
    spec.extensions = Some(map(&[(text("credProps"), map(&[]))]));
    refused(&spec, Error::AuthenticatorDataTrailingBytes);
}

#[test]
fn authenticator_extensions_are_read_but_not_interpreted() {
    let mut spec = spec();
    spec.flags = UP | UV | AT | ED;
    spec.extensions = Some(map(&[(text("credProtect"), int(2))]));
    assert!(verify(&spec.build(key(Algorithm::Es256))).is_ok());

    // The flag without extension bytes, with something that is not a map, and
    // with bytes after the map.
    let mut missing = spec.clone();
    missing.extensions = None;
    refused(&missing, Error::Cbor(CborError::Truncated));
    let mut not_map = spec.clone();
    not_map.extensions = Some(int(1));
    refused(&not_map, Error::MalformedAuthenticatorData);
    let mut trailing = spec.clone();
    trailing.trailing = vec![0xf6];
    refused(&trailing, Error::AuthenticatorDataTrailingBytes);
}

#[test]
fn the_attestation_object_has_exactly_its_three_members() {
    let auth = spec().auth_data(key(Algorithm::Es256));
    let ok = [
        (text("fmt"), text("none")),
        (text("attStmt"), map(&[])),
        (text("authData"), bytes(&auth)),
    ];
    let cases: Vec<Vec<u8>> = vec![
        map(&ok[..2]),
        map(&[
            ok[0].clone(),
            ok[1].clone(),
            ok[2].clone(),
            (text("extra"), int(1)),
        ]),
        map(&[(text("fmt"), int(1)), ok[1].clone(), ok[2].clone()]),
        map(&[ok[0].clone(), (text("attStmt"), bytes(&[])), ok[2].clone()]),
        map(&[ok[0].clone(), ok[1].clone(), (text("authData"), text("x"))]),
        int(1),
        [head(4, 0)].concat(),
    ];
    for object in cases {
        let mut spec = spec();
        spec.attestation_object = Some(object);
        refused(&spec, Error::MalformedAttestationObject);
    }
}

#[test]
fn cbor_the_specification_forbids_is_refused() {
    let auth = spec().auth_data(key(Algorithm::Es256));
    // A duplicate key (the third member repeats attStmt), an indefinite map,
    // and a length that is not in its shortest form.
    let duplicate = [
        head(5, 3),
        text("fmt"),
        text("none"),
        text("attStmt"),
        map(&[]),
        text("attStmt"),
        map(&[]),
    ]
    .concat();
    let indefinite = [
        vec![0xbf],
        text("fmt"),
        text("none"),
        text("attStmt"),
        map(&[]),
        text("authData"),
        bytes(&auth),
        vec![0xff],
    ]
    .concat();
    let long_form = [
        head(5, 3),
        text("fmt"),
        vec![0x78, 4],
        b"none".to_vec(),
        text("attStmt"),
        map(&[]),
        text("authData"),
        bytes(&auth),
    ]
    .concat();
    for (object, expected) in [
        (duplicate, Error::Cbor(CborError::DuplicateKey)),
        (indefinite, Error::Cbor(CborError::IndefiniteLength)),
        (long_form, Error::Cbor(CborError::NonMinimalEncoding)),
    ] {
        let mut spec = spec();
        spec.attestation_object = Some(object);
        refused(&spec, expected);
    }
}

#[test]
fn truncated_authenticator_data_is_refused_at_every_length() {
    let spec = spec();
    let auth = spec.auth_data(key(Algorithm::Es256));
    for len in 0..auth.len() {
        let object = map(&[
            (text("fmt"), text("none")),
            (text("attStmt"), map(&[])),
            (text("authData"), bytes(&auth[..len])),
        ]);
        let mut spec = self::spec();
        spec.attestation_object = Some(object);
        let result = verify(&spec.build(key(Algorithm::Es256)));
        assert!(result.is_err(), "{len}");
    }
}

#[test]
fn the_cose_key_inside_the_attested_data_must_be_whole() {
    let mut spec = spec();
    let cose = key(Algorithm::Es256).cose.clone();
    spec.cose = Some(cose[..cose.len() - 1].to_vec());
    let result = verify(&spec.build(key(Algorithm::Es256)));
    assert!(result.is_err());
}

#[test]
fn ed25519_keys_of_small_order_are_refused() {
    let mut identity = [0u8; 32];
    identity[0] = 1;
    let mut spellings = vec![[0u8; 32], identity];
    let mut high_sign = identity;
    high_sign[31] = 0x80;
    spellings.push(high_sign);
    // y = p - 1 (order 2), y = p (a spelling of 0) and y = p + 1 (of 1).
    for low in [0xec, 0xed, 0xee] {
        let mut point = [0xff; 32];
        point[0] = low;
        point[31] = 0x7f;
        spellings.push(point);
    }
    for point in spellings {
        let cose = map(&[
            (int(1), int(1)),
            (int(3), int(-8)),
            (int(-1), int(6)),
            (int(-2), bytes(&point)),
        ]);
        let mut spec = spec();
        spec.cose = Some(cose);
        let result = verify(&spec.build(key(Algorithm::EdDsa)));
        assert_eq!(
            result.err(),
            Some(Error::MalformedCoseKey("point of small order")),
            "{point:02x?}"
        );
    }
}

#[test]
fn points_off_the_curve_are_refused() {
    for (alg, crv, size) in [(-7, 1, 32), (-35, 2, 48), (-36, 3, 66)] {
        for (x, y) in [(1u8, 1u8), (1, 0), (0, 1), (7, 9)] {
            let mut xs = vec![0u8; size];
            let mut ys = vec![0u8; size];
            xs[size - 1] = x;
            ys[size - 1] = y;
            let mut spec = spec();
            spec.cose = Some(cose_ec(alg, crv, &xs, &ys));
            let result = verify(&spec.build(key(Algorithm::Es256)));
            assert_eq!(
                result.err(),
                Some(Error::MalformedCoseKey("point rejected")),
                "alg {alg} ({x}, {y})"
            );
        }
    }
}
