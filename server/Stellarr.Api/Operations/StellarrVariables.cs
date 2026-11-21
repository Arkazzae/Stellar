using System.Collections;
using System.Globalization;
using System.Reflection;
using Microsoft.Extensions.Configuration.EnvironmentVariables;
using Microsoft.Extensions.Configuration.Memory;

namespace Stellarr.Api.Operations;

/// <summary>
/// A setting an operator sets as an environment variable: in <c>.env</c>, with
/// <c>docker run -e</c> or in a platform template, always under this one name.
/// </summary>
/// <param name="Name">The variable, <c>STELLARR_</c> and the setting's name.</param>
/// <param name="Key">
/// The configuration key it sets in the API and the CLI. Null for a name only
/// the container's entrypoint reads, which the CLI repeats to report the value,
/// and for the CLI's own.
/// </param>
public sealed record StellarrVariable(string Name, string? Key);

/// <summary>
/// Every <c>STELLARR_*</c> name the API and the CLI read, and where each lands.
/// This is the only place server code spells one. A variable that is empty or
/// only whitespace counts as not set; one that is set wins over
/// <c>appsettings*.json</c> and over the raw <c>Section__Key</c> variable of
/// the same key. Settings without a name here keep only the raw form, such as
/// <c>Oidc__Providers__&lt;id&gt;__*</c> and <c>Logging__LogLevel__*</c>.
/// </summary>
/// <remarks>
/// One row per line, as <c>infra/tests/setting-names.test.sh</c> reads them.
/// Defaults are not here: they live in the option classes and the
/// <c>appsettings*.json</c> files, and the entrypoint's own in the entrypoint.
/// </remarks>
public static class StellarrVariables
{
    // Address and TLS.
    public static readonly StellarrVariable Domain = new("STELLARR_DOMAIN", "Rtc:PublicHost");
    public static readonly StellarrVariable PublicPort = new("STELLARR_PUBLIC_PORT", null);
    public static readonly StellarrVariable PublicUrl = new("STELLARR_PUBLIC_URL", null);
    public static readonly StellarrVariable HttpsPort = new("STELLARR_HTTPS_PORT", null);
    public static readonly StellarrVariable TrustedProxies = new("STELLARR_TRUSTED_PROXIES", "Operations:TrustedProxies");

    // Voice.
    public static readonly StellarrVariable MediaPort = new("STELLARR_MEDIA_PORT", "Rtc:MediaPort");
    public static readonly StellarrVariable TurnPort = new("STELLARR_TURN_PORT", "Rtc:TurnPort");
    public static readonly StellarrVariable TurnsPort = new("STELLARR_TURNS_PORT", "Rtc:TurnsPort");
    public static readonly StellarrVariable TurnTcpEnabled = new("STELLARR_TURN_TCP_ENABLED", "Rtc:TurnTcpEnabled");
    public static readonly StellarrVariable TurnsEnabled = new("STELLARR_TURNS_ENABLED", "Rtc:TurnsEnabled");
    public static readonly StellarrVariable TurnSharedSecret = new("STELLARR_TURN_SHARED_SECRET", "Rtc:TurnSharedSecret");
    public static readonly StellarrVariable MediaControlSecret = new("STELLARR_MEDIA_CONTROL_SECRET", "Rtc:MediaControlSecret");

    // Uploads.
    public static readonly StellarrVariable UploadUserBytes = new("STELLARR_UPLOAD_USER_BYTES", "Uploads:MaxStoredBytesPerUser");
    public static readonly StellarrVariable UploadTotalBytes = new("STELLARR_UPLOAD_TOTAL_BYTES", "Uploads:MaxStoredBytes");
    public static readonly StellarrVariable MinFreeBytes = new("STELLARR_MIN_FREE_BYTES", "Uploads:MinimumFreeDataRootBytes");
    public static readonly StellarrVariable MaxConcurrentUploads = new("STELLARR_MAX_CONCURRENT_UPLOADS", "Uploads:MaxConcurrentUploads");

    // Mail.
    public static readonly StellarrVariable MailEnabled = new("STELLARR_MAIL_ENABLED", "Mail:Enabled");
    public static readonly StellarrVariable SmtpPassword = new("STELLARR_SMTP_PASSWORD", "Mail:SmtpPassword");
    public static readonly StellarrVariable ResendApiKey = new("STELLARR_RESEND_API_KEY", "Mail:ResendApiKey");
    public static readonly StellarrVariable MailFrom = new("STELLARR_MAIL_FROM", "Mail:FromAddress");
    public static readonly StellarrVariable MailFromName = new("STELLARR_MAIL_FROM_NAME", "Mail:FromName");

    // Web Push.
    public static readonly StellarrVariable PushEnabled = new("STELLARR_PUSH_ENABLED", "Push:Enabled");
    public static readonly StellarrVariable PushSubject = new("STELLARR_PUSH_SUBJECT", "Push:Subject");
    public static readonly StellarrVariable PushPublicKey = new("STELLARR_PUSH_PUBLIC_KEY", "Push:PublicKey");
    public static readonly StellarrVariable PushPrivateKey = new("STELLARR_PUSH_PRIVATE_KEY", "Push:PrivateKey");

    // Single sign-on.
    public static readonly StellarrVariable OidcEnabled = new("STELLARR_OIDC_ENABLED", "Oidc:Enabled");
    public static readonly StellarrVariable OidcIssuer = new("STELLARR_OIDC_ISSUER", "Oidc:Authority");
    public static readonly StellarrVariable OidcClientId = new("STELLARR_OIDC_CLIENT_ID", "Oidc:ClientId");
    public static readonly StellarrVariable OidcClientSecret = new("STELLARR_OIDC_CLIENT_SECRET", "Oidc:ClientSecret");
    public static readonly StellarrVariable OidcClientSecretFile = new("STELLARR_OIDC_CLIENT_SECRET_FILE", "Oidc:ClientSecretFile");
    public static readonly StellarrVariable OidcDisplayName = new("STELLARR_OIDC_DISPLAY_NAME", "Oidc:DisplayName");
    public static readonly StellarrVariable OidcTrustProviderMfa = new("STELLARR_OIDC_TRUST_PROVIDER_MFA", "Oidc:TrustProviderMfa");
    public static readonly StellarrVariable OidcProviderMfaAmr = new("STELLARR_OIDC_PROVIDER_MFA_AMR", "Oidc:ProviderMfaAmr");

    // Automatic backups and Cloudflare R2.
    public static readonly StellarrVariable BackupEnabled = new("STELLARR_BACKUP_ENABLED", "Backups:Enabled");
    public static readonly StellarrVariable BackupIntervalHours = new("STELLARR_BACKUP_INTERVAL_HOURS", "Backups:IntervalHours");
    public static readonly StellarrVariable BackupRetryMinutes = new("STELLARR_BACKUP_RETRY_MINUTES", "Backups:RetryMinutes");
    public static readonly StellarrVariable BackupInitialDelayMinutes = new("STELLARR_BACKUP_INITIAL_DELAY_MINUTES", "Backups:InitialDelayMinutes");
    public static readonly StellarrVariable CloudflareR2Enabled = new("STELLARR_CLOUDFLARE_R2_ENABLED", "Cloudflare:R2:Enabled");
    public static readonly StellarrVariable CloudflareR2Mode = new("STELLARR_CLOUDFLARE_R2_MODE", "Cloudflare:R2:Mode");
    public static readonly StellarrVariable CloudflareR2Endpoint = new("STELLARR_CLOUDFLARE_R2_ENDPOINT", "Cloudflare:R2:Endpoint");
    public static readonly StellarrVariable CloudflareR2Region = new("STELLARR_CLOUDFLARE_R2_REGION", "Cloudflare:R2:Region");
    public static readonly StellarrVariable CloudflareR2Bucket = new("STELLARR_CLOUDFLARE_R2_BUCKET", "Cloudflare:R2:Bucket");
    public static readonly StellarrVariable CloudflareR2AccessKey = new("STELLARR_CLOUDFLARE_R2_ACCESS_KEY", "Cloudflare:R2:AccessKey");
    public static readonly StellarrVariable CloudflareR2SecretKey = new("STELLARR_CLOUDFLARE_R2_SECRET_KEY", "Cloudflare:R2:SecretKey");
    public static readonly StellarrVariable CloudflareR2WorkerToken = new("STELLARR_CLOUDFLARE_R2_WORKER_TOKEN", "Cloudflare:R2:WorkerToken");
    public static readonly StellarrVariable CloudflareR2Prefix = new("STELLARR_CLOUDFLARE_R2_PREFIX", "Cloudflare:R2:Prefix");

    // Language.
    public static readonly StellarrVariable Language = new("STELLARR_LANGUAGE", "Stellarr:Language");

    // Instance identity and desktop handshake.
    public static readonly StellarrVariable InstanceId = new("STELLARR_INSTANCE_ID", "Stellarr:InstanceId");
    public static readonly StellarrVariable InstanceName = new("STELLARR_INSTANCE_NAME", "Stellarr:InstanceName");
    public static readonly StellarrVariable TimeZone = new("STELLARR_TIME_ZONE", "Operations:TimeZone");
    public static readonly StellarrVariable DesktopMinimumVersion = new("STELLARR_DESKTOP_MINIMUM_VERSION", "Desktop:MinimumVersion");
    public static readonly StellarrVariable DesktopRecommendedVersion = new("STELLARR_DESKTOP_RECOMMENDED_VERSION", "Desktop:RecommendedVersion");

    // Other settings.
    public static readonly StellarrVariable GiphyApiKey = new("STELLARR_GIPHY_API_KEY", "Giphy:ApiKey");
    public static readonly StellarrVariable EfCommandLogLevel = new("STELLARR_EF_COMMAND_LOG_LEVEL", "Logging:LogLevel:Microsoft.EntityFrameworkCore.Database.Command");

    // The image's own data path, which the entrypoint, the TLS script and the
    // CLI share, and the address the CLI checks the API at.
    public static readonly StellarrVariable DataRoot = new("STELLARR_DATA_ROOT", "DataRoot");
    public static readonly StellarrVariable ApiUrl = new("STELLARR_API_URL", null);

    /// <summary>Every row above, gathered from the fields so that a row cannot be left out.</summary>
    /// <remarks>Declared after the rows: static fields are initialised in the order they are written.</remarks>
    public static IReadOnlyList<StellarrVariable> All { get; } = typeof(StellarrVariables)
        .GetFields(BindingFlags.Public | BindingFlags.Static)
        .Where(field => field.FieldType == typeof(StellarrVariable))
        .Select(field => (StellarrVariable)field.GetValue(null)!)
        .ToArray();

    /// <summary>The row that sets <paramref name="key"/>.</summary>
    /// <exception cref="InvalidOperationException">No variable sets that key.</exception>
    public static StellarrVariable ForKey(string key) =>
        All.FirstOrDefault(variable => string.Equals(variable.Key, key, StringComparison.OrdinalIgnoreCase))
        ?? throw new InvalidOperationException($"No STELLARR_* variable sets {key}.");

    /// <summary>The value of <paramref name="variable"/> in <paramref name="environment"/>, or null when it is unset or blank.</summary>
    public static string? ValueIn(IReadOnlyDictionary<string, string?> environment, StellarrVariable variable)
    {
        ArgumentNullException.ThrowIfNull(environment);
        ArgumentNullException.ThrowIfNull(variable);
        return environment.TryGetValue(variable.Name, out var value) && !string.IsNullOrWhiteSpace(value) ? value : null;
    }

    /// <summary>The configuration <paramref name="environment"/> sets through the rows that have a key.</summary>
    public static Dictionary<string, string?> Configuration(IReadOnlyDictionary<string, string?> environment)
    {
        var values = new Dictionary<string, string?>(StringComparer.OrdinalIgnoreCase);
        foreach (var variable in All)
        {
            if (variable.Key is { } key && ValueIn(environment, variable) is { } value)
            {
                values[key] = value;
            }
        }

        return values;
    }

    /// <summary>This process's environment, as the API and the CLI read it at start.</summary>
    public static Dictionary<string, string?> ProcessEnvironment()
    {
        var values = new Dictionary<string, string?>(StringComparer.Ordinal);
        foreach (DictionaryEntry entry in Environment.GetEnvironmentVariables())
        {
            values[Convert.ToString(entry.Key, CultureInfo.InvariantCulture) ?? ""] =
                Convert.ToString(entry.Value, CultureInfo.InvariantCulture);
        }

        return values;
    }
}

public static class StellarrVariablesConfigurationExtensions
{
    /// <summary>
    /// Adds what <paramref name="environment"/> sets through
    /// <see cref="StellarrVariables"/>, right above the raw environment
    /// variables: over <c>appsettings*.json</c> and <c>Section__Key</c>, under
    /// the command line and anything added later. Without a raw environment
    /// source the values go on top.
    /// </summary>
    public static IConfigurationBuilder AddStellarrVariables(
        this IConfigurationBuilder builder,
        IReadOnlyDictionary<string, string?> environment)
    {
        ArgumentNullException.ThrowIfNull(builder);
        var source = new MemoryConfigurationSource { InitialData = StellarrVariables.Configuration(environment) };
        var sources = builder.Sources;
        var rawEnvironment = -1;
        for (var index = 0; index < sources.Count; index++)
        {
            if (sources[index] is EnvironmentVariablesConfigurationSource { Prefix: null or "" })
            {
                rawEnvironment = index;
            }
        }

        if (rawEnvironment < 0)
        {
            sources.Add(source);
        }
        else
        {
            sources.Insert(rawEnvironment + 1, source);
        }

        return builder;
    }
}
