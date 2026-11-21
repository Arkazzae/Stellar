using Microsoft.Extensions.Configuration;
using Stellarr.Api.Operations;

namespace Stellarr.Cli;

internal sealed record StartupContext(
    IConfiguration Configuration,
    IReadOnlyDictionary<string, string?> Environment);

/// <summary>A read-only startup setting with the source of the value we print.</summary>
internal sealed record StartupValue(string Value, string Source);

/// <summary>
/// Reads the startup settings as the CLI reports them. <c>docker exec</c>
/// starts the CLI with the container's configured environment, not with what
/// the entrypoint exported. The image's wrapper (<c>infra/stellarr-cli.sh</c>)
/// adds the public address, derived by the entrypoint's own rules, under its
/// configuration key; the rest is the operator's variables, or the image's
/// default when one is not set.
/// </summary>
internal static class ContainerSettings
{
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
}
