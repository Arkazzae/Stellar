using System.Globalization;
using Microsoft.Extensions.Configuration;
using Stellarr.Api.Operations;

namespace Stellarr.Cli;

internal sealed record StartupContext(
    IConfiguration Configuration,
    IReadOnlyDictionary<string, string?> Environment);

/// <summary>A read-only startup setting with the source of the value we print.</summary>
internal sealed record StartupValue(string Value, string Source);

/// <summary>
/// Resolves the values the container entrypoint would export before the API
/// starts. <c>docker exec</c> inherits the container's environment but not
/// those exports, so the CLI repeats the entrypoint's public-address rules from
/// the operator's variables instead of showing the API defaults.
/// </summary>
internal static class ContainerSettings
{
    internal static StartupValue PublicUrl(StartupContext context)
    {
        var explicitUrl = StellarrVariables.ValueIn(context.Environment, StellarrVariables.PublicUrl);
        var domain = StellarrVariables.ValueIn(context.Environment, StellarrVariables.Domain);
        if (domain is null && explicitUrl is null)
        {
            var configured = context.Configuration["Operations:PublicUrl"];
            return string.IsNullOrEmpty(configured)
                ? new StartupValue("", "unavailable")
                : new StartupValue(configured.TrimEnd('/'), "configuration");
        }

        if (explicitUrl is not null)
        {
            return new StartupValue(explicitUrl.TrimEnd('/'), "environment");
        }

        var publicPort = StellarrVariables.ValueIn(context.Environment, StellarrVariables.PublicPort)
            ?? HttpsPort(context).Value;
        return new StartupValue(PublicUrlFor(domain!, publicPort), "environment");
    }

    internal static StartupValue PublicHost(StartupContext context)
    {
        var domain = StellarrVariables.ValueIn(context.Environment, StellarrVariables.Domain);
        return domain is null
            ? new StartupValue("localhost", "entrypoint")
            : new StartupValue(domain, "environment");
    }

    /// <summary>The host port HTTPS is published on, which the entrypoint takes as 443 unless told.</summary>
    internal static StartupValue HttpsPort(StartupContext context)
    {
        var value = StellarrVariables.ValueIn(context.Environment, StellarrVariables.HttpsPort);
        return value is null
            ? new StartupValue("443", "default")
            : new StartupValue(value, "environment");
    }

    /// <summary>
    /// The configured value of <paramref name="key"/>, from the environment
    /// when its <c>STELLARR_*</c> variable or its raw <c>Section__Key</c> is set.
    /// </summary>
    internal static StartupValue ConfigValue(StartupContext context, string key, string fallback)
    {
        var value = context.Configuration[key];
        if (value is null)
        {
            return new StartupValue(fallback, "default");
        }

        var variable = StellarrVariables.ForKey(key);
        var fromEnvironment =
            (variable is not null && StellarrVariables.ValueIn(context.Environment, variable) is not null) ||
            (context.Environment.TryGetValue(key.Replace(":", "__", StringComparison.Ordinal), out var raw) &&
                !string.IsNullOrEmpty(raw));
        return new StartupValue(value, fromEnvironment ? "environment" : "configuration");
    }

    /// <summary>
    /// The named identity providers, by id, each marked when the instance
    /// trusts its own second step. Only ids and that flag: never a secret.
    /// </summary>
    internal static StartupValue OidcProviders(StartupContext context)
    {
        var providers = context.Configuration.GetSection("Oidc:Providers").GetChildren()
            .OrderBy(x => x.Key, StringComparer.Ordinal)
            .Select(x => string.Equals(x["TrustProviderMfa"], "true", StringComparison.OrdinalIgnoreCase)
                ? $"{x.Key} (trusts provider MFA)"
                : x.Key)
            .ToArray();
        if (providers.Length == 0)
        {
            return new StartupValue("(none)", "default");
        }

        var fromEnvironment = context.Environment.Keys.Any(
            name => name.StartsWith("Oidc__Providers__", StringComparison.OrdinalIgnoreCase));
        return new StartupValue(string.Join(", ", providers), fromEnvironment ? "environment" : "configuration");
    }

    private static string PublicUrlFor(string host, string publicPort)
    {
        // The shell wraps any host containing a colon, matching the entrypoint.
        var displayHost = host.Contains(':', StringComparison.Ordinal) ? $"[{host}]" : host;
        var port = int.TryParse(publicPort, NumberStyles.Integer, CultureInfo.InvariantCulture, out var parsed)
            ? parsed
            : 443;
        return port == 443
            ? $"https://{displayHost}"
            : $"https://{displayHost}:{port.ToString(CultureInfo.InvariantCulture)}";
    }
}
