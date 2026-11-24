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
    assert!(
        directives.contains("Microsoft.AspNetCore=warn"),
        "{directives}"
    );
}
