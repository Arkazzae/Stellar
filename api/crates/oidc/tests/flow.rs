//! The sign-in round trip: the request, the sealed state, the callback, the
//! token request and the answers to it, and the tickets.

use stellarr_oidc::discovery::{self, Discovery};
use stellarr_oidc::flow::{
    self, Actor, Answer, Callback, CallbackError as C, Flow, Purpose, ReplayGuard, STATE_LIFETIME,
    Start,
};
use stellarr_oidc::form;
use stellarr_oidc::idtoken::{Expect, IdTokenError};
use stellarr_oidc::jwks;
use stellarr_oidc::options::{Provider, ProviderSettings, Settings};
use stellarr_oidc::sealed::SealingKey;
use stellarr_oidc::ticket::{self, TICKET_LIFETIME, TicketError, TicketKind};
use stellarr_oidc::token::{
    self, TokenError, UserinfoError, parse_token_response, parse_userinfo, token_request,
    userinfo_request,
};

const NOW: i64 = 1_800_000_000;
const JSON: Option<&str> = Some("application/json");
const REDIRECT: &str = "https://localhost/api/auth/oidc/callback";

fn key() -> SealingKey {
    SealingKey::from_bytes(&[7; 32]).unwrap()
}

fn provider(id: &str, trust_mfa: bool) -> Provider {
    let mut settings = Settings::default();
    let provider = ProviderSettings {
        authority: "https://identity.test".into(),
        client_id: "stellarr-tests".into(),
        client_secret: "test-client-secret".into(),
        trust_provider_mfa: trust_mfa,
        ..ProviderSettings::default()
    };
    if id == "default" {
        settings.original = ProviderSettings {
            enabled: true,
            ..provider
        };
    } else {
        settings.named.insert(id.into(), provider);
    }
    settings.find(id, |_| None).unwrap().unwrap()
}

fn discovery() -> Discovery {
    discovery::parse(
        br#"{"issuer":"https://identity.test","authorization_endpoint":"https://identity.test/authorize",
        "token_endpoint":"https://identity.test/token","jwks_uri":"https://identity.test/jwks",
        "userinfo_endpoint":"https://identity.test/userinfo"}"#,
    )
    .unwrap()
}

fn actor() -> Actor {
    Actor {
        user_id: "u".repeat(32),
        session_id: "s".repeat(32),
        security_version: 3,
    }
}

fn begin(
    provider: &Provider,
    purpose: Purpose,
    return_url: Option<&str>,
    short_lived: bool,
) -> flow::Started {
    flow::start(
        &key(),
        &Start {
            provider,
            discovery: &discovery(),
            redirect_uri: REDIRECT,
            purpose,
            return_url,
            short_lived,
            now: NOW,
        },
    )
    .unwrap()
}

fn query(started: &flow::Started) -> std::collections::HashMap<String, String> {
    let (path, query) = started.redirect_to.split_once('?').unwrap();
    assert_eq!(path, "https://identity.test/authorize");
    form::parse(query).unwrap().into_iter().collect()
}

fn answer_for(started: &flow::Started, extra: &str) -> Callback {
    let state = &query(started)["state"];
    Callback::parse(&format!("state={}{extra}", form::encode_component(state))).unwrap()
}

fn accept_at(
    guard: &ReplayGuard,
    started: &flow::Started,
    extra: &str,
    now: i64,
) -> Result<Answer, C> {
    flow::accept(
        &key(),
        guard,
        "default",
        &started.browser_binding,
        &answer_for(started, extra),
        now,
    )
}

fn opened(started: &flow::Started) -> Flow {
    match accept_at(&ReplayGuard::new(), started, "&code=c", NOW).unwrap() {
        Answer::Code { flow, .. } => flow,
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_request_carries_every_parameter_the_sign_in_needs() {
    let started = begin(
        &provider("default", false),
        Purpose::SignIn,
        Some("/invite/abc?source=oidc"),
        false,
    );
    let q = query(&started);
    assert_eq!(q["client_id"], "stellarr-tests");
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["response_mode"], "form_post");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["redirect_uri"], REDIRECT);
    assert_eq!(q["scope"], "openid profile email");
    assert!(
        started
            .redirect_to
            .contains("scope=openid%20profile%20email")
    );
    // An ordinary sign-in leaves the provider to its own session.
    assert!(!q.contains_key("prompt"));
    assert!(!q.contains_key("max_age"));
    assert_eq!(q.len(), 9);

    let flow = opened(&started);
    assert_eq!(q["nonce"], flow.nonce);
    assert_eq!(
        q["code_challenge"],
        flow::code_challenge(&flow.code_verifier)
    );
    assert_eq!(flow.return_url, "/invite/abc?source=oidc");
    assert_eq!(flow.redirect_uri, REDIRECT);
    assert!(!flow.short_lived && !flow.demand_fresh_sign_in);
    assert_eq!(flow.purpose, Purpose::SignIn);
    // The secrets are random per sign-in and long enough to guess nothing.
    let other = opened(&begin(
        &provider("default", false),
        Purpose::SignIn,
        None,
        false,
    ));
    assert_ne!(flow.nonce, other.nonce);
    assert_ne!(flow.code_verifier, other.code_verifier);
    assert_ne!(flow.state_id, other.state_id);
    assert!(flow.nonce.len() >= 43 && flow.code_verifier.len() >= 43);
    // The verifier is within the lengths PKCE allows.
    assert!((43..=128).contains(&flow.code_verifier.len()));
}

#[test]
fn pkce_challenge_matches_the_rfc_7636_vector() {
    // RFC 7636 appendix B; the pair was also recomputed with openssl.
    assert_eq!(
        flow::code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

#[test]
fn only_a_trusted_provider_is_asked_to_sign_the_person_in_again() {
    let plain = query(&begin(
        &provider("default", false),
        Purpose::SignIn,
        None,
        false,
    ));
    assert!(!plain.contains_key("max_age"));
    let trusted = begin(&provider("default", true), Purpose::SignIn, None, false);
    assert_eq!(query(&trusted)["max_age"], "300");
    assert!(!query(&trusted).contains_key("prompt"));
    // A trusted sign-in is not a demand: a stale auth_time still signs in.
    assert!(!opened(&trusted).demand_fresh_sign_in);
}

#[test]
fn a_fresh_sign_in_asked_for_as_proof_prompts_and_demands() {
    for plain in [false, true] {
        let started = begin(
            &provider("default", plain),
            Purpose::Reauthenticate(actor()),
            Some("/channels/abc"),
            false,
        );
        let q = query(&started);
        assert_eq!(q["max_age"], "300");
        assert_eq!(q["prompt"], "login");
        let flow = opened(&started);
        assert!(flow.demand_fresh_sign_in);
        let discovery = discovery();
        let expect = Expect::for_flow(&discovery, "stellarr-tests", &flow, None, NOW);
        assert_eq!(expect.fresh_within, Some(300));
    }
    // A link needs no fresh sign-in.
    let link = opened(&begin(
        &provider("default", false),
        Purpose::Link(actor()),
        None,
        false,
    ));
    assert!(!link.demand_fresh_sign_in);
    assert_eq!(link.purpose, Purpose::Link(actor()));
}

#[test]
fn the_flow_keeps_the_choice_made_before_leaving() {
    assert!(
        opened(&begin(
            &provider("default", false),
            Purpose::SignIn,
            None,
            true
        ))
        .short_lived
    );
}

#[test]
fn the_return_address_cannot_leave_the_origin() {
    for (given, kept) in [
        ("https://evil.example/", "/"),
        ("//evil.example/", "/"),
        ("/x", "/x"),
    ] {
        assert_eq!(
            opened(&begin(
                &provider("default", false),
                Purpose::SignIn,
                Some(given),
                false
            ))
            .return_url,
            kept
        );
    }
}

#[test]
fn the_authorization_endpoint_may_already_have_a_query() {
    let mut discovery = discovery();
    discovery.authorization_endpoint = "https://identity.test/authorize?tenant=1".into();
    let started = flow::start(
        &key(),
        &Start {
            provider: &provider("default", false),
            discovery: &discovery,
            redirect_uri: REDIRECT,
            purpose: Purpose::SignIn,
            return_url: None,
            short_lived: false,
            now: NOW,
        },
    )
    .unwrap();
    assert!(
        started
            .redirect_to
            .starts_with("https://identity.test/authorize?tenant=1&client_id=")
    );
}

#[test]
fn refuses_a_redirect_address_that_is_not_one() {
    for bad in [
        "",
        "/api/auth/oidc/callback",
        "javascript:alert(1)",
        "https://a.test/cb#frag",
        "https://a.test/c b",
        "https://a.test/\n",
        &format!("https://a.test/{}", "a".repeat(2048)),
    ] {
        let result = flow::start(
            &key(),
            &Start {
                provider: &provider("default", false),
                discovery: &discovery(),
                redirect_uri: bad,
                purpose: Purpose::SignIn,
                return_url: None,
                short_lived: false,
                now: NOW,
            },
        );
        assert_eq!(
            result.map(|_| ()),
            Err(flow::StartError::BadRedirectUri),
            "{bad:?}"
        );
    }
}

#[test]
fn a_valid_callback_gives_the_code_and_the_flow() {
    let started = begin(
        &provider("default", false),
        Purpose::SignIn,
        Some("/"),
        false,
    );
    let guard = ReplayGuard::new();
    let Ok(Answer::Code { code, flow }) = accept_at(&guard, &started, "&code=the-code", NOW + 5)
    else {
        panic!("not accepted");
    };
    assert_eq!(code, "the-code");
    assert_eq!(flow.provider_id, "default");
    assert_eq!(flow.expires_at, NOW + STATE_LIFETIME);
}

#[test]
fn refuses_a_state_that_belongs_to_another_browser() {
    let started = begin(&provider("default", false), Purpose::SignIn, None, false);
    let elsewhere = begin(&provider("default", false), Purpose::SignIn, None, false);
    let callback = answer_for(&started, "&code=c");
    for binding in [
        elsewhere.browser_binding.as_str(),
        "",
        "x",
        &"b".repeat(129),
    ] {
        let result = flow::accept(
            &key(),
            &ReplayGuard::new(),
            "default",
            binding,
            &callback,
            NOW,
        );
        assert_eq!(result.map(|_| ()), Err(C::UnreadableState), "{binding:?}");
    }
    // Another server's key opens nothing either.
    let other_key = SealingKey::from_bytes(&[8; 32]).unwrap();
    let result = flow::accept(
        &other_key,
        &ReplayGuard::new(),
        "default",
        &started.browser_binding,
        &callback,
        NOW,
    );
    assert_eq!(result.map(|_| ()), Err(C::UnreadableState));
}

#[test]
fn refuses_a_state_that_was_already_used() {
    let started = begin(&provider("default", false), Purpose::SignIn, None, false);
    let guard = ReplayGuard::new();
    assert!(accept_at(&guard, &started, "&code=c", NOW + 1).is_ok());
    assert_eq!(
        accept_at(&guard, &started, "&code=c", NOW + 2).map(|_| ()),
        Err(C::Replayed)
    );
    // An error answer uses it up as well, and cannot be replayed into a code.
    let second = begin(&provider("default", false), Purpose::SignIn, None, false);
    assert!(accept_at(&guard, &second, "&error=access_denied", NOW).is_ok());
    assert_eq!(
        accept_at(&guard, &second, "&code=c", NOW).map(|_| ()),
        Err(C::Replayed)
    );
}

#[test]
fn refuses_a_state_that_has_expired() {
    let started = begin(&provider("default", false), Purpose::SignIn, None, false);
    assert!(
        accept_at(
            &ReplayGuard::new(),
            &started,
            "&code=c",
            NOW + STATE_LIFETIME - 1
        )
        .is_ok()
    );
    for now in [NOW + STATE_LIFETIME, NOW + STATE_LIFETIME + 1, NOW + 86_400] {
        assert_eq!(
            accept_at(&ReplayGuard::new(), &started, "&code=c", now).map(|_| ()),
            Err(C::Expired),
            "{now}"
        );
    }
    // Expired and replayed at once reads as expired, and the guard is untouched.
    let guard = ReplayGuard::new();
    assert_eq!(
        accept_at(&guard, &started, "&code=c", NOW + STATE_LIFETIME).map(|_| ()),
        Err(C::Expired)
    );
    assert!(accept_at(&guard, &started, "&code=c", NOW + 1).is_ok());
}

#[test]
fn refuses_a_state_made_for_another_providers_callback() {
    let started = begin(&provider("work", false), Purpose::SignIn, None, false);
    let callback = answer_for(&started, "&code=c");
    let wrong = flow::accept(
        &key(),
        &ReplayGuard::new(),
        "default",
        &started.browser_binding,
        &callback,
        NOW,
    );
    assert_eq!(wrong.map(|_| ()), Err(C::WrongProvider));
    let right = flow::accept(
        &key(),
        &ReplayGuard::new(),
        "work",
        &started.browser_binding,
        &callback,
        NOW,
    );
    assert!(right.is_ok());
}

#[test]
fn refuses_a_state_that_was_damaged_or_never_made() {
    let started = begin(&provider("default", false), Purpose::SignIn, None, false);
    let state = query(&started)["state"].clone();
    let mut bytes = state.clone().into_bytes();
    let middle = bytes.len() / 2;
    bytes[middle] = if bytes[middle] == b'A' { b'B' } else { b'A' };
    for bad in [
        String::from_utf8(bytes).unwrap(),
        state[..state.len() - 4].to_string(),
        "x".into(),
        "!".into(),
        "a".repeat(5000),
    ] {
        let callback = Callback {
            state: Some(bad.clone()),
            code: Some("c".into()),
            ..Callback::default()
        };
        let result = flow::accept(
            &key(),
            &ReplayGuard::new(),
            "default",
            &started.browser_binding,
            &callback,
            NOW,
        );
        assert!(matches!(result, Err(C::UnreadableState)), "{bad:.20}");
    }
    let none = Callback {
        code: Some("c".into()),
        ..Callback::default()
    };
    let result = flow::accept(
        &key(),
        &ReplayGuard::new(),
        "default",
        &started.browser_binding,
        &none,
        NOW,
    );
    assert_eq!(result.map(|_| ()), Err(C::MissingState));
    let empty = Callback {
        state: Some(String::new()),
        ..none
    };
    let result = flow::accept(
        &key(),
        &ReplayGuard::new(),
        "default",
        &started.browser_binding,
        &empty,
        NOW,
    );
    assert_eq!(result.map(|_| ()), Err(C::MissingState));
}

#[test]
fn a_ticket_or_other_sealed_value_is_not_a_state() {
    let ticket = ticket::issue(&key(), TicketKind::Link, actor(), "default", NOW).unwrap();
    let callback = Callback {
        state: Some(ticket),
        code: Some("c".into()),
        ..Callback::default()
    };
    let result = flow::accept(
        &key(),
        &ReplayGuard::new(),
        "default",
        "binding",
        &callback,
        NOW,
    );
    assert_eq!(result.map(|_| ()), Err(C::UnreadableState));
}

#[test]
fn provider_refusals_are_read_only_after_the_state_opens() {
    let started = begin(
        &provider("default", false),
        Purpose::Link(actor()),
        None,
        false,
    );
    for (error, cancelled) in [("access_denied", true), ("server_error", false)] {
        let answer = accept_at(
            &ReplayGuard::new(),
            &started,
            &format!("&error={error}&error_description=no+thanks"),
            NOW,
        );
        let Ok(Answer::Refused {
            error: seen,
            description,
            cancelled: said_no,
            flow,
        }) = answer
        else {
            panic!("{error}");
        };
        assert_eq!((seen.as_str(), said_no), (error, cancelled));
        assert_eq!(description.as_deref(), Some("no thanks"));
        // The flow says which page to go back to.
        assert_eq!(flow.purpose, Purpose::Link(actor()));
    }
    // An error beside a code is still an error.
    assert!(matches!(
        accept_at(&ReplayGuard::new(), &started, "&error=x&code=c", NOW),
        Ok(Answer::Refused { .. })
    ));
    // An error with a state nobody can open is not believed.
    let forged = Callback::parse("error=access_denied&state=nonsense").unwrap();
    let result = flow::accept(
        &key(),
        &ReplayGuard::new(),
        "default",
        &started.browser_binding,
        &forged,
        NOW,
    );
    assert_eq!(result.map(|_| ()), Err(C::UnreadableState));
}

#[test]
fn an_answer_with_neither_code_nor_error_is_refused() {
    let started = begin(&provider("default", false), Purpose::SignIn, None, false);
    for extra in ["", "&code="] {
        assert_eq!(
            accept_at(&ReplayGuard::new(), &started, extra, NOW).map(|_| ()),
            Err(C::MissingCode),
            "{extra:?}"
        );
    }
}

#[test]
fn the_callback_parser_refuses_what_is_malformed_or_oversized() {
    let parsed =
        Callback::parse("code=a%2Bb&state=s&error=e&error_description=a+b&other=ignored").unwrap();
    assert_eq!(parsed.code.as_deref(), Some("a+b"));
    assert_eq!(parsed.error_description.as_deref(), Some("a b"));
    assert_eq!(Callback::parse("").unwrap(), Callback::default());
    for bad in [
        "code=1&code=2",
        "state=1&state=2",
        "error=1&error=2",
        "code=%zz",
        "code=%ff",
        &format!("code={}", "a".repeat(8 * 1024 + 1)),
        &format!("state={}", "a".repeat(4 * 1024 + 1)),
        &format!("error={}", "a".repeat(257)),
        &format!("x={}", "a".repeat(40_000)),
    ] {
        assert_eq!(Callback::parse(bad), Err(C::Malformed), "{:.30}", bad);
    }
    // A repeated parameter nobody reads is no harm.
    assert!(Callback::parse("x=1&x=2").is_ok());
}

#[test]
fn the_token_request_says_what_to_post_and_how_the_client_proves_itself() {
    let provider = provider("default", false);
    let flow = opened(&begin(&provider, Purpose::SignIn, None, false));
    let request = token_request(&discovery(), &provider, &flow, "the code&=");
    assert_eq!(request.url, "https://identity.test/token");
    assert_eq!(
        token::TokenRequest::CONTENT_TYPE,
        "application/x-www-form-urlencoded"
    );
    let body: std::collections::HashMap<_, _> =
        form::parse(&request.body).unwrap().into_iter().collect();
    assert_eq!(body["grant_type"], "authorization_code");
    assert_eq!(body["code"], "the code&=");
    assert_eq!(body["redirect_uri"], REDIRECT);
    assert_eq!(body["client_id"], "stellarr-tests");
    assert_eq!(body["client_secret"], "test-client-secret");
    assert_eq!(body["code_verifier"], flow.code_verifier);
    assert_eq!(body.len(), 6);
    // Neither secrets nor codes reach a log through Debug.
    let shown = format!("{request:?} {flow:?}");
    for secret in [
        "test-client-secret",
        &flow.code_verifier,
        &flow.nonce,
        "the+code",
    ] {
        assert!(!shown.contains(secret), "{secret}");
    }
}

#[test]
fn reads_the_token_response() {
    let ok = parse_token_response(
        200,
        br#"{"id_token":"a.b.c","access_token":"at","token_type":"Bearer","expires_in":300}"#,
    )
    .unwrap();
    assert_eq!(
        (ok.id_token.as_str(), ok.access_token.as_str()),
        ("a.b.c", "at")
    );
    // An answer without an access token is refused, whatever else it holds.
    for body in [
        &br#"{"id_token":"a.b.c"}"#[..],
        br#"{"id_token":"x","access_token":""}"#,
        br#"{"id_token":"x","access_token":null}"#,
        br#"{"id_token":"x","access_token":5}"#,
    ] {
        assert_eq!(
            parse_token_response(200, body).unwrap_err(),
            TokenError::MissingAccessToken,
            "{body:?}"
        );
    }

    let refused = parse_token_response(
        400,
        br#"{"error":"invalid_grant","error_description":"expired"}"#,
    );
    assert_eq!(
        refused.unwrap_err(),
        TokenError::Provider {
            error: "invalid_grant".into(),
            description: Some("expired".into())
        }
    );
    // A provider that names an error with a success status is still an error.
    assert!(matches!(
        parse_token_response(200, br#"{"error":"x","id_token":"a.b.c"}"#),
        Err(TokenError::Provider { .. })
    ));
    assert_eq!(
        parse_token_response(500, b"oops").unwrap_err(),
        TokenError::Status(500)
    );
    assert_eq!(
        parse_token_response(302, br#"{"id_token":"a.b.c"}"#).unwrap_err(),
        TokenError::Status(302)
    );
    for body in [
        &br#"{}"#[..],
        br#"{"id_token":""}"#,
        br#"{"id_token":5}"#,
        br#"{"id_token":null}"#,
    ] {
        assert_eq!(
            parse_token_response(200, body).unwrap_err(),
            TokenError::MissingIdToken,
            "{body:?}"
        );
    }
    for body in [&b""[..], b"[]", b"null", b"{", b"\xff"] {
        assert_eq!(
            parse_token_response(200, body).unwrap_err(),
            TokenError::NotJson,
            "{body:?}"
        );
    }
    assert_eq!(
        parse_token_response(200, &vec![b' '; stellarr_oidc::MAX_DOCUMENT + 1]).unwrap_err(),
        TokenError::TooLarge
    );
    let long = format!(r#"{{"error":"{}"}}"#, "e".repeat(5000));
    let TokenError::Provider { error, .. } =
        parse_token_response(400, long.as_bytes()).unwrap_err()
    else {
        panic!()
    };
    assert_eq!(error.len(), 256);
}

#[test]
fn the_flow_decides_what_the_id_token_must_say() {
    // A token for another sign-in's nonce is refused through the flow's own expectations.
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/fixtures.json")).unwrap();
    let keys = jwks::parse(&serde_json::to_vec(&fixtures["jwks"]).unwrap()).unwrap();
    let provider = provider("default", false);
    let flow = opened(&begin(&provider, Purpose::SignIn, None, false));
    let discovery = discovery();
    let expect = Expect::for_flow(
        &discovery,
        provider.client_id(),
        &flow,
        None,
        fixtures["now"].as_i64().unwrap(),
    );
    assert_eq!(expect.issuer, "https://identity.test");
    assert_eq!(expect.nonce, flow.nonce);
    let token = fixtures["tokens"]["rs256"].as_str().unwrap();
    let refused = stellarr_oidc::idtoken::verify(token, &keys, &expect).map(|_| ());
    assert_eq!(refused, Err(IdTokenError::WrongNonce));
}

#[test]
fn userinfo_is_asked_only_when_the_provider_has_an_endpoint() {
    let mut discovery = discovery();
    let request = userinfo_request(&discovery, Some("at")).unwrap().unwrap();
    assert_eq!(
        (request.url.as_str(), request.authorization.as_str()),
        ("https://identity.test/userinfo", "Bearer at")
    );
    assert!(!format!("{request:?}").contains("Bearer"));
    assert_eq!(
        userinfo_request(&discovery, None),
        Err(UserinfoError::NoAccessToken)
    );
    assert_eq!(
        userinfo_request(&discovery, Some("")),
        Err(UserinfoError::NoAccessToken)
    );
    discovery.userinfo_endpoint = None;
    assert_eq!(userinfo_request(&discovery, None), Ok(None));
}

#[test]
fn reads_the_userinfo_answer_for_the_person_the_token_names() {
    let body = br#"{"sub":"subject-1","name":"Jan","email":"jan@example.test"}"#;
    for content_type in [
        Some("application/json"),
        Some("Application/JSON; charset=utf-8"),
    ] {
        let info = parse_userinfo(200, content_type, body, "subject-1").unwrap();
        assert_eq!(info["name"], "Jan");
    }
    // The subject is compared exactly: padding is another subject.
    let padded = br#"{"sub":" subject-1 "}"#;
    assert_eq!(
        parse_userinfo(200, JSON, padded, "subject-1").unwrap_err(),
        UserinfoError::SubjectMismatch
    );
    // Only JSON is read; a missing or other content type is refused.
    for content_type in [
        None,
        Some(""),
        Some("text/plain"),
        Some("application/jsonx"),
    ] {
        assert_eq!(
            parse_userinfo(200, content_type, body, "subject-1").unwrap_err(),
            UserinfoError::WrongContentType,
            "{content_type:?}"
        );
    }
    // Someone else's profile, or none, is never taken.
    for body in [
        &br#"{"sub":"someone-else"}"#[..],
        br#"{"name":"Jan"}"#,
        br#"{"sub":5}"#,
        br#"{"sub":null}"#,
    ] {
        assert_eq!(
            parse_userinfo(200, JSON, body, "subject-1").unwrap_err(),
            UserinfoError::SubjectMismatch,
            "{body:?}"
        );
    }
    assert_eq!(
        parse_userinfo(401, JSON, body, "subject-1").unwrap_err(),
        UserinfoError::Status(401)
    );
    assert_eq!(
        parse_userinfo(200, Some("application/jwt"), b"a.b.c", "subject-1").unwrap_err(),
        UserinfoError::SignedAnswer
    );
    for body in [&b""[..], b"[]", b"null", b"{", b"\xff"] {
        assert_eq!(
            parse_userinfo(200, JSON, body, "subject-1").unwrap_err(),
            UserinfoError::NotJson,
            "{body:?}"
        );
    }
    let huge = vec![b' '; stellarr_oidc::MAX_DOCUMENT + 1];
    assert_eq!(
        parse_userinfo(200, JSON, &huge, "subject-1").unwrap_err(),
        UserinfoError::TooLarge
    );
}

#[test]
fn a_ticket_is_bound_to_its_kind_its_account_and_its_lifetime() {
    let link = ticket::issue(&key(), TicketKind::Link, actor(), "work", NOW).unwrap();
    let read = ticket::read(&key(), TicketKind::Link, &link, NOW + 10).unwrap();
    assert_eq!((read.actor, read.provider_id.as_str()), (actor(), "work"));

    // The ticket for a fresh sign-in needs no proof, so it must never start a link, nor the reverse.
    let fresh = ticket::issue(&key(), TicketKind::Reauthenticate, actor(), "work", NOW).unwrap();
    assert_eq!(
        ticket::read(&key(), TicketKind::Link, &fresh, NOW),
        Err(TicketError::Unreadable)
    );
    assert_eq!(
        ticket::read(&key(), TicketKind::Reauthenticate, &link, NOW),
        Err(TicketError::Unreadable)
    );
    assert!(ticket::read(&key(), TicketKind::Reauthenticate, &fresh, NOW).is_ok());

    assert!(ticket::read(&key(), TicketKind::Link, &link, NOW + TICKET_LIFETIME - 1).is_ok());
    for now in [NOW + TICKET_LIFETIME, NOW + 3600] {
        assert_eq!(
            ticket::read(&key(), TicketKind::Link, &link, now),
            Err(TicketError::Expired)
        );
    }
    let other_key = SealingKey::from_bytes(&[9; 32]).unwrap();
    assert_eq!(
        ticket::read(&other_key, TicketKind::Link, &link, NOW),
        Err(TicketError::Unreadable)
    );
    for bad in ["", "x", "!!", &"a".repeat(2049), &link[..link.len() - 3]] {
        assert_eq!(
            ticket::read(&key(), TicketKind::Link, bad, NOW),
            Err(TicketError::Unreadable),
            "{bad:.20}"
        );
    }
    // A sign-in's state is not a ticket.
    let state = query(&begin(
        &provider("default", false),
        Purpose::SignIn,
        None,
        false,
    ))["state"]
        .clone();
    assert_eq!(
        ticket::read(&key(), TicketKind::Link, &state, NOW),
        Err(TicketError::Unreadable)
    );
}

#[test]
fn no_type_that_holds_a_secret_prints_it() {
    let provider = provider("default", false);
    let started = begin(&provider, Purpose::SignIn, None, false);
    let flow = opened(&started);
    let callback = answer_for(&started, "&code=THE-CODE-VALUE");
    let answer = accept_at(&ReplayGuard::new(), &started, "&code=THE-CODE-VALUE", NOW).unwrap();
    let refused = accept_at(
        &ReplayGuard::new(),
        &begin(&provider, Purpose::SignIn, None, false),
        "&error=x",
        NOW,
    );
    let discovery = discovery();
    let expect = Expect::for_flow(
        &discovery,
        "stellarr-tests",
        &flow,
        Some("ACCESS-TOKEN-VALUE"),
        NOW,
    );
    let response = parse_token_response(
        200,
        br#"{"id_token":"ID.TOKEN.VALUE","access_token":"ACCESS-TOKEN-VALUE"}"#,
    )
    .unwrap();
    let userinfo = userinfo_request(&discovery, Some("ACCESS-TOKEN-VALUE"))
        .unwrap()
        .unwrap();
    let shown = format!(
        "{started:?} {callback:?} {answer:?} {refused:?} {expect:?} {flow:?} {response:?} {userinfo:?} {provider:?} {:?}",
        provider.settings
    );
    let state = query(&started)["state"].clone();
    for secret in [
        started.browser_binding.as_str(),
        &state,
        &flow.nonce,
        &flow.code_verifier,
        "THE-CODE-VALUE",
        "ACCESS-TOKEN-VALUE",
        "ID.TOKEN.VALUE",
        "test-client-secret",
    ] {
        assert!(!shown.contains(secret), "{secret} is printed");
    }
}

#[test]
fn a_state_its_own_callback_would_refuse_is_never_made() {
    let provider = provider("default", false);
    let discovery = discovery();
    let go = |redirect_uri: &str, return_url: &str| {
        flow::start(
            &key(),
            &Start {
                provider: &provider,
                discovery: &discovery,
                redirect_uri,
                purpose: Purpose::SignIn,
                return_url: Some(return_url),
                short_lived: false,
                now: NOW,
            },
        )
    };
    // A long return address that escapes to twice its length falls back to `/`.
    let quotes = format!("/{}", "\"".repeat(2047));
    let started = go(REDIRECT, &quotes).unwrap();
    let state = &query(&started)["state"];
    assert!(state.len() <= 4096, "{}", state.len());
    assert_eq!(opened(&started).return_url, "/");
    // A moderate one is kept.
    assert_eq!(opened(&go(REDIRECT, "/a/b").unwrap()).return_url, "/a/b");
    // A redirect address too big even with no return address is an error.
    let big = format!("https://a.test/{}", "\"".repeat(2020));
    assert_eq!(go(&big, "/").map(|_| ()), Err(flow::StartError::TooLarge));
    // Whatever is made is read back by the callback parser.
    let started = go(REDIRECT, &quotes).unwrap();
    assert!(
        Callback::parse(&format!(
            "state={}",
            form::encode_component(&query(&started)["state"])
        ))
        .is_ok()
    );
}

#[test]
fn an_empty_error_parameter_is_no_error() {
    let started = begin(&provider("default", false), Purpose::SignIn, None, false);
    let answer = accept_at(&ReplayGuard::new(), &started, "&error=&code=c", NOW).unwrap();
    assert!(matches!(answer, Answer::Code { .. }));
    let started = begin(&provider("default", false), Purpose::SignIn, None, false);
    assert_eq!(
        accept_at(&ReplayGuard::new(), &started, "&error=", NOW).map(|_| ()),
        Err(C::MissingCode)
    );
}
