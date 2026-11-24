use super::*;

fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn production(extra: &[(&str, &str)]) -> Result<Settings, SettingError> {
    let mut pairs = vec![
        ("ASPNETCORE_ENVIRONMENT", "Production"),
        ("Rtc__TurnSharedSecret", "a-secret"),
    ];
    pairs.extend_from_slice(extra);
    Settings::load(&env(&pairs))
}

#[test]
fn an_empty_production_environment_gets_the_documented_defaults() {
    let settings = production(&[]).unwrap();

    assert_eq!(
        settings.listen,
        Listen(vec!["127.0.0.1:5000".parse().unwrap()])
    );
    assert_eq!(settings.data_root, PathBuf::from("data"));
    assert_eq!(settings.database.path, PathBuf::from("data/stellarr.db"));
    assert_eq!(settings.database.busy_timeout, Duration::from_secs(30));
    assert_eq!(settings.language, Language::English);
    assert_eq!(settings.rtc.media.control_url, "http://127.0.0.1:5200/");
    assert_eq!(settings.rtc.media.media_port, 49000);
    assert!(settings.operations.trusted_proxies.is_empty());
    assert!(settings.clients.require_identification);
    assert_eq!(
        settings.clients.allowed,
        ["web", "cockpit", "desktop-linux", "mobile-android"]
    );
    assert_eq!(settings.clients.download_path.as_deref(), Some("/"));
    assert_eq!(settings.desktop.minimum_version.as_deref(), Some("0.1.0"));
    assert_eq!(
        settings.desktop.recommended_version.as_deref(),
        Some("0.1.3")
    );
    assert_eq!(settings.rate_limits.login, 10);
    assert_eq!(settings.rate_limits.login_poll, 120);
    assert_eq!(settings.rate_limits.auth_write, 5);
    assert_eq!(settings.rate_limits.security_write, 30);
    assert_eq!(settings.rate_limits.invite_preview, 30);
    assert_eq!(settings.rate_limits.uploads_per_minute, 30);
    assert_eq!(settings.rate_limits.pin_unlock, 5);
    assert_eq!(settings.rate_limits.concurrent_uploads, 4);
}

#[test]
fn the_turn_secret_is_required_outside_development_and_testing() {
    let refused = Settings::load(&env(&[("ASPNETCORE_ENVIRONMENT", "Production")])).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "Rtc:TurnSharedSecret is required outside development. In the container the entrypoint \
         sets it from the secret it keeps in the data volume."
    );
    // Not setting the environment at all is production too.
    assert!(Settings::load(&[]).is_err());
    // A blank secret is no secret.
    assert!(production(&[("Rtc__TurnSharedSecret", "  ")]).is_err());

    let development = Settings::load(&env(&[("ASPNETCORE_ENVIRONMENT", "Development")])).unwrap();
    assert_eq!(
        development.rtc.turn_shared_secret.as_deref(),
        Some("stellarr-turn-development-secret")
    );
    assert!(Settings::load(&env(&[("ASPNETCORE_ENVIRONMENT", "testing")])).is_ok());
}

#[test]
fn development_has_its_own_database_file() {
    let settings = Settings::load(&env(&[("ASPNETCORE_ENVIRONMENT", "Development")])).unwrap();

    assert_eq!(
        settings.database.path,
        PathBuf::from("data/stellarr-dev.db")
    );
    assert_eq!(settings.rtc.public_host, "localhost");
}

#[test]
fn the_language_is_read_from_the_variable_and_refused_when_unknown() {
    assert_eq!(
        production(&[("STELLARR_LANGUAGE", "pl")]).unwrap().language,
        Language::Polish
    );
    // A variable a deployment file passes on without filling counts as unset.
    assert_eq!(
        production(&[("STELLARR_LANGUAGE", " ")]).unwrap().language,
        Language::English
    );
    // The raw key set to nothing is the same as English.
    assert_eq!(
        production(&[("Stellarr__Language", "")]).unwrap().language,
        Language::English
    );
    assert_eq!(
        production(&[("STELLARR_LANGUAGE", "de")])
            .unwrap_err()
            .to_string(),
        "Stellarr:Language is \"de\" but must be \"en\" or \"pl\"."
    );
}

#[test]
fn the_data_root_and_the_database_follow_the_variables() {
    let settings = production(&[
        ("STELLARR_DATA_ROOT", "/data/stellarr"),
        (
            "ConnectionStrings__Database",
            "Data Source=/data/stellarr/db/stellarr.db;Pooling=True",
        ),
    ])
    .unwrap();

    assert_eq!(settings.data_root, PathBuf::from("/data/stellarr"));
    assert_eq!(
        settings.database.path,
        PathBuf::from("/data/stellarr/db/stellarr.db")
    );
}

#[test]
fn a_shared_cache_is_refused_and_foreign_keys_stay_on_whatever_the_string_says() {
    let refused = production(&[(
        "ConnectionStrings__Database",
        "Data Source=stellarr.db;Cache=Shared;Pooling=True",
    )])
    .unwrap_err();
    assert!(refused.to_string().contains("Cache=Shared"));

    let kept = production(&[(
        "ConnectionStrings__Database",
        "Data Source=stellarr.db;Foreign Keys=False",
    )])
    .unwrap();
    assert_eq!(kept.database.path, PathBuf::from("stellarr.db"));

    assert!(production(&[("ConnectionStrings__Database", "Pooling=True")]).is_err());
    assert!(production(&[("ConnectionStrings__Database", "Data Source=x.db;Bogus=1")]).is_err());
}

#[test]
fn the_command_timeout_of_the_connection_string_is_the_busy_timeout() {
    let settings = production(&[(
        "ConnectionStrings__Database",
        "Data Source=x.db;Default Timeout=5",
    )])
    .unwrap();

    assert_eq!(settings.database.busy_timeout, Duration::from_secs(5));
}

#[test]
fn the_shipped_connection_strings_are_accepted() {
    for environment in ["Production", "Development"] {
        let settings = Settings::load(&env(&[
            ("ASPNETCORE_ENVIRONMENT", environment),
            ("Rtc__TurnSharedSecret", "s"),
        ]))
        .unwrap();

        assert!(settings.database.path.to_string_lossy().ends_with(".db"));
    }
}

#[test]
fn urls_name_the_addresses_to_listen_on() {
    let listen = |value: &str| Listen::parse(value);

    assert_eq!(
        listen("http://127.0.0.1:5000").unwrap().0,
        ["127.0.0.1:5000".parse().unwrap()]
    );
    assert_eq!(
        listen("http://+:80").unwrap().0,
        ["0.0.0.0:80".parse().unwrap()]
    );
    assert_eq!(
        listen("http://localhost:0/").unwrap().0,
        ["127.0.0.1:0".parse().unwrap()]
    );
    assert_eq!(
        listen("http://[::1]:5000").unwrap().0,
        ["[::1]:5000".parse().unwrap()]
    );
    assert_eq!(
        listen("http://127.0.0.1:1;http://127.0.0.1:2")
            .unwrap()
            .0
            .len(),
        2
    );
    assert!(listen("https://127.0.0.1:5000").is_err());
    assert!(listen("127.0.0.1:5000").is_err());
    assert!(listen("http://127.0.0.1").is_err());
    assert!(listen("http://127.0.0.1:99999").is_err());
    assert!(listen(" ; ").is_err());
}

#[test]
fn the_trusted_proxies_come_from_the_variable_and_a_bad_entry_stops_the_start() {
    let settings = production(&[("STELLARR_TRUSTED_PROXIES", "172.18.0.1, 10.0.0.0/8")]).unwrap();
    assert_eq!(settings.operations.trusted_proxies.networks().len(), 2);

    let refused = production(&[("STELLARR_TRUSTED_PROXIES", "0.0.0.0/0")]).unwrap_err();
    assert!(refused.to_string().contains("STELLARR_TRUSTED_PROXIES"));
}

#[test]
fn the_media_server_settings_are_checked() {
    assert_eq!(
        production(&[("Rtc__MediaControlUrl", "http://media:5200")])
            .unwrap()
            .rtc
            .media
            .control_url,
        "http://media:5200/"
    );
    assert_eq!(
        production(&[("Rtc__MediaControlUrl", "ftp://x")])
            .unwrap_err()
            .to_string(),
        "Rtc:MediaControlUrl must be an absolute http or https address."
    );
    assert_eq!(
        production(&[("STELLARR_MEDIA_PORT", "70000")])
            .unwrap_err()
            .to_string(),
        "Rtc:MediaPort must be between 1 and 65535."
    );
    assert_eq!(
        production(&[("STELLARR_MEDIA_PORT", "50000")])
            .unwrap()
            .rtc
            .media
            .media_port,
        50000
    );
}

#[test]
fn clients_listed_by_the_operator_are_admitted_next_to_the_built_in_ones() {
    let settings = production(&[
        ("Clients__Allowed__0", "web"),
        ("Clients__Allowed__1", "kiosk"),
        ("Clients__RequireIdentification", "false"),
        ("Clients__DownloadPath", ""),
    ])
    .unwrap();

    assert!(settings.clients.allowed.contains(&"kiosk".to_string()));
    assert!(
        settings
            .clients
            .allowed
            .contains(&"mobile-android".to_string())
    );
    assert!(!settings.clients.require_identification);
    assert_eq!(settings.clients.download_path, None);
}

#[test]
fn a_permit_limit_is_held_at_one_or_more_and_testing_lifts_the_limits() {
    let low = production(&[("RateLimiting__Login__PermitLimit", "0")]).unwrap();
    assert_eq!(low.rate_limits.login, 1);
    let set = production(&[("RateLimiting__AuthWrite__PermitLimit", "50")]).unwrap();
    assert_eq!(set.rate_limits.auth_write, 50);
    let wide = production(&[("STELLARR_MAX_CONCURRENT_UPLOADS", "500")]).unwrap();
    assert_eq!(wide.rate_limits.concurrent_uploads, 32);

    let testing = Settings::load(&env(&[("ASPNETCORE_ENVIRONMENT", "Testing")])).unwrap();
    assert_eq!(testing.rate_limits.login, 10_000);
    assert_eq!(testing.rate_limits.concurrent_uploads, 100);
}

#[test]
fn the_workers_that_run_are_chosen_in_the_testing_environment_only() {
    let testing = |extra: &[(&str, &str)]| {
        let mut variables = vec![("ASPNETCORE_ENVIRONMENT", "Testing")];
        variables.extend_from_slice(extra);
        Settings::load(&env(&variables)).unwrap().testing_workers
    };
    assert_eq!(testing(&[]), Some(vec![]));
    assert_eq!(
        testing(&[("Testing__Workers", " mail, previews ,")]),
        Some(vec!["mail".to_string(), "previews".to_string()])
    );

    let other = production(&[("Testing__Workers", "mail")]).unwrap();
    assert_eq!(other.testing_workers, None);
}

#[test]
fn the_event_log_is_honoured_in_the_testing_environment_only() {
    let testing = Settings::load(&env(&[
        ("ASPNETCORE_ENVIRONMENT", "Testing"),
        ("Testing__EventLog", "/tmp/events.jsonl"),
    ]))
    .unwrap();
    assert_eq!(testing.event_log, Some(PathBuf::from("/tmp/events.jsonl")));

    let other = production(&[("Testing__EventLog", "/tmp/events.jsonl")]).unwrap();
    assert_eq!(other.event_log, None);
}

#[test]
fn log_levels_become_tracing_directives() {
    let settings = production(&[
        ("Logging__LogLevel__Stellarr.Api.Requests", "Warning"),
        ("Logging__LogLevel__Default", "Debug"),
    ])
    .unwrap();

    let directives = settings.log.directives();
    assert!(directives.starts_with("debug,"), "{directives}");
    assert!(
        directives.contains("Stellarr.Api.Requests=warn"),
        "{directives}"
    );
}

#[test]
fn mail_starts_from_the_environment_and_the_secrets_keep_their_spaces() {
    let by_default = production(&[]).unwrap();
    let on = production(&[
        ("STELLARR_MAIL_ENABLED", "false"),
        ("STELLARR_MAIL_FROM", " noreply@chat.test "),
        ("Mail__FromName", "  "),
        ("STELLARR_SMTP_PASSWORD", " pass with spaces "),
        ("Mail__ResendApiKey", " re_key "),
    ])
    .unwrap();

    assert!(by_default.mail.enabled);
    assert_eq!(by_default.mail.from_name, "Stellarr");
    assert!(!on.mail.enabled);
    assert_eq!(on.mail.from_address, "noreply@chat.test");
    assert_eq!(on.mail.from_name, "");
    assert_eq!(on.secrets.get("Mail:SmtpPassword"), " pass with spaces ");
    assert_eq!(on.secrets.get("Mail:ResendApiKey"), " re_key ");
    assert_eq!(on.secrets.get("Giphy:ApiKey"), "");
}

#[test]
fn a_secret_is_never_printed_with_the_settings() {
    let settings = production(&[
        ("STELLARR_SMTP_PASSWORD", "hunter2"),
        ("Rtc__MediaControlSecret", "media-hunter3"),
        ("Rtc__TurnSharedSecret", "turn-hunter4"),
    ])
    .unwrap();

    let printed = format!("{settings:?} {:?} {:?}", settings.rtc, settings.rtc.media);
    for secret in ["hunter2", "media-hunter3", "turn-hunter4"] {
        assert!(!printed.contains(secret), "{secret}");
    }
    assert!(printed.contains("control_secret: \"(set)\""));
    assert!(printed.contains("turn_shared_secret: \"(set)\""));
}

#[test]
fn the_canonical_origin_is_reduced_and_a_bad_one_stops_the_start() {
    let settings =
        production(&[("Desktop__CanonicalOrigin", "https://Chat.Example.test:443/")]).unwrap();
    assert_eq!(
        settings.desktop.canonical_origin,
        "https://chat.example.test"
    );
    assert_eq!(production(&[]).unwrap().desktop.canonical_origin, "");

    for bad in [
        "chat.example.test",
        "ftp://chat.example.test",
        "https://u:p@chat.example.test",
        "https://chat.example.test/?next=1",
    ] {
        let refused = production(&[("Desktop__CanonicalOrigin", bad)]).unwrap_err();
        assert!(
            refused
                .to_string()
                .starts_with("Desktop:CanonicalOrigin must be"),
            "{bad}"
        );
    }
}

#[test]
fn the_instance_id_and_the_build_are_read_when_given() {
    let none = production(&[]).unwrap();
    assert_eq!(none.discovery.instance_id, None);
    assert_eq!(none.discovery.build_sha, None);

    let given = production(&[
        (
            "STELLARR_INSTANCE_ID",
            "{11111111-2222-4333-8444-555555555555}",
        ),
        ("Build__Sha", " abc1234 "),
    ])
    .unwrap();
    assert_eq!(
        given.discovery.instance_id,
        Id::parse("11111111-2222-4333-8444-555555555555")
    );
    assert_eq!(given.discovery.build_sha.as_deref(), Some("abc1234"));

    let refused = production(&[("STELLARR_INSTANCE_ID", "not-a-uuid")]).unwrap_err();
    assert_eq!(refused.to_string(), "Stellarr:InstanceId must be a UUID.");
}

#[test]
fn the_waits_for_the_media_server_are_fixed_outside_the_testing_environment() {
    let waits = [
        ("Testing__MediaCommandTimeoutMs", "300"),
        ("Testing__MediaEventWaitMs", "250"),
        ("Testing__MediaRetryPauseMs", "40"),
    ];

    let production = production(&waits).unwrap().rtc.media.timing;
    assert_eq!(production.command, Duration::from_secs(5));
    assert_eq!(production.connect, Duration::from_secs(2));
    assert_eq!(production.event_wait_ms, 25_000);
    assert_eq!(production.retry_pause, Duration::from_secs(2));

    let mut variables = vec![("ASPNETCORE_ENVIRONMENT", "Testing")];
    variables.extend(waits);
    let testing = Settings::load(&env(&variables)).unwrap().rtc.media.timing;
    assert_eq!(testing.command, Duration::from_millis(300));
    assert_eq!(testing.event_wait_ms, 250);
    assert_eq!(testing.retry_pause, Duration::from_millis(40));
    assert_eq!(testing.connect, Duration::from_secs(2));

    variables.push(("Testing__MediaEventWaitMs", "soon"));
    assert!(Settings::load(&env(&variables)).is_err());
}

fn oidc(extra: &[(&str, &str)]) -> Result<Settings, SettingError> {
    production(extra)
}

#[test]
fn no_provider_is_offered_until_one_is_set_up() {
    assert!(oidc(&[]).unwrap().oidc.providers.is_empty());
    // Off by default, whatever else is set.
    let off = oidc(&[
        ("STELLARR_OIDC_ISSUER", "https://id.test"),
        ("STELLARR_OIDC_CLIENT_ID", "stellarr"),
        ("STELLARR_OIDC_CLIENT_SECRET", "s"),
    ])
    .unwrap();
    assert!(off.oidc.providers.is_empty());
}

#[test]
fn the_original_provider_and_the_named_ones_are_read_in_the_order_of_their_ids() {
    let settings = oidc(&[
        ("STELLARR_OIDC_ENABLED", "true"),
        ("STELLARR_OIDC_ISSUER", "https://id.test/"),
        ("STELLARR_OIDC_CLIENT_ID", "stellarr"),
        ("STELLARR_OIDC_CLIENT_SECRET", " s "),
        ("STELLARR_OIDC_DISPLAY_NAME", "Main"),
        ("Oidc__Providers__work__Authority", "https://work.test"),
        ("Oidc__Providers__work__ClientId", "w"),
        ("Oidc__Providers__work__ClientSecret", "ws"),
        ("Oidc__Providers__work__TrustProviderMfa", "TRUE"),
        ("Oidc__Providers__work__ProviderMfaAmr", "mfa,hwk"),
        ("Oidc__Providers__a1__Authority", "https://a.test"),
        ("Oidc__Providers__a1__ClientId", "a"),
        ("Oidc__Providers__a1__ClientSecret", "as"),
        ("Oidc__Providers__a1__Enabled", "false"),
        ("Oidc__Providers__unfinished__Authority", "https://u.test"),
    ])
    .unwrap();

    let ids: Vec<&str> = settings
        .oidc
        .providers
        .iter()
        .map(|provider| provider.id.as_str())
        .collect();
    assert_eq!(ids, ["default", "work"]);
    let work = settings.oidc.find("work").unwrap();
    assert!(work.settings.trust_provider_mfa);
    assert_eq!(work.callback_path, "/api/auth/oidc/work/callback");
    assert_eq!(settings.oidc.find("default").unwrap().client_secret(), "s");
}

#[test]
fn a_secret_is_read_from_its_file_when_none_is_written() {
    let folder = crate::testing::TempDir::new();
    let file = folder.path().join("secret");
    std::fs::write(&file, "  from-a-file\n").unwrap();
    let settings = oidc(&[
        ("Oidc__Providers__work__Authority", "https://work.test"),
        ("Oidc__Providers__work__ClientId", "w"),
        (
            "Oidc__Providers__work__ClientSecretFile",
            file.to_str().unwrap(),
        ),
    ])
    .unwrap();

    assert_eq!(
        settings.oidc.find("work").unwrap().client_secret(),
        "from-a-file"
    );
}

#[test]
fn a_provider_id_no_route_could_serve_refuses_the_start() {
    for id in ["Work", "login", "default", "-x", "a_b"] {
        let refused = oidc(&[(
            &format!("Oidc__Providers__{id}__Authority"),
            "https://x.test",
        )])
        .unwrap_err()
        .to_string();
        assert!(
            refused.contains("is not a valid provider id"),
            "{id}: {refused}"
        );
    }
}

#[test]
fn a_flag_that_is_not_true_or_false_and_a_moved_callback_refuse_the_start() {
    let refused = oidc(&[("Oidc__TrustProviderMfa", "maybe")])
        .unwrap_err()
        .to_string();
    assert!(refused.contains("Oidc:TrustProviderMfa"), "{refused}");
    let refused = oidc(&[("Oidc__CallbackPath", "/signin-oidc")])
        .unwrap_err()
        .to_string();
    assert!(refused.contains("Oidc:CallbackPath"), "{refused}");
    assert!(oidc(&[("Oidc__CallbackPath", "/api/auth/oidc/callback")]).is_ok());
}

#[test]
fn a_provider_missing_a_piece_or_reached_over_http_is_not_set_up() {
    let working = [
        ("Oidc__Enabled", "true"),
        ("Oidc__Authority", "https://identity.test"),
        ("Oidc__ClientId", "stellarr"),
        ("Oidc__ClientSecret", "secret"),
    ];
    assert!(oidc(&working).unwrap().oidc.any_set_up());
    for missing in [
        "Oidc__Enabled",
        "Oidc__Authority",
        "Oidc__ClientId",
        "Oidc__ClientSecret",
    ] {
        let pairs: Vec<_> = working
            .iter()
            .filter(|(key, _)| *key != missing)
            .copied()
            .collect();
        assert!(!oidc(&pairs).unwrap().oidc.any_set_up(), "{missing}");
    }
    for bad in [
        "http://identity.test",
        "identity.test",
        "https://",
        "https://id.test:99999",
        "https://bad host",
        "ftp://id.test",
        "",
    ] {
        let mut pairs = working.to_vec();
        pairs.retain(|(key, _)| *key != "Oidc__Authority");
        pairs.push(("Oidc__Authority", bad));
        assert!(!oidc(&pairs).unwrap().oidc.any_set_up(), "{bad}");
    }
    let found = oidc(&working).unwrap();
    let provider = found.oidc.find("default").unwrap();
    assert_eq!(provider.login_path(), "/api/auth/oidc/login");
    assert_eq!(provider.callback_path, "/api/auth/oidc/callback");
}

#[test]
fn a_provider_id_longer_than_thirty_two_characters_refuses_the_start() {
    let id = "a-very-long-id-that-goes-past-thirty-two-characters";
    let refused = oidc(&[(
        &format!("Oidc__Providers__{id}__Authority"),
        "https://x.test",
    )])
    .unwrap_err();
    assert!(refused.to_string().contains(id), "{refused}");
}

#[test]
fn web_push_is_off_until_it_is_asked_for_and_never_stops_the_start() {
    let off = production(&[]).unwrap();
    let on = production(&[
        ("STELLARR_PUSH_ENABLED", "TRUE"),
        ("STELLARR_PUSH_SUBJECT", "mailto:a@chat.test"),
        ("STELLARR_PUSH_PUBLIC_KEY", "pub"),
        ("STELLARR_PUSH_PRIVATE_KEY", "priv"),
    ])
    .unwrap();
    let odd = production(&[("STELLARR_PUSH_ENABLED", "maybe")]).unwrap();

    assert!(!off.push.enabled);
    assert!(on.push.enabled);
    assert_eq!(on.push.subject, "mailto:a@chat.test");
    assert_eq!(on.push.public_key, "pub");
    assert!(!odd.push.enabled);
    assert!(!format!("{:?}", on.push).contains("priv"));
}
