using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.Configuration.CommandLine;
using Microsoft.Extensions.Configuration.EnvironmentVariables;
using Microsoft.Extensions.Configuration.Memory;
using Stellarr.Api.Operations;

namespace Stellarr.Api.Tests;

public sealed class StellarrVariablesTests
{
    /// <summary>
    /// Every variable and the key it must land on, written out here rather
    /// than read from the table: the pairs docker-compose.yml translated, and
    /// those the entrypoint exported under a raw key.
    /// </summary>
    private static readonly (string Variable, string Key)[] Expected =
    [
        ("STELLARR_DOMAIN", "Rtc:PublicHost"),
        ("STELLARR_TRUSTED_PROXIES", "Operations:TrustedProxies"),
        ("STELLARR_MEDIA_PORT", "Rtc:MediaPort"),
        ("STELLARR_TURN_PORT", "Rtc:TurnPort"),
        ("STELLARR_TURNS_PORT", "Rtc:TurnsPort"),
        ("STELLARR_TURN_TCP_ENABLED", "Rtc:TurnTcpEnabled"),
        ("STELLARR_TURNS_ENABLED", "Rtc:TurnsEnabled"),
        ("STELLARR_TURN_SHARED_SECRET", "Rtc:TurnSharedSecret"),
        ("STELLARR_MEDIA_CONTROL_SECRET", "Rtc:MediaControlSecret"),
        ("STELLARR_UPLOAD_USER_BYTES", "Uploads:MaxStoredBytesPerUser"),
        ("STELLARR_UPLOAD_TOTAL_BYTES", "Uploads:MaxStoredBytes"),
        ("STELLARR_MIN_FREE_BYTES", "Uploads:MinimumFreeDataRootBytes"),
        ("STELLARR_MAX_CONCURRENT_UPLOADS", "Uploads:MaxConcurrentUploads"),
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
        ("STELLARR_BACKUP_INITIAL_DELAY_MINUTES", "Backups:InitialDelayMinutes"),
        ("STELLARR_CLOUDFLARE_R2_ENABLED", "Cloudflare:R2:Enabled"),
        ("STELLARR_CLOUDFLARE_R2_MODE", "Cloudflare:R2:Mode"),
        ("STELLARR_CLOUDFLARE_R2_ENDPOINT", "Cloudflare:R2:Endpoint"),
        ("STELLARR_CLOUDFLARE_R2_REGION", "Cloudflare:R2:Region"),
        ("STELLARR_CLOUDFLARE_R2_BUCKET", "Cloudflare:R2:Bucket"),
        ("STELLARR_CLOUDFLARE_R2_ACCESS_KEY", "Cloudflare:R2:AccessKey"),
        ("STELLARR_CLOUDFLARE_R2_SECRET_KEY", "Cloudflare:R2:SecretKey"),
        ("STELLARR_CLOUDFLARE_R2_WORKER_TOKEN", "Cloudflare:R2:WorkerToken"),
        ("STELLARR_CLOUDFLARE_R2_PREFIX", "Cloudflare:R2:Prefix"),
        ("STELLARR_LANGUAGE", "Stellarr:Language"),
        ("STELLARR_INSTANCE_ID", "Stellarr:InstanceId"),
        ("STELLARR_INSTANCE_NAME", "Stellarr:InstanceName"),
        ("STELLARR_TIME_ZONE", "Operations:TimeZone"),
        ("STELLARR_DESKTOP_MINIMUM_VERSION", "Desktop:MinimumVersion"),
        ("STELLARR_DESKTOP_RECOMMENDED_VERSION", "Desktop:RecommendedVersion"),
        ("STELLARR_GIPHY_API_KEY", "Giphy:ApiKey"),
        ("STELLARR_EF_COMMAND_LOG_LEVEL", "Logging:LogLevel:Microsoft.EntityFrameworkCore.Database.Command"),
        ("STELLARR_DATA_ROOT", "DataRoot"),
    ];

    public static TheoryData<string, string> Pairs
    {
        get
        {
            var data = new TheoryData<string, string>();
            foreach (var (variable, key) in Expected)
            {
                data.Add(variable, key);
            }

            return data;
        }
    }

    /// <summary>The names the entrypoint reads and the CLI repeats, and the CLI's own: none sets a key.</summary>
    private static readonly string[] Keyless =
        ["STELLARR_PUBLIC_PORT", "STELLARR_PUBLIC_URL", "STELLARR_HTTPS_PORT", "STELLARR_API_URL"];

    [Theory]
    [MemberData(nameof(Pairs))]
    public void EachVariableLandsOnItsKeyAndNowhereElse(string variable, string key)
    {
        var value = $"value of {variable}";

        var configuration = new ConfigurationBuilder()
            .AddStellarrVariables(new Dictionary<string, string?> { [variable] = value })
            .Build();

        Assert.Equal(value, configuration[key]);
        var set = Assert.Single(configuration.AsEnumerable(), x => x.Value is not null);
        Assert.Equal(key, set.Key, ignoreCase: true);
    }

    [Fact]
    public void TheTableHoldsExactlyTheseVariables()
    {
        var expected = Expected.Select(row => $"{row.Variable} -> {row.Key}")
            .Concat(Keyless.Select(name => $"{name} -> (none)"))
            .Order(StringComparer.Ordinal);
        var actual = StellarrVariables.All.Select(row => $"{row.Name} -> {row.Key ?? "(none)"}")
            .Order(StringComparer.Ordinal);

        Assert.Equal(expected, actual);
    }

    [Fact]
    public void AKeylessVariableSetsNoConfiguration()
    {
        var environment = Keyless.ToDictionary(name => name, string? (name) => "set");

        Assert.Empty(StellarrVariables.Configuration(environment));
    }

    [Theory]
    [InlineData("")]
    [InlineData("   ")]
    [InlineData(" \t\r\n")]
    public void ABlankVariableCountsAsNotSet(string blank)
    {
        var configuration = Layered(
            file: new() { ["Mail:FromName"] = "from the file" },
            raw: new() { ["Mail:FromAddress"] = "raw@example.test" },
            named: new() { ["STELLARR_MAIL_FROM_NAME"] = blank, ["STELLARR_MAIL_FROM"] = blank });

        Assert.Equal("from the file", configuration["Mail:FromName"]);
        Assert.Equal("raw@example.test", configuration["Mail:FromAddress"]);
        Assert.Null(StellarrVariables.ValueIn(
            new Dictionary<string, string?> { ["STELLARR_MAIL_FROM"] = blank }, StellarrVariables.MailFrom));
    }

    [Fact]
    public void AVariableWinsOverTheFileAndTheRawKeyAndKeepsItsSpaces()
    {
        var configuration = Layered(
            file: new() { ["Mail:SmtpPassword"] = "from the file", ["Mail:FromName"] = "from the file" },
            raw: new() { ["Mail:SmtpPassword"] = "raw", ["Mail:FromName"] = "raw" },
            named: new() { ["STELLARR_SMTP_PASSWORD"] = " named with spaces " });

        Assert.Equal(" named with spaces ", configuration["Mail:SmtpPassword"]);
        Assert.Equal("raw", configuration["Mail:FromName"]);
    }

    [Fact]
    public void TheVariablesGoRightAboveTheRawEnvironment()
    {
        var builder = new ConfigurationBuilder()
            .AddInMemoryCollection(new Dictionary<string, string?>())
            .AddEnvironmentVariables(prefix: "DOTNET_")
            .AddEnvironmentVariables()
            .AddCommandLine([]);

        builder.AddStellarrVariables(new Dictionary<string, string?>());

        Assert.Collection(
            builder.Sources,
            source => Assert.IsType<MemoryConfigurationSource>(source),
            source => Assert.Equal("DOTNET_", Assert.IsType<EnvironmentVariablesConfigurationSource>(source).Prefix),
            source => Assert.Null(Assert.IsType<EnvironmentVariablesConfigurationSource>(source).Prefix),
            source => Assert.IsType<MemoryConfigurationSource>(source),
            source => Assert.IsType<CommandLineConfigurationSource>(source));
    }

    [Fact]
    public void ARowIsFoundByItsKey()
    {
        Assert.Same(StellarrVariables.GiphyApiKey, StellarrVariables.ForKey("Giphy:ApiKey"));
        Assert.Throws<InvalidOperationException>(() => StellarrVariables.ForKey("Oidc:CallbackPath"));
    }

    /// <summary>
    /// An appsettings file, then the raw variables as the CLI lays them out
    /// for an environment it is handed, then the table's.
    /// </summary>
    private static IConfiguration Layered(
        Dictionary<string, string?> file,
        Dictionary<string, string?> raw,
        Dictionary<string, string?> named) =>
        new ConfigurationBuilder()
            .AddInMemoryCollection(file)
            .AddInMemoryCollection(raw)
            .AddStellarrVariables(named)
            .Build();
}
