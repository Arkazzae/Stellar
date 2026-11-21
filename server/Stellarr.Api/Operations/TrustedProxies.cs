using System.Diagnostics.CodeAnalysis;
using System.Globalization;
using System.Net;
using System.Net.Sockets;
using Microsoft.AspNetCore.HttpOverrides;
using IPNetwork = System.Net.IPNetwork;

namespace Stellarr.Api.Operations;

/// <summary>
/// The reverse proxies the operator runs in front of the container, listed in
/// <c>Operations:TrustedProxies</c> (<see cref="StellarrVariables.TrustedProxies"/>): those
/// whose word the API takes for the address of the person behind a request.
/// </summary>
/// <remarks>
/// <para>
/// The API listens on loopback behind the container's nginx, which appends the
/// address of whoever connected to it to X-Forwarded-For. With nothing listed
/// that last entry is the address: the client's own, or the proxy's when one
/// sits in front.
/// </para>
/// <para>
/// With proxies listed, the header is read from the right, one entry for each
/// hop that is trusted. nginx's entry, believed because nginx connects from
/// loopback, names who connected to the container; when that is a listed
/// proxy, the entry before it names who connected to that proxy, and so on,
/// until an address that is not a listed proxy: the client. Past nginx,
/// loopback is an address like any other.
/// What a client writes into the header itself stays to the left of the entry
/// its proxy appended, so it is never reached, and a client that reaches the
/// container without going through a listed proxy is itself the first address
/// that ends the walk.
/// </para>
/// <para>
/// The rate limits, the session records and the hub connections all take the
/// address the walk ends at, from <c>Connection.RemoteIpAddress</c>.
/// </para>
/// </remarks>
public sealed class TrustedProxies
{
    public const string SettingName = "Operations:TrustedProxies";
    public static string VariableName => StellarrVariables.TrustedProxies.Name;

    // Commas and any whitespace separate entries, so a list written one entry
    // per line or with spaces after the commas reads the same.
    private static readonly char[] Separators = [',', ' ', '\t', '\r', '\n'];

    private TrustedProxies(IReadOnlyList<IPNetwork> networks) => Networks = networks;

    /// <summary>Each listed proxy; a single address is a network of one.</summary>
    public IReadOnlyList<IPNetwork> Networks { get; }

    /// <summary>Reads <see cref="SettingName"/>.</summary>
    /// <exception cref="InvalidOperationException">The setting lists an entry the API cannot use, named in the message.</exception>
    public static TrustedProxies Load(IConfiguration configuration) => Parse(configuration[SettingName]);

    /// <summary>
    /// Parses a list of IPv4 and IPv6 addresses and CIDR networks, separated
    /// by commas or whitespace. Blank means none.
    /// </summary>
    /// <exception cref="InvalidOperationException">An entry is not an address or a network, named in the message.</exception>
    public static TrustedProxies Parse(string? value)
    {
        var networks = new List<IPNetwork>();
        foreach (var entry in (value ?? "").Split(Separators, StringSplitOptions.RemoveEmptyEntries))
        {
            networks.Add(ParseEntry(entry));
        }

        return new TrustedProxies(networks);
    }

    /// <summary>
    /// X-Forwarded-Proto and X-Forwarded-Host, believed one hop deep from
    /// whoever connects to the API, which only the container's nginx can. The
    /// list of proxies plays no part in them.
    /// </summary>
    internal static ForwardedHeadersOptions SchemeAndHostOptions()
    {
        var options = new ForwardedHeadersOptions
        {
            ForwardedHeaders = ForwardedHeaders.XForwardedProto | ForwardedHeaders.XForwardedHost,
            ForwardLimit = 1,
        };
        options.KnownIPNetworks.Clear();
        options.KnownProxies.Clear();
        return options;
    }

    /// <summary>
    /// The last X-Forwarded-For entry, the one the container's nginx appended:
    /// who connected to the container. With nothing listed it is believed from
    /// whoever connects to the API, as the scheme and host are; with proxies
    /// listed only from loopback, where nginx is.
    /// </summary>
    internal ForwardedHeadersOptions ContainerPeerOptions()
    {
        var options = new ForwardedHeadersOptions
        {
            ForwardedHeaders = ForwardedHeaders.XForwardedFor,
            ForwardLimit = 1,
        };
        options.KnownIPNetworks.Clear();
        options.KnownProxies.Clear();
        if (Networks.Count > 0)
        {
            options.KnownIPNetworks.Add(new IPNetwork(IPAddress.Loopback, 8));
            options.KnownIPNetworks.Add(new IPNetwork(IPAddress.IPv6Loopback, 128));
        }

        return options;
    }

    /// <summary>
    /// The entries before it, read while the address reached is a listed
    /// proxy; null when none is listed. Loopback counts only when it is
    /// listed itself, so otherwise an entry naming loopback ends the walk like
    /// any other address that is not listed.
    /// </summary>
    internal ForwardedHeadersOptions? ListedProxyOptions()
    {
        if (Networks.Count == 0)
        {
            return null;
        }

        var options = new ForwardedHeadersOptions
        {
            ForwardedHeaders = ForwardedHeaders.XForwardedFor,
            ForwardLimit = null,
        };
        options.KnownIPNetworks.Clear();
        options.KnownProxies.Clear();
        foreach (var network in Networks)
        {
            options.KnownIPNetworks.Add(network);
        }

        return options;
    }

    private static IPNetwork ParseEntry(string entry)
    {
        var slash = entry.IndexOf('/', StringComparison.Ordinal);
        if (!TryParseAddress(slash < 0 ? entry : entry[..slash], out var address))
        {
            throw Invalid(entry, "is not an IP address or a network such as 172.18.0.0/16");
        }

        var width = address.AddressFamily == AddressFamily.InterNetwork ? 32 : 128;
        var prefix = width;
        if (slash >= 0)
        {
            var digits = entry[(slash + 1)..];
            if (digits.Length is 0 or > 3 ||
                !digits.All(char.IsAsciiDigit) ||
                !int.TryParse(digits, NumberStyles.None, CultureInfo.InvariantCulture, out prefix) ||
                prefix > width)
            {
                throw Invalid(
                    entry,
                    "has a prefix length that is not a number from 1 to " + width.ToString(CultureInfo.InvariantCulture));
            }

            if (prefix == 0)
            {
                throw Invalid(entry, "covers every address, so anyone could choose the address the API records");
            }
        }

        var network = Masked(address, prefix);
        if (!network.Equals(address))
        {
            throw Invalid(
                entry,
                "has bits set past its prefix length; the network is " +
                network + "/" + prefix.ToString(CultureInfo.InvariantCulture));
        }

        return new IPNetwork(address, prefix);
    }

    /// <summary>
    /// Only the plain forms: four decimal numbers, or IPv6 without brackets,
    /// port or zone. <see cref="IPAddress.TryParse(string, out IPAddress)"/>
    /// also reads shorthand such as "10.1" for 10.0.0.1, which would turn a
    /// typo into some other address.
    /// </summary>
    private static bool TryParseAddress(string text, [NotNullWhen(true)] out IPAddress? address)
    {
        address = null;
        if (text.Contains(':', StringComparison.Ordinal))
        {
            return text.All(c => char.IsAsciiHexDigit(c) || c is ':' or '.') &&
                IPAddress.TryParse(text, out address) &&
                address.AddressFamily == AddressFamily.InterNetworkV6;
        }

        var parts = text.Split('.');
        if (parts.Length != 4)
        {
            return false;
        }

        var bytes = new byte[4];
        for (var i = 0; i < parts.Length; i++)
        {
            if (parts[i].Length is 0 or > 3 ||
                !parts[i].All(char.IsAsciiDigit) ||
                !byte.TryParse(parts[i], NumberStyles.None, CultureInfo.InvariantCulture, out bytes[i]))
            {
                return false;
            }
        }

        address = new IPAddress(bytes);
        return true;
    }

    private static IPAddress Masked(IPAddress address, int prefix)
    {
        var bytes = address.GetAddressBytes();
        for (var i = 0; i < bytes.Length; i++)
        {
            var kept = Math.Clamp(prefix - i * 8, 0, 8);
            bytes[i] &= (byte)(0xFF << (8 - kept));
        }

        return new IPAddress(bytes);
    }

    private static InvalidOperationException Invalid(string entry, string problem) =>
        new($"{SettingName} ({VariableName}) lists \"{entry}\", which {problem}.");
}

/// <summary>Adds the reading of the X-Forwarded-* headers to the pipeline.</summary>
public static class TrustedProxiesExtensions
{
    /// <summary>
    /// Takes the scheme and host the container's nginx reports, then the
    /// address that connected to the container, then the address each listed
    /// proxy reports in turn. Each is a pass of its own, so that the list
    /// decides the address and nothing else, and loopback is believed as the
    /// first hop only.
    /// </summary>
    public static IApplicationBuilder UseForwardedHeaders(this IApplicationBuilder app, TrustedProxies trustedProxies)
    {
        app.UseForwardedHeaders(TrustedProxies.SchemeAndHostOptions());
        app.UseForwardedHeaders(trustedProxies.ContainerPeerOptions());
        if (trustedProxies.ListedProxyOptions() is { } listed)
        {
            app.UseForwardedHeaders(listed);
        }

        return app;
    }
}
