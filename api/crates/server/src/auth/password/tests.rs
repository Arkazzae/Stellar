use super::*;

// Written by Python's hashlib.pbkdf2_hmac('sha256', password, salt, iterations, 32).
const FROM_PYTHON: [(&str, &str); 3] = [
    (
        "Passw0rd!2026",
        "pbkdf2-sha256$210000$AAECAwQFBgcICQoLDA0ODw==$F7XbbcEiPZ9xiSyI75OfVvMnEQBrxxhoPRwTxTL13LM=",
    ),
    (
        "zażółć gęślą jaźń",
        "pbkdf2-sha256$100000$BwcHBwcHBwcHBwcHBwcHBw==$viF1WPZGU5GDXyNVfSjE++2NKjoI4YRnCNxCGQpWnoY=",
    ),
    (
        "",
        "pbkdf2-sha256$210000$AAAAAAAAAAAAAAAAAAAAAA==$PTtFNTvzwocxY8dnIaViLKZoW77k1SXKIetMHXRZe9I=",
    ),
];

#[test]
fn hashes_written_elsewhere_verify_here() {
    for (password, stored) in FROM_PYTHON {
        assert!(verify(password, Some(stored)), "{password:?}");
        assert!(!verify("another password", Some(stored)), "{password:?}");
    }
}

#[test]
fn a_new_hash_has_the_stored_shape_and_verifies() {
    let stored = hash("Passw0rd!2026");

    let parts: Vec<&str> = stored.split('$').collect();
    assert_eq!(
        (parts[0], parts[1], parts.len()),
        ("pbkdf2-sha256", "210000", 4)
    );
    assert_eq!(from_base64(parts[2]).unwrap().len(), 16);
    assert_eq!(from_base64(parts[3]).unwrap().len(), 32);
    assert!(verify("Passw0rd!2026", Some(&stored)));
    assert!(!verify("Passw0rd!2027", Some(&stored)));
    assert_ne!(stored, hash("Passw0rd!2026"));
}

#[test]
fn a_hash_the_policy_does_not_accept_verifies_nothing() {
    let (password, stored) = FROM_PYTHON[0];
    for broken in [
        stored.replace("210000", "99999"),
        stored.replace("210000", "1000001"),
        stored.replace("pbkdf2-sha256", "pbkdf2-sha512"),
        stored.replace("$F7Xb", "$F7X"),
        stored.replace("AAECAwQFBgcICQoLDA0ODw==", "AAEC"),
        format!("{stored}$extra"),
        "pbkdf2-sha256$210000$AAECAwQFBgcICQoLDA0ODw==".to_string(),
        String::new(),
    ] {
        assert!(!verify(password, Some(&broken)), "{broken}");
    }
}

#[test]
fn an_account_without_a_password_matches_nothing() {
    assert!(!verify("anything", None));
    assert!(!verify("", None));
}

#[test]
fn length_is_counted_in_utf16_units() {
    assert!(is_too_short(None));
    assert!(is_too_short(Some("short12")));
    assert!(!is_too_short(Some("12345678")));
    // Four characters outside the basic plane are eight units.
    assert!(!is_too_short(Some("𝔘𝔘𝔘𝔘")));
    assert!(!is_too_long(Some(&"p".repeat(256))));
    assert!(is_too_long(Some(&"p".repeat(257))));
    assert!(is_too_long(Some(&"𝔘".repeat(129))));
}
