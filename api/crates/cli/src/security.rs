//! `security show | methods | recover | unlink`: an account's second steps and
//! sign-ins, the recovery that revokes them and issues one credential, and the
//! removal of a link to an identity provider.
//!
//! What this tool prints of an account never includes a seed, a hash or a code.
//! The credential `recover` issues is the one thing shown once, and nothing
//! logs it.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, TransactionBehavior, params};
use stellarr_api::auth::policy::SecurityPolicy;
use stellarr_api::auth::ways_in::{self, Providers};
use stellarr_api::auth::{events, recovery};
use stellarr_api::instance::operations;
use stellarr_api::values::{Id, Timestamp};
use stellarr_oidc::options::{ProviderSettings, Settings as OidcSettings};

use crate::database::CliDatabase;
use crate::errors::{CliError, CliOutcome, CliResult, exit};
use crate::invocation::trim;
use crate::json::{Json, moment, round_trip};
use crate::runtime::Runtime;
use crate::support::{User, require_instance_server, resolve_user, user_not_found, yes_no};

const MAX_EVENTS: usize = 100;

fn optional_moment(at: Option<Timestamp>) -> Json {
    at.map_or(Json::Null, moment)
}

fn load_policy(runtime: &Runtime, connection: &rusqlite::Connection) -> CliOutcome<SecurityPolicy> {
    let server = require_instance_server(connection)?;
    let loaded = operations::load(connection, server.id, server.created_at, &runtime.config)?;
    let settings = loaded.settings;
    Ok(SecurityPolicy {
        mfa_requirement: stellarr_api::auth::policy::normalize_requirement(Some(
            &settings.mfa_requirement,
        )),
        trusted_device_days: settings.trusted_device_days,
        auth_challenge_minutes: settings.auth_challenge_minutes,
        email_code_minutes: settings.email_code_minutes,
        email_code_cooldown_seconds: settings.email_code_cooldown_seconds,
        email_code_max_attempts: settings.email_code_max_attempts,
        email_code_sends_per_hour: settings.email_code_sends_per_hour,
        cli_recovery_minutes: settings.cli_recovery_minutes,
    })
}

pub fn show(runtime: &mut Runtime, id_or_username: &str) -> CliOutcome {
    show_as(runtime, id_or_username, "security.show")
}

pub fn methods(runtime: &mut Runtime, id_or_username: &str) -> CliOutcome {
    show_as(runtime, id_or_username, "security.methods")
}

struct Method {
    id: Id,
    kind: String,
    name: String,
    confirmed: bool,
    last_used_at: Option<Timestamp>,
    email: Option<String>,
}

struct Device {
    id: Id,
    name: String,
    device: String,
    created_at: Timestamp,
    expires_at: Timestamp,
    last_used_at: Option<Timestamp>,
}

struct Session {
    id: Id,
    device: String,
    ip_address: String,
    created_at: Timestamp,
    last_seen_at: Timestamp,
    expires_at: Timestamp,
}

struct Identity {
    id: Id,
    issuer: String,
    provider: String,
    created_at: Timestamp,
}

struct Event {
    kind: String,
    detail: String,
    ip_address: String,
    device: String,
    created_at: Timestamp,
}

fn show_as(runtime: &mut Runtime, id_or_username: &str, command: &'static str) -> CliOutcome {
    let database = runtime.open_database(false)?;
    let connection = &database.connection;
    let user = resolve_user(connection, id_or_username)?;
    let now = Timestamp::now();

    let mut statement = connection.prepare(
        "SELECT \"Id\", \"Type\", \"Name\", \"ConfirmedAt\" IS NOT NULL, \"LastUsedAt\", \"EmailAddress\"
         FROM \"MfaMethods\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL ORDER BY \"CreatedAt\"",
    )?;
    let methods = statement
        .query_map([user.id], |row| {
            let kind: String = row.get(1)?;
            let email: Option<String> = row.get(5)?;
            Ok(Method {
                id: row.get(0)?,
                email: if kind == "email" { email } else { None },
                kind,
                name: row.get(2)?,
                confirmed: row.get(3)?,
                last_used_at: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut statement = connection.prepare(
        "SELECT \"Id\", \"Name\", \"Device\", \"CreatedAt\", \"ExpiresAt\", \"LastUsedAt\" FROM \"TrustedDevices\"
         WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?2 ORDER BY \"CreatedAt\" DESC",
    )?;
    let devices = statement
        .query_map(params![user.id, now], |row| {
            Ok(Device {
                id: row.get(0)?,
                name: row.get(1)?,
                device: row.get(2)?,
                created_at: row.get(3)?,
                expires_at: row.get(4)?,
                last_used_at: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut statement = connection.prepare(
        "SELECT \"Id\", \"Device\", \"IpAddress\", \"CreatedAt\", \"LastSeenAt\", \"ExpiresAt\" FROM \"UserSessions\"
         WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?2 ORDER BY \"LastSeenAt\" DESC",
    )?;
    let sessions = statement
        .query_map(params![user.id, now], |row| {
            Ok(Session {
                id: row.get(0)?,
                device: row.get(1)?,
                ip_address: row.get(2)?,
                created_at: row.get(3)?,
                last_seen_at: row.get(4)?,
                expires_at: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let remaining_codes: i64 = connection.query_row(
        "SELECT COUNT(*) FROM \"RecoveryCodes\" WHERE \"UserId\" = ?1 AND \"UsedAt\" IS NULL",
        [user.id],
        |row| row.get(0),
    )?;
    let mut statement = connection.prepare(
        "SELECT \"Id\", \"Issuer\", \"ProviderId\", \"CreatedAt\" FROM \"ExternalIdentities\"
         WHERE \"UserId\" = ?1 ORDER BY \"CreatedAt\"",
    )?;
    let identities = statement
        .query_map([user.id], |row| {
            let provider: Option<String> = row.get(2)?;
            Ok(Identity {
                id: row.get(0)?,
                issuer: row.get(1)?,
                provider: provider.unwrap_or_else(|| "default".to_string()),
                created_at: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut statement = connection.prepare(&format!(
        "SELECT \"Kind\", \"Detail\", \"IpAddress\", \"Device\", \"CreatedAt\" FROM \"SecurityEvents\"
         WHERE \"UserId\" = ?1 ORDER BY \"CreatedAt\" DESC LIMIT {MAX_EVENTS}"
    ))?;
    let events = statement
        .query_map([user.id], |row| {
            Ok(Event {
                kind: row.get(0)?,
                detail: row.get(1)?,
                ip_address: row.get(2)?,
                device: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let policy = load_policy(runtime, connection)?;

    let data = Json::object([
        ("userId", Json::text(user.id.to_string())),
        ("username", Json::text(&user.username)),
        ("displayName", Json::text(&user.display_name)),
        ("disabled", user.disabled.into()),
        (
            "securityRecoveryRequired",
            user.security_recovery_required.into(),
        ),
        ("sessionVersion", user.session_version.into()),
        (
            "policy",
            Json::object([
                ("mfaRequirement", policy.mfa_requirement.into()),
                ("trustedDeviceDays", policy.trusted_device_days.into()),
                ("challengeMinutes", policy.auth_challenge_minutes.into()),
                ("emailCodeMinutes", policy.email_code_minutes.into()),
                (
                    "emailCodeCooldownSeconds",
                    policy.email_code_cooldown_seconds.into(),
                ),
                (
                    "emailCodeMaxAttempts",
                    policy.email_code_max_attempts.into(),
                ),
                (
                    "emailCodeSendsPerHour",
                    policy.email_code_sends_per_hour.into(),
                ),
                ("cliRecoveryMinutes", policy.cli_recovery_minutes.into()),
            ]),
        ),
        (
            "methods",
            Json::array(methods.iter().map(|method| {
                Json::object([
                    ("id", Json::text(method.id.to_string())),
                    ("type", Json::text(&method.kind)),
                    ("name", Json::text(&method.name)),
                    ("confirmed", method.confirmed.into()),
                    ("lastUsedAt", optional_moment(method.last_used_at)),
                    ("email", Json::optional_text(method.email.clone())),
                ])
            })),
        ),
        (
            "trustedDevices",
            Json::array(devices.iter().map(|device| {
                Json::object([
                    ("id", Json::text(device.id.to_string())),
                    ("name", Json::text(&device.name)),
                    ("device", Json::text(&device.device)),
                    ("createdAt", moment(device.created_at)),
                    ("expiresAt", moment(device.expires_at)),
                    ("lastUsedAt", optional_moment(device.last_used_at)),
                ])
            })),
        ),
        (
            "sessions",
            Json::array(sessions.iter().map(|session| {
                Json::object([
                    ("id", Json::text(session.id.to_string())),
                    ("device", Json::text(&session.device)),
                    ("ipAddress", Json::text(&session.ip_address)),
                    ("createdAt", moment(session.created_at)),
                    ("lastSeenAt", moment(session.last_seen_at)),
                    ("expiresAt", moment(session.expires_at)),
                ])
            })),
        ),
        ("remainingRecoveryCodes", remaining_codes.into()),
        (
            "identities",
            Json::array(identities.iter().map(|identity| {
                Json::object([
                    ("id", Json::text(identity.id.to_string())),
                    ("issuer", Json::text(&identity.issuer)),
                    ("providerId", Json::text(&identity.provider)),
                    ("createdAt", moment(identity.created_at)),
                ])
            })),
        ),
        (
            "events",
            Json::array(events.iter().map(|event| {
                Json::object([
                    ("kind", Json::text(&event.kind)),
                    ("detail", Json::text(&event.detail)),
                    ("ipAddress", Json::text(&event.ip_address)),
                    ("device", Json::text(&event.device)),
                    ("createdAt", moment(event.created_at)),
                ])
            })),
        ),
    ]);

    let mut lines = vec![
        format!(
            "Account: {} ({}) [{}]",
            user.username, user.display_name, user.id
        ),
        format!("Disabled: {}", yes_no(user.disabled)),
        format!(
            "Recovery required: {}",
            yes_no(user.security_recovery_required)
        ),
        format!("MFA requirement: {}", policy.mfa_requirement),
        format!("Security methods: {}", methods.len()),
    ];
    for method in &methods {
        lines.push(format!(
            "  {}  {}  {}  {}  last used {}{}",
            method.id,
            method.kind,
            method.name,
            if method.confirmed {
                "confirmed"
            } else {
                "pending"
            },
            method
                .last_used_at
                .map_or("(never)".to_string(), round_trip),
            method
                .email
                .as_ref()
                .map_or(String::new(), |email| format!("  {email}"))
        ));
    }
    lines.push(format!("Trusted devices: {}", devices.len()));
    lines.push(format!("Active sessions: {}", sessions.len()));
    lines.push(format!("Unused recovery codes: {remaining_codes}"));
    lines.push(format!("Linked identities: {}", identities.len()));
    for identity in &identities {
        lines.push(format!(
            "  {}  {}  {}  linked {}",
            identity.id,
            identity.issuer,
            identity.provider,
            round_trip(identity.created_at)
        ));
    }
    lines.push(format!("Recent security events: {}", events.len()));
    Ok(CliResult::new(command, data, lines))
}

/// The providers the instance has set up, from the environment, in the order
/// they are configured, and the id the original goes by.
fn configured_providers(runtime: &Runtime) -> CliOutcome<Vec<String>> {
    let config = &runtime.config;
    let flag = |key: &str, default: bool| {
        config
            .raw(key)
            .map_or(default, |value| value.trim().eq_ignore_ascii_case("true"))
    };
    let text = |key: &str, default: &str| {
        config
            .raw(key)
            .map_or_else(|| default.to_string(), str::to_string)
    };
    let provider = |prefix: &str, enabled: bool| ProviderSettings {
        enabled: flag(&format!("{prefix}Enabled"), enabled),
        authority: text(&format!("{prefix}Authority"), ""),
        client_id: text(&format!("{prefix}ClientId"), ""),
        client_secret: text(&format!("{prefix}ClientSecret"), ""),
        client_secret_file: text(&format!("{prefix}ClientSecretFile"), ""),
        display_name: text(&format!("{prefix}DisplayName"), ""),
        trust_provider_mfa: flag(&format!("{prefix}TrustProviderMfa"), false),
        provider_mfa_amr: text(&format!("{prefix}ProviderMfaAmr"), "mfa"),
    };
    let mut named: BTreeMap<String, ProviderSettings> = BTreeMap::new();
    for (name, _) in config.prefixed("Oidc:Providers:") {
        let id = name.split(':').next().unwrap_or("").to_string();
        if !id.is_empty() && !named.contains_key(&id) {
            let settings = provider(&format!("Oidc:Providers:{id}:"), true);
            named.insert(id, settings);
        }
    }
    let settings = OidcSettings {
        original: provider("Oidc:", false),
        named,
        ..OidcSettings::default()
    };
    let providers = settings
        .configured_providers(|path| {
            std::fs::read_to_string(path)
                .ok()
                .map(|content| content.trim().to_string())
        })
        .map_err(|_| CliError::unexpected("InvalidOperationException"))?;
    Ok(providers.into_iter().map(|provider| provider.id).collect())
}

pub fn unlink(runtime: &mut Runtime, id_or_username: &str, identity_id: &str) -> CliOutcome {
    let Some(id) = Id::parse_guid(&trim(identity_id)) else {
        return Err(CliError::new(
            "identity_not_found",
            format!(
                "No linked identity matches '{identity_id}'. Take the id from `security show`."
            ),
            exit::NOT_FOUND,
        ));
    };
    let mut database = runtime.open_database(true)?;
    let user = resolve_user(&database.connection, id_or_username)?;
    let issuer: Option<String> = database
        .connection
        .query_row(
            "SELECT \"Issuer\" FROM \"ExternalIdentities\" WHERE \"Id\" = ?1 AND \"UserId\" = ?2",
            params![id, user.id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(issuer) = issuer else {
        return Err(CliError::new(
            "identity_not_found",
            format!(
                "{} has no linked identity '{identity_id}'. Take the id from `security show`.",
                user.username
            ),
            exit::NOT_FOUND,
        ));
    };
    // The web refuses to remove an account's last way in; an operator may have
    // a reason to, so the tool only says so before asking. It reads the
    // providers from the same settings the server does; one it cannot see
    // counts for nothing, so it warns rather more often, never less.
    let configured = configured_providers(runtime)?;
    let providers = Providers {
        configured: &configured,
        default_id: "default",
    };
    let last_way_in = !ways_in::remain(&database.connection, &providers, user.id, Some(id), None)?;
    runtime.confirm(&format!(
        "Will unlink {issuer} from {} ({}); that identity will no longer sign in to the account. Sessions already open stay open; end them with `sessions revoke`.{}",
        user.display_name,
        user.username,
        if last_way_in {
            " It looks like the account's last way in: no password, no passkey and no other identity at a provider configured here, so only a password reset by mail would open it."
        } else {
            ""
        }
    ))?;
    runtime.check_cancelled()?;
    let transaction = database
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Counted again where nothing can change under it, with the link still there.
    let last_way_in = !ways_in::remain(&transaction, &providers, user.id, Some(id), None)?;
    // Someone else may have removed the link while the question waited: then
    // nothing is recorded, as nothing was changed here.
    let removed = transaction.execute(
        "DELETE FROM \"ExternalIdentities\" WHERE \"Id\" = ?1 AND \"UserId\" = ?2",
        params![id, user.id],
    )?;
    if removed == 0 {
        return Err(CliError::new(
            "identity_not_found",
            format!(
                "{} has no linked identity '{identity_id}'. Take the id from `security show`.",
                user.username
            ),
            exit::NOT_FOUND,
        ));
    }
    let verified = events::verified_email(&transaction, user.id)?;
    events::record(
        &transaction,
        user.id,
        events::IDENTITY_UNLINKED,
        "",
        "",
        &format!("operator: {issuer}"),
        verified.as_deref(),
    )
    .map_err(|_| CliError::unexpected("DbError"))?;
    CliDatabase::audit(&transaction, "security.unlink", &user.id.to_string())?;
    transaction.commit()?;
    let mut result = CliResult::new(
        "security.unlink",
        Json::object([
            ("userId", Json::text(user.id.to_string())),
            ("username", Json::text(&user.username)),
            ("identityId", Json::text(id.to_string())),
            ("issuer", Json::text(&issuer)),
            ("auditActor", "local-cli".into()),
            ("lastWayIn", last_way_in.into()),
        ]),
        vec![
            format!(
                "Unlinked {issuer} from {} ({}).",
                user.display_name, user.username
            ),
            format!("Audit: local-cli security.unlink {}", user.id),
        ],
    );
    result = result.changed();
    if last_way_in {
        result = result.warnings(vec![format!(
            "{} ({}) has no other way in now: no password, no passkey and no other identity at a provider configured here. Only a password reset by mail would open the account; `security recover` issues a credential to enroll new security.",
            user.display_name, user.username
        )]);
    }
    Ok(result)
}

pub fn recover(runtime: &mut Runtime, id_or_username: &str, reason: &str) -> CliOutcome {
    let reason = trim(reason);
    if !(3..=500).contains(&reason.encode_utf16().count()) {
        return Err(CliError::new(
            "invalid_reason",
            "A recovery reason must be 3 to 500 characters long.",
            exit::USAGE,
        ));
    }
    let mut database = runtime.open_database(true)?;
    let connection = &database.connection;
    require_instance_server(connection)?;
    let user = load_user(connection, id_or_username)?;
    if user.disabled {
        return Err(account_disabled());
    }
    let policy = load_policy(runtime, connection)?;
    let preview_now = Timestamp::now();
    let count = |sql: &str, with_now: bool| -> rusqlite::Result<i64> {
        if with_now {
            connection.query_row(sql, params![user.id, preview_now], |row| row.get(0))
        } else {
            connection.query_row(sql, [user.id], |row| row.get(0))
        }
    };
    let methods = count(
        "SELECT COUNT(*) FROM \"MfaMethods\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL",
        false,
    )?;
    let devices = count(
        "SELECT COUNT(*) FROM \"TrustedDevices\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL",
        false,
    )?;
    let sessions = count(
        "SELECT COUNT(*) FROM \"UserSessions\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?2",
        true,
    )?;
    let handoffs = count(
        "SELECT COUNT(*) FROM \"DesktopBrowserLogins\" WHERE \"UserId\" = ?1 AND \"ApprovedAt\" IS NOT NULL AND \"RedeemedAt\" IS NULL",
        false,
    )?;
    runtime.confirm(&format!(
        "Will revoke {methods} security method(s), {devices} trusted device(s), {sessions} active session(s) and {handoffs} approved desktop handoff(s) of {} ({}), then issue one recovery credential valid for {} minutes. Reason: {reason}",
        user.display_name, user.username, policy.cli_recovery_minutes
    ))?;
    runtime.check_cancelled()?;

    // Everything from the policy to the credential is read again inside the
    // write lock: the account may have been disabled, or recovered by someone
    // else, while the question waited.
    let transaction = database
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let policy = load_policy(runtime, &transaction)?;
    let user_id = user.id;
    let recovered = recovery::recover(
        &transaction,
        user_id,
        &reason,
        &policy,
        Timestamp::now(),
        |audit| {
            CliDatabase::audit(audit, "security.recover", &user_id.to_string())
                .map_err(stellarr_api::database::DbError::from)
        },
    )
    .map_err(|_| CliError::unexpected("DbError"))?;
    let Some(recovered) = recovered else {
        return Err(account_disabled());
    };
    transaction.commit()?;
    let data = Json::object([
        ("userId", Json::text(user.id.to_string())),
        ("username", Json::text(&user.username)),
        ("methodsRevoked", recovered.methods_revoked.into()),
        (
            "trustedDevicesRevoked",
            recovered.trusted_devices_revoked.into(),
        ),
        ("sessionsRevoked", recovered.sessions_revoked.into()),
        ("handoffsInvalidated", recovered.handoffs_invalidated.into()),
        ("recoveryCredential", Json::text(&recovered.credential)),
        ("recoveryExpiresAt", moment(recovered.expires_at)),
        ("sessionVersion", recovered.session_version.into()),
        ("securityRecoveryRequired", true.into()),
        ("auditActor", "local-cli".into()),
    ]);
    Ok(CliResult::new(
        "security.recover",
        data,
        vec![
            format!(
                "Recovered the account security of {} ({}).",
                user.display_name, user.username
            ),
            format!(
                "Revoked {} method(s), {} trusted device(s), {} session(s) and {} handoff(s).",
                recovered.methods_revoked,
                recovered.trusted_devices_revoked,
                recovered.sessions_revoked,
                recovered.handoffs_invalidated
            ),
            "Ordinary password, OIDC and desktop sign-in remain blocked until replacement security is enrolled."
                .to_string(),
            format!(
                "Recovery credential (shown once, valid until {}):",
                round_trip(recovered.expires_at)
            ),
            recovered.credential.clone(),
            "Exchange it through POST /api/auth/recovery/redeem; it can only enroll replacement security."
                .to_string(),
            format!("Audit: local-cli security.recover {}", user.id),
        ],
    )
    .changed())
}

fn load_user(connection: &rusqlite::Connection, id_or_username: &str) -> CliOutcome<User> {
    // Recovery names the account as it was typed, blank or not.
    if trim(id_or_username).is_empty() {
        return Err(user_not_found(id_or_username));
    }
    resolve_user(connection, id_or_username).map_err(|error| {
        if error.code == "user_not_found" {
            user_not_found(id_or_username)
        } else {
            error
        }
    })
}

fn account_disabled() -> CliError {
    CliError::new(
        "account_disabled",
        "The account is disabled. Enable it before recovering its security methods.",
        exit::CONFLICT,
    )
}
