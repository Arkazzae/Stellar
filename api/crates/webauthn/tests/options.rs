//! The options JSON the web client reads.

mod support;

use serde_json::{Value, json};
use stellarr_webauthn::{
    Challenge, CredentialDescriptor, Error, Transport, User, assertion_options, b64,
    creation_options,
};
use support::relying_party;

fn challenge() -> Challenge {
    Challenge::from_bytes([0xAB; 32])
}

fn parse(json: &str) -> Value {
    serde_json::from_str(json).unwrap()
}

#[test]
fn creation_options_have_the_members_the_client_reads() {
    let user = User {
        handle: &[1, 2, 3, 4],
        name: "maja",
        display_name: "Maja K.",
    };
    let exclude = [
        CredentialDescriptor {
            id: vec![9; 20],
            transports: vec![Transport::Usb, Transport::SmartCard],
        },
        CredentialDescriptor {
            id: vec![8; 5],
            transports: vec![],
        },
    ];
    let options = creation_options(&relying_party(), &user, &exclude, challenge()).unwrap();

    assert_eq!(
        parse(&options.json),
        json!({
            "rp": { "id": "chat.test", "name": "Stellarr" },
            "user": { "id": "AQIDBA", "name": "maja", "displayName": "Maja K." },
            "challenge": b64::encode(&[0xAB; 32]),
            "pubKeyCredParams": [
                {"type": "public-key", "alg": -8},
                {"type": "public-key", "alg": -7},
                {"type": "public-key", "alg": -257},
                {"type": "public-key", "alg": -37},
                {"type": "public-key", "alg": -35},
                {"type": "public-key", "alg": -258},
                {"type": "public-key", "alg": -38},
                {"type": "public-key", "alg": -36},
                {"type": "public-key", "alg": -259},
                {"type": "public-key", "alg": -39},
            ],
            "timeout": 120000,
            "attestation": "none",
            "authenticatorSelection": {
                "residentKey": "preferred",
                "requireResidentKey": false,
                "userVerification": "required"
            },
            "excludeCredentials": [
                {"type": "public-key", "id": b64::encode(&[9; 20]), "transports": ["usb", "smart-card"]},
                {"type": "public-key", "id": b64::encode(&[8; 5])},
            ],
        })
    );
    assert_eq!(options.challenge, challenge());
}

#[test]
fn assertion_options_have_the_members_the_client_reads() {
    let allow = [CredentialDescriptor {
        id: vec![7; 32],
        transports: vec![Transport::Internal],
    }];
    let options = assertion_options(&relying_party(), &allow, challenge()).unwrap();
    assert_eq!(
        parse(&options.json),
        json!({
            "challenge": b64::encode(&[0xAB; 32]),
            "timeout": 120000,
            "rpId": "chat.test",
            "allowCredentials": [
                {"type": "public-key", "id": b64::encode(&[7; 32]), "transports": ["internal"]}
            ],
            "userVerification": "required",
        })
    );

    // Signing in without naming an account sends an empty list, not none.
    let open = assertion_options(&relying_party(), &[], challenge()).unwrap();
    assert_eq!(parse(&open.json)["allowCredentials"], json!([]));
}

#[test]
fn the_user_handle_must_be_one_to_sixty_four_bytes() {
    for (handle, ok) in [
        (vec![], false),
        (vec![1], true),
        (vec![1; 64], true),
        (vec![1; 65], false),
    ] {
        let user = User {
            handle: &handle,
            name: "a",
            display_name: "A",
        };
        let result = creation_options(&relying_party(), &user, &[], challenge());
        assert_eq!(result.is_ok(), ok, "{}", handle.len());
    }
}

#[test]
fn text_in_the_options_is_escaped() {
    let user = User {
        handle: &[1],
        name: "a\"b",
        display_name: "</script>\n",
    };
    let options = creation_options(&relying_party(), &user, &[], challenge()).unwrap();
    let parsed = parse(&options.json);
    assert_eq!(parsed["user"]["name"], "a\"b");
    assert_eq!(parsed["user"]["displayName"], "</script>\n");
}

#[test]
fn challenges_are_32_random_bytes() {
    let a = Challenge::random().unwrap();
    let b = Challenge::random().unwrap();
    assert_eq!(a.as_bytes().len(), 32);
    assert_ne!(a, b);
    assert_ne!(a.as_bytes(), &[0u8; 32]);
}

#[test]
fn transports_round_trip_and_unknown_ones_do_not_parse() {
    for transport in [
        Transport::Usb,
        Transport::Nfc,
        Transport::Ble,
        Transport::SmartCard,
        Transport::Hybrid,
        Transport::Internal,
    ] {
        assert_eq!(Transport::parse(transport.as_str()), Some(transport));
    }
    assert_eq!(Transport::parse("Usb"), None);
    assert_eq!(Transport::parse(""), None);
}

#[test]
fn an_options_error_is_an_error_value() {
    let user = User {
        handle: &[],
        name: "a",
        display_name: "A",
    };
    assert!(matches!(
        creation_options(&relying_party(), &user, &[], challenge()),
        Err(Error::InvalidRequest(_))
    ));
}
