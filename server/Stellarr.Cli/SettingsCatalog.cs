using System.Globalization;
using Microsoft.Extensions.Configuration;
using Stellarr.Api.Contracts;
using Stellarr.Api.Domain;
using Stellarr.Api.Integrations;
using Stellarr.Api.Mail;
using Stellarr.Api.Operations;
using Stellarr.Api.Secrets;

namespace Stellarr.Cli;

internal enum SettingKind
{
    Text,
    Boolean,
    Integer,
    TimeZone,
    MailAddress,
    SmtpSecurity,
    MfaRequirement,
}

/// <summary>
/// A setting Cockpit and the CLI both store in the database. The configured
/// preference and the effective availability are separate: GIFs can be switched
/// on while no key is set, and the CLI must not report the preference as false.
/// </summary>
internal sealed record WriteSetting(
    string Key,
    SettingKind Kind,
    string? SeedConfigurationKey,
    Func<OperationsSettings, StellarrServer, ResolvedSecrets, object?> ReadConfigured,
    Func<OperationsSettings, StellarrServer, ResolvedSecrets, object?> ReadEffective,
    Func<OperationsSettings, bool> IsStored,
    Func<UpdateCockpitSettingsRequest, string, UpdateCockpitSettingsRequest> Apply);

/// <summary>A setting that only Compose and the environment can change.</summary>
internal sealed record StartupSetting(
    string Key,
    string? EnvironmentVariable,
    string Guidance,
    Func<StartupContext, StartupValue> Read);

internal static class SettingsCatalog
{
    internal static readonly IReadOnlyList<WriteSetting> Writable =
    [
        Scalar("instance.name", SettingKind.Text, null,
            (_, server, _) => server.Name,
            (request, value) => request with { InstanceName = value }),
        Scalar("time-zone", SettingKind.TimeZone, "Operations:TimeZone",
            (settings, _, _) => settings.TimeZone,
            (request, value) => request with { TimeZone = value }),
        Scalar("mail.enabled", SettingKind.Boolean, "Mail:Enabled",
            (settings, _, _) => settings.MailingEnabled,
            (request, value) => request with { MailingEnabled = ParseBoolean(value) }),
        Scalar("mail.from-address", SettingKind.MailAddress, "Mail:FromAddress",
            (settings, _, _) => settings.MailFromAddress,
            (request, value) => request with { MailFromAddress = value }),
        Scalar("mail.smtp.host", SettingKind.Text, null,
            (settings, _, _) => settings.MailSmtpHost,
            (request, value) => request with { SmtpHost = value }),
        Scalar("mail.smtp.port", SettingKind.Integer, null,
            (settings, _, _) => settings.MailSmtpPort,
            (request, value) => request with { SmtpPort = ParseInteger(value) }),
        Scalar("mail.smtp.username", SettingKind.Text, null,
            (settings, _, _) => settings.MailSmtpUsername,
            (request, value) => request with { SmtpUsername = value }),
        Scalar("mail.smtp.security", SettingKind.SmtpSecurity, null,
            (settings, _, _) => settings.MailSmtpSecurity,
            (request, value) => request with { SmtpSecurity = value }),
        new("mail.resend.enabled", SettingKind.Boolean, null,
            (settings, _, _) => settings.MailResendEnabled,
            (settings, _, secrets) => MailTransports.Resolve(settings, secrets).Provider == MailProvider.Resend,
            settings => settings.MailResendEnabled is not null,
            (request, value) => request with { ResendEnabled = ParseBoolean(value) }),
        Scalar("mail.invites.enabled", SettingKind.Boolean, null,
            (settings, _, _) => settings.InviteMailEnabled,
            (request, value) => request with { InviteMailEnabled = ParseBoolean(value) }),
        Scalar("mail.recovery.enabled", SettingKind.Boolean, null,
            (settings, _, _) => settings.RecoveryMailEnabled,
            (request, value) => request with { RecoveryMailEnabled = ParseBoolean(value) }),
        Scalar("mail.alerts.enabled", SettingKind.Boolean, null,
            (settings, _, _) => settings.AlertMailEnabled,
            (request, value) => request with { AlertMailEnabled = ParseBoolean(value) }),
        new("giphy.enabled", SettingKind.Boolean, null,
            (settings, _, _) => settings.GiphyEnabled,
            (settings, _, secrets) => GiphyState.Resolve(settings, secrets).Enabled,
            settings => settings.GiphyEnabled is not null,
            (request, value) => request with { GiphyEnabled = ParseBoolean(value) }),
        Scalar("retention.backups-days", SettingKind.Integer, null,
            (settings, _, _) => settings.BackupRetentionDays,
            (request, value) => request with { BackupRetentionDays = ParseInteger(value) }),
        Scalar("retention.logs-days", SettingKind.Integer, null,
            (settings, _, _) => settings.LogRetentionDays,
            (request, value) => request with { LogRetentionDays = ParseInteger(value) }),
        Scalar("retention.orphans-days", SettingKind.Integer, null,
            (settings, _, _) => settings.OrphanObjectRetentionDays,
            (request, value) => request with { OrphanObjectRetentionDays = ParseInteger(value) }),
        Scalar("cleanup.enabled", SettingKind.Boolean, null,
            (settings, _, _) => settings.AutomaticCleanupEnabled,
            (request, value) => request with { AutomaticCleanupEnabled = ParseBoolean(value) }),
        Scalar("mfa.requirement", SettingKind.MfaRequirement, null,
            (settings, _, _) => MfaRequirements.Normalize(settings.MfaRequirement),
            (request, value) => request with { MfaRequirement = MfaRequirements.Normalize(value) }),
        Scalar("mfa.trusted-device-days", SettingKind.Integer, null,
            (settings, _, _) => settings.TrustedDeviceDays,
            (request, value) => request with { TrustedDeviceDays = ParseInteger(value) }),
        Scalar("mfa.challenge-minutes", SettingKind.Integer, null,
            (settings, _, _) => settings.AuthChallengeMinutes,
            (request, value) => request with { AuthChallengeMinutes = ParseInteger(value) }),
        Scalar("mfa.email-code-minutes", SettingKind.Integer, null,
            (settings, _, _) => settings.EmailCodeMinutes,
            (request, value) => request with { EmailCodeMinutes = ParseInteger(value) }),
        Scalar("mfa.email-cooldown-seconds", SettingKind.Integer, null,
            (settings, _, _) => settings.EmailCodeCooldownSeconds,
            (request, value) => request with { EmailCodeCooldownSeconds = ParseInteger(value) }),
        Scalar("mfa.email-attempts", SettingKind.Integer, null,
            (settings, _, _) => settings.EmailCodeMaxAttempts,
            (request, value) => request with { EmailCodeMaxAttempts = ParseInteger(value) }),
        Scalar("mfa.email-sends-per-hour", SettingKind.Integer, null,
            (settings, _, _) => settings.EmailCodeSendsPerHour,
            (request, value) => request with { EmailCodeSendsPerHour = ParseInteger(value) }),
        Scalar("mfa.cli-recovery-minutes", SettingKind.Integer, null,
            (settings, _, _) => settings.CliRecoveryMinutes,
            (request, value) => request with { CliRecoveryMinutes = ParseInteger(value) }),
    ];

    internal static readonly IReadOnlyList<StartupSetting> Startup =
    [
        new("public-url",
            $"{StellarrVariables.PublicUrl.Name} / {StellarrVariables.Domain.Name} / {StellarrVariables.PublicPort.Name}",
            $"Set {StellarrVariables.PublicUrl.Name}, or {StellarrVariables.Domain.Name} and {StellarrVariables.PublicPort.Name}, in .env; then recreate the container.",
            ContainerSettings.PublicUrl),
        new("tls.domain", StellarrVariables.Domain.Name,
            $"Set {StellarrVariables.Domain.Name} in .env; the entrypoint also derives TLS and TURN from it. Recreate the container.",
            ContainerSettings.PublicHost),
        new("https-port", StellarrVariables.HttpsPort.Name,
            $"Set {StellarrVariables.HttpsPort.Name} in .env to the host port that HTTPS is published on, then recreate the container.",
            ContainerSettings.HttpsPort),
        FromVariable("oidc.enabled", StellarrVariables.OidcEnabled, "false"),
        FromVariable("oidc.authority", StellarrVariables.OidcIssuer, ""),
        FromVariable("oidc.client-id", StellarrVariables.OidcClientId, ""),
        FromVariable("oidc.trust-provider-mfa", StellarrVariables.OidcTrustProviderMfa, "false",
            " Off unless the provider reports a real second factor in amr."),
        FromVariable("oidc.provider-mfa-amr", StellarrVariables.OidcProviderMfaAmr, "mfa"),
        new("oidc.providers", "Oidc__Providers__<id>__*",
            "Add or remove Oidc__Providers__<id>__Authority, __ClientId, __ClientSecret(File), __DisplayName and the optional __TrustProviderMfa and __ProviderMfaAmr in .env, then recreate the container.",
            ContainerSettings.OidcProviders),
        new("oidc.callback-path", "Oidc__CallbackPath",
            "Set Oidc__CallbackPath in .env and recreate the container. The normal value is /api/auth/oidc/callback.",
            context => ContainerSettings.ConfigValue(context, "Oidc:CallbackPath", "/api/auth/oidc/callback")),
        new("rtc.public-host", StellarrVariables.Domain.Name,
            $"Set {StellarrVariables.Domain.Name} in .env and recreate the container.",
            ContainerSettings.PublicHost),
        FromVariable("rtc.turn-port", StellarrVariables.TurnPort, "3478"),
        FromVariable("rtc.turns-port", StellarrVariables.TurnsPort, "5349"),
        FromVariable("rtc.turns-enabled", StellarrVariables.TurnsEnabled, "false"),
        FromVariable("rtc.turn-tcp-enabled", StellarrVariables.TurnTcpEnabled, "false"),
        FromVariable("rtc.media-port", StellarrVariables.MediaPort, "49000"),
    ];

    /// <summary>A startup setting that is one <c>STELLARR_*</c> variable, read from configuration.</summary>
    private static StartupSetting FromVariable(string key, StellarrVariable variable, string fallback, string note = "") =>
        new(key, variable.Name,
            $"Set {variable.Name} in .env and recreate the container.{note}",
            context => ContainerSettings.ConfigValue(context, variable.Key!, fallback));

    private static WriteSetting Scalar(
        string key,
        SettingKind kind,
        string? seedConfigurationKey,
        Func<OperationsSettings, StellarrServer, ResolvedSecrets, object?> read,
        Func<UpdateCockpitSettingsRequest, string, UpdateCockpitSettingsRequest> apply) =>
        new(key, kind, seedConfigurationKey, read, read, _ => true, apply);

    internal static WriteSetting? FindWritable(string key) =>
        Writable.FirstOrDefault(setting => string.Equals(setting.Key, key, StringComparison.Ordinal));

    internal static StartupSetting? FindStartup(string key) =>
        Startup.FirstOrDefault(setting => string.Equals(setting.Key, key, StringComparison.Ordinal));

    internal static bool ParseBoolean(string value) => value.Trim().ToLowerInvariant() switch
    {
        "true" or "1" or "yes" or "on" => true,
        "false" or "0" or "no" or "off" => false,
        _ => throw new CliException(
            "invalid_value",
            $"'{value}' is not a boolean. Use true or false.",
            CliExit.Usage),
    };

    internal static int ParseInteger(string value)
    {
        if (!int.TryParse(value.Trim(), NumberStyles.Integer, CultureInfo.InvariantCulture, out var parsed))
        {
            throw new CliException(
                "invalid_value",
                $"'{value}' is not an integer.",
                CliExit.Usage);
        }

        return parsed;
    }

    internal static string Display(object? value) => value switch
    {
        null => "(auto)",
        bool boolean => boolean ? "true" : "false",
        int number => number.ToString(CultureInfo.InvariantCulture),
        _ => Convert.ToString(value, CultureInfo.InvariantCulture) ?? "",
    };
}
