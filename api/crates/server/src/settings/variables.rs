//! Every `STELLARR_*` name the API reads, and the key each one sets.
//!
//! This is the only place that spells one. A name without a key either is read
//! by the container's entrypoint or by the operator CLI, or is one of the two
//! the API reads before it has layered any key, [`ENVIRONMENT`] and
//! [`LISTEN`]; it is listed so that the table stays the same as the documented
//! one. The image sets those two itself and an operator never should.

/// The environment the process runs in: `Production`, `Development` or
/// `Testing`. The container's entrypoint sets `Production`; the development
/// stack and the tests set the others.
pub const ENVIRONMENT: &str = "STELLARR_ENVIRONMENT";

/// The addresses the API listens on, such as `127.0.0.1:5000`.
pub const LISTEN: &str = "STELLARR_API_LISTEN";

pub struct Variable {
    pub name: &'static str,
    pub key: Option<&'static str>,
}

const fn row(name: &'static str, key: Option<&'static str>) -> Variable {
    Variable { name, key }
}

pub const VARIABLES: &[Variable] = &[
    // Address and TLS.
    row("STELLARR_DOMAIN", Some("Rtc:PublicHost")),
    row("STELLARR_PUBLIC_PORT", None),
    row("STELLARR_PUBLIC_URL", None),
    row("STELLARR_HTTPS_PORT", None),
    row(
        "STELLARR_TRUSTED_PROXIES",
        Some("Operations:TrustedProxies"),
    ),
    // Voice.
    row("STELLARR_MEDIA_PORT", Some("Rtc:MediaPort")),
    row("STELLARR_TURN_ENABLED", Some("Rtc:TurnEnabled")),
    row("STELLARR_TURN_PORT", Some("Rtc:TurnPort")),
    row("STELLARR_TURNS_PORT", Some("Rtc:TurnsPort")),
    row("STELLARR_TURN_TCP_ENABLED", Some("Rtc:TurnTcpEnabled")),
    row("STELLARR_TURNS_ENABLED", Some("Rtc:TurnsEnabled")),
    // Uploads.
    row(
        "STELLARR_UPLOAD_USER_BYTES",
        Some("Uploads:MaxStoredBytesPerUser"),
    ),
    row(
        "STELLARR_UPLOAD_TOTAL_BYTES",
        Some("Uploads:MaxStoredBytes"),
    ),
    row(
        "STELLARR_MIN_FREE_BYTES",
        Some("Uploads:MinimumFreeDataRootBytes"),
    ),
    row(
        "STELLARR_MAX_CONCURRENT_UPLOADS",
        Some("Uploads:MaxConcurrentUploads"),
    ),
    // Mail.
    row("STELLARR_MAIL_ENABLED", Some("Mail:Enabled")),
    row("STELLARR_SMTP_PASSWORD", Some("Mail:SmtpPassword")),
    row("STELLARR_RESEND_API_KEY", Some("Mail:ResendApiKey")),
    row("STELLARR_MAIL_FROM", Some("Mail:FromAddress")),
    row("STELLARR_MAIL_FROM_NAME", Some("Mail:FromName")),
    // Web Push.
    row("STELLARR_PUSH_ENABLED", Some("Push:Enabled")),
    row("STELLARR_PUSH_SUBJECT", Some("Push:Subject")),
    row("STELLARR_PUSH_PUBLIC_KEY", Some("Push:PublicKey")),
    row("STELLARR_PUSH_PRIVATE_KEY", Some("Push:PrivateKey")),
    // Single sign-on.
    row("STELLARR_OIDC_ENABLED", Some("Oidc:Enabled")),
    row("STELLARR_OIDC_ISSUER", Some("Oidc:Authority")),
    row("STELLARR_OIDC_CLIENT_ID", Some("Oidc:ClientId")),
    row("STELLARR_OIDC_CLIENT_SECRET", Some("Oidc:ClientSecret")),
    row(
        "STELLARR_OIDC_CLIENT_SECRET_FILE",
        Some("Oidc:ClientSecretFile"),
    ),
    row("STELLARR_OIDC_DISPLAY_NAME", Some("Oidc:DisplayName")),
    row(
        "STELLARR_OIDC_TRUST_PROVIDER_MFA",
        Some("Oidc:TrustProviderMfa"),
    ),
    row(
        "STELLARR_OIDC_PROVIDER_MFA_AMR",
        Some("Oidc:ProviderMfaAmr"),
    ),
    // Automatic backups and Cloudflare R2.
    row("STELLARR_BACKUP_ENABLED", Some("Backups:Enabled")),
    row(
        "STELLARR_BACKUP_INTERVAL_HOURS",
        Some("Backups:IntervalHours"),
    ),
    row(
        "STELLARR_BACKUP_RETRY_MINUTES",
        Some("Backups:RetryMinutes"),
    ),
    row(
        "STELLARR_BACKUP_INITIAL_DELAY_MINUTES",
        Some("Backups:InitialDelayMinutes"),
    ),
    row(
        "STELLARR_CLOUDFLARE_R2_ENABLED",
        Some("Cloudflare:R2:Enabled"),
    ),
    row("STELLARR_CLOUDFLARE_R2_MODE", Some("Cloudflare:R2:Mode")),
    row(
        "STELLARR_CLOUDFLARE_R2_ENDPOINT",
        Some("Cloudflare:R2:Endpoint"),
    ),
    row(
        "STELLARR_CLOUDFLARE_R2_REGION",
        Some("Cloudflare:R2:Region"),
    ),
    row(
        "STELLARR_CLOUDFLARE_R2_BUCKET",
        Some("Cloudflare:R2:Bucket"),
    ),
    row(
        "STELLARR_CLOUDFLARE_R2_ACCESS_KEY",
        Some("Cloudflare:R2:AccessKey"),
    ),
    row(
        "STELLARR_CLOUDFLARE_R2_SECRET_KEY",
        Some("Cloudflare:R2:SecretKey"),
    ),
    row(
        "STELLARR_CLOUDFLARE_R2_WORKER_TOKEN",
        Some("Cloudflare:R2:WorkerToken"),
    ),
    row(
        "STELLARR_CLOUDFLARE_R2_PREFIX",
        Some("Cloudflare:R2:Prefix"),
    ),
    // Language.
    row("STELLARR_LANGUAGE", Some("Stellarr:Language")),
    // Instance identity and desktop handshake.
    row("STELLARR_INSTANCE_ID", Some("Stellarr:InstanceId")),
    row("STELLARR_TIME_ZONE", Some("Operations:TimeZone")),
    row(
        "STELLARR_DESKTOP_MINIMUM_VERSION",
        Some("Desktop:MinimumVersion"),
    ),
    row(
        "STELLARR_DESKTOP_RECOMMENDED_VERSION",
        Some("Desktop:RecommendedVersion"),
    ),
    // Other settings.
    row("STELLARR_GIPHY_API_KEY", Some("Giphy:ApiKey")),
    // The image's own data path, and the address the CLI checks the API at.
    row("STELLARR_DATA_ROOT", Some("DataRoot")),
    row("STELLARR_API_URL", None),
    // What the image sets for the process, and an operator never does.
    row(ENVIRONMENT, None),
    row(LISTEN, None),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Config;

    /// Every variable and the key it must land on, written out here rather
    /// than read from the table.
    const EXPECTED: &[(&str, &str)] = &[
        ("STELLARR_DOMAIN", "Rtc:PublicHost"),
        ("STELLARR_TRUSTED_PROXIES", "Operations:TrustedProxies"),
        ("STELLARR_MEDIA_PORT", "Rtc:MediaPort"),
        ("STELLARR_TURN_ENABLED", "Rtc:TurnEnabled"),
        ("STELLARR_TURN_PORT", "Rtc:TurnPort"),
        ("STELLARR_TURNS_PORT", "Rtc:TurnsPort"),
        ("STELLARR_TURN_TCP_ENABLED", "Rtc:TurnTcpEnabled"),
        ("STELLARR_TURNS_ENABLED", "Rtc:TurnsEnabled"),
        (
            "STELLARR_UPLOAD_USER_BYTES",
            "Uploads:MaxStoredBytesPerUser",
        ),
        ("STELLARR_UPLOAD_TOTAL_BYTES", "Uploads:MaxStoredBytes"),
        (
            "STELLARR_MIN_FREE_BYTES",
            "Uploads:MinimumFreeDataRootBytes",
        ),
        (
            "STELLARR_MAX_CONCURRENT_UPLOADS",
            "Uploads:MaxConcurrentUploads",
        ),
        ("STELLARR_MAIL_ENABLED", "Mail:Enabled"),
        ("STELLARR_SMTP_PASSWORD", "Mail:SmtpPassword"),
        ("STELLARR_RESEND_API_KEY", "Mail:ResendApiKey"),
        ("STELLARR_MAIL_FROM", "Mail:FromAddress"),
        ("STELLARR_MAIL_FROM_NAME", "Mail:FromName"),
        ("STELLARR_PUSH_ENABLED", "Push:Enabled"),
        ("STELLARR_PUSH_SUBJECT", "Push:Subject"),
        ("STELLARR_PUSH_PUBLIC_KEY", "Push:PublicKey"),
        ("STELLARR_PUSH_PRIVATE_KEY", "Push:PrivateKey"),
        ("STELLARR_OIDC_ENABLED", "Oidc:Enabled"),
        ("STELLARR_OIDC_ISSUER", "Oidc:Authority"),
        ("STELLARR_OIDC_CLIENT_ID", "Oidc:ClientId"),
        ("STELLARR_OIDC_CLIENT_SECRET", "Oidc:ClientSecret"),
        ("STELLARR_OIDC_CLIENT_SECRET_FILE", "Oidc:ClientSecretFile"),
        ("STELLARR_OIDC_DISPLAY_NAME", "Oidc:DisplayName"),
        ("STELLARR_OIDC_TRUST_PROVIDER_MFA", "Oidc:TrustProviderMfa"),
        ("STELLARR_OIDC_PROVIDER_MFA_AMR", "Oidc:ProviderMfaAmr"),
        ("STELLARR_BACKUP_ENABLED", "Backups:Enabled"),
        ("STELLARR_BACKUP_INTERVAL_HOURS", "Backups:IntervalHours"),
        ("STELLARR_BACKUP_RETRY_MINUTES", "Backups:RetryMinutes"),
        (
            "STELLARR_BACKUP_INITIAL_DELAY_MINUTES",
            "Backups:InitialDelayMinutes",
        ),
        ("STELLARR_CLOUDFLARE_R2_ENABLED", "Cloudflare:R2:Enabled"),
        ("STELLARR_CLOUDFLARE_R2_MODE", "Cloudflare:R2:Mode"),
        ("STELLARR_CLOUDFLARE_R2_ENDPOINT", "Cloudflare:R2:Endpoint"),
        ("STELLARR_CLOUDFLARE_R2_REGION", "Cloudflare:R2:Region"),
        ("STELLARR_CLOUDFLARE_R2_BUCKET", "Cloudflare:R2:Bucket"),
        (
            "STELLARR_CLOUDFLARE_R2_ACCESS_KEY",
            "Cloudflare:R2:AccessKey",
        ),
        (
            "STELLARR_CLOUDFLARE_R2_SECRET_KEY",
            "Cloudflare:R2:SecretKey",
        ),
        (
            "STELLARR_CLOUDFLARE_R2_WORKER_TOKEN",
            "Cloudflare:R2:WorkerToken",
        ),
        ("STELLARR_CLOUDFLARE_R2_PREFIX", "Cloudflare:R2:Prefix"),
        ("STELLARR_LANGUAGE", "Stellarr:Language"),
        ("STELLARR_INSTANCE_ID", "Stellarr:InstanceId"),
        ("STELLARR_TIME_ZONE", "Operations:TimeZone"),
        ("STELLARR_DESKTOP_MINIMUM_VERSION", "Desktop:MinimumVersion"),
        (
            "STELLARR_DESKTOP_RECOMMENDED_VERSION",
            "Desktop:RecommendedVersion",
        ),
        ("STELLARR_GIPHY_API_KEY", "Giphy:ApiKey"),
        ("STELLARR_DATA_ROOT", "DataRoot"),
    ];

    /// None sets a key: the entrypoint or the CLI reads them, or the API reads
    /// them before it has any key.
    const KEYLESS: &[&str] = &[
        "STELLARR_PUBLIC_PORT",
        "STELLARR_PUBLIC_URL",
        "STELLARR_HTTPS_PORT",
        "STELLARR_API_URL",
        ENVIRONMENT,
        LISTEN,
    ];

    #[test]
    fn each_variable_lands_on_its_key() {
        for (variable, key) in EXPECTED {
            let value = format!("value of {variable}");
            let config = Config::layered("Production", &[(variable.to_string(), value.clone())]);

            assert_eq!(config.raw(key), Some(value.as_str()), "{variable}");
        }
    }

    #[test]
    fn the_table_holds_exactly_these_variables() {
        let mut expected: Vec<String> = EXPECTED
            .iter()
            .map(|(name, key)| format!("{name} -> {key}"))
            .chain(KEYLESS.iter().map(|name| format!("{name} -> (none)")))
            .collect();
        let mut actual: Vec<String> = VARIABLES
            .iter()
            .map(|row| format!("{} -> {}", row.name, row.key.unwrap_or("(none)")))
            .collect();
        expected.sort();
        actual.sort();

        assert_eq!(expected, actual);
    }

    #[test]
    fn a_keyless_variable_sets_no_configuration() {
        let before = Config::layered("Production", &[]);
        let variables: Vec<(String, String)> = KEYLESS
            .iter()
            .map(|name| (name.to_string(), "set".to_string()))
            .collect();
        let after = Config::layered("Production", &variables);

        assert_eq!(before.raw("Rtc:PublicHost"), after.raw("Rtc:PublicHost"));
        for variable in VARIABLES.iter().filter(|row| row.key.is_some()) {
            let key = variable.key.unwrap();
            assert_eq!(before.raw(key), after.raw(key), "{key}");
        }
    }
}
