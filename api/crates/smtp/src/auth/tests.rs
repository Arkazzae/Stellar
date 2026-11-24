use super::*;

// The vectors of RFC 7677 section 3 and RFC 5802 section 5, and the same
// values recomputed with Python's hashlib and hmac.
#[test]
fn scram_sha_256_matches_the_rfc_7677_exchange() {
    let (mut scram, first) = Scram::with_nonce(
        Mechanism::ScramSha256,
        "user",
        "pencil",
        "rOprNGfwEbeRWgbNEkqO".into(),
    )
    .unwrap();
    assert_eq!(first, b"n,,n=user,r=rOprNGfwEbeRWgbNEkqO");
    let last = scram
        .respond(b"r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096")
        .unwrap();
    assert_eq!(
        String::from_utf8(last).unwrap(),
        "c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ="
    );
    scram
        .verify(b"v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=")
        .unwrap();
    assert!(
        scram
            .verify(b"v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G5=")
            .is_err()
    );
    assert!(scram.verify(b"e=invalid-proof").is_err());
    assert!(scram.verify(b"").is_err());
}

#[test]
fn scram_sha_1_matches_the_rfc_5802_exchange() {
    let (mut scram, first) = Scram::with_nonce(
        Mechanism::ScramSha1,
        "user",
        "pencil",
        "fyko+d2lbbFgONRv9qkxdawL".into(),
    )
    .unwrap();
    assert_eq!(first, b"n,,n=user,r=fyko+d2lbbFgONRv9qkxdawL");
    let last = scram
        .respond(b"r=fyko+d2lbbFgONRv9qkxdawL3rfcNHYJY1ZVvWVs7j,s=QSXCR+Q6sek8bf92,i=4096")
        .unwrap();
    assert_eq!(
        String::from_utf8(last).unwrap(),
        "c=biws,r=fyko+d2lbbFgONRv9qkxdawL3rfcNHYJY1ZVvWVs7j,p=v0X8v3Bz2T0CJGbJQyF0X+HI4Ts="
    );
    scram.verify(b"v=rmF9pqV8S7suAoZWja4dJRkFsKQ=").unwrap();
}

#[test]
fn a_hostile_challenge_is_refused() {
    let fresh = || {
        Scram::with_nonce(Mechanism::ScramSha256, "u", "p", "abc".into())
            .unwrap()
            .0
    };
    for bad in [
        "r=abc,s=c2FsdA==,i=4096",          // nonce not extended
        "r=xyzabc,s=c2FsdA==,i=4096",       // nonce does not start with ours
        "r=abcdef,s=c2FsdA==",              // no iterations
        "r=abcdef,s=c2FsdA==,i=0",          // zero
        "r=abcdef,s=c2FsdA==,i=4000000000", // far too many
        "r=abcdef,i=4096",                  // no salt
        "r=abcdef,s=!!!,i=4096",            // salt is not base64
        "m=x,r=abcdef,s=c2FsdA==,i=4096",   // mandatory extension
        "",
    ] {
        assert!(fresh().respond(bad.as_bytes()).is_err(), "{bad}");
    }
    assert!(fresh().respond(&[0xff, 0xfe]).is_err());
}

#[test]
fn the_user_name_is_escaped() {
    let (_, first) = Scram::with_nonce(Mechanism::ScramSha1, "a=b,c", "p", "n".into()).unwrap();
    assert_eq!(first, b"n,,n=a=3Db=2Cc,r=n");
}

#[test]
fn the_strongest_scram_and_one_simple_mechanism_are_tried() {
    let offered = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    let pick = |names: &[&str], u: &str, p: &str| Mechanism::candidates(&offered(names), u, p);
    use Mechanism::*;
    assert_eq!(
        pick(
            &["PLAIN", "LOGIN", "SCRAM-SHA-1", "SCRAM-SHA-256"],
            "u",
            "p"
        ),
        [ScramSha256, Plain]
    );
    assert_eq!(
        pick(&["SCRAM-SHA-1", "LOGIN"], "u", "p"),
        [ScramSha1, Login]
    );
    assert_eq!(pick(&["login", "plain"], "u", "p"), [Plain]);
    assert_eq!(pick(&["LOGIN"], "u", "p"), [Login]);
    assert!(
        pick(
            &[
                "CRAM-MD5",
                "DIGEST-MD5",
                "NTLM",
                "XOAUTH2",
                "SCRAM-SHA-512",
                "SCRAM-SHA-256-PLUS"
            ],
            "u",
            "p"
        )
        .is_empty()
    );
    // Without SASLprep a non-ASCII secret does not go through SCRAM.
    assert_eq!(pick(&["SCRAM-SHA-256", "PLAIN"], "u", "zażółć"), [Plain]);
    assert!(pick(&["SCRAM-SHA-256"], "u", "zażółć").is_empty());
}

#[test]
fn plain_joins_the_parts_with_nul() {
    assert_eq!(plain("jo", "pw"), b"\0jo\0pw");
}
