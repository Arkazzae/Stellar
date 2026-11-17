using System.Net;
using System.Security.Cryptography;
using Stellarr.Api.Embeds;

namespace Stellarr.Api.Notifications;

/// <summary>A subscription's fields after the validation the endpoints and the transport share.</summary>
internal sealed record ValidatedPushSubscription(string Endpoint, string P256dh, string Auth);

/// <summary>
/// Checks the values a browser gives for a Push API subscription, and the
/// endpoint before every delivery. Nothing here contacts the endpoint: a
/// hostname that resolves to a private address is refused when the socket is
/// opened, so DNS rebinding cannot turn a public name into an internal one.
/// </summary>
internal static class PushSubscriptionValidation
{
    internal const int EndpointMaxLength = 2_048;
    internal const int P256dhMaxLength = 256;
    internal const int AuthMaxLength = 64;

    private const int UncompressedP256Length = 65;
    private const int AuthSecretLength = 16;

    internal static bool TryValidate(
        string? endpoint,
        string? p256dh,
        string? auth,
        out ValidatedPushSubscription subscription)
    {
        subscription = null!;
        if (!TryNormalizeEndpoint(endpoint, out var normalizedEndpoint) ||
            !TryNormalizeP256dh(p256dh, out var normalizedP256dh) ||
            !TryNormalizeAuth(auth, out var normalizedAuth))
        {
            return false;
        }

        subscription = new ValidatedPushSubscription(normalizedEndpoint, normalizedP256dh, normalizedAuth);
        return true;
    }

    internal static bool TryNormalizeEndpoint(string? value, out string endpoint)
    {
        endpoint = "";
        var candidate = value?.Trim();
        if (candidate is not { Length: > 0 } || candidate.Length > EndpointMaxLength ||
            !Uri.TryCreate(candidate, UriKind.Absolute, out var uri) ||
            uri.Scheme != Uri.UriSchemeHttps ||
            (!uri.IsDefaultPort && uri.Port != 443) ||
            uri.UserInfo.Length != 0 ||
            uri.Fragment.Length != 0 ||
            candidate.Contains('#') ||
            uri.AbsoluteUri.Length > EndpointMaxLength)
        {
            return false;
        }

        var host = uri.Host.TrimEnd('.');
        if (host.Length == 0 ||
            host.Equals("localhost", StringComparison.OrdinalIgnoreCase) ||
            host.EndsWith(".localhost", StringComparison.OrdinalIgnoreCase) ||
            host.EndsWith(".local", StringComparison.OrdinalIgnoreCase) ||
            host.EndsWith(".internal", StringComparison.OrdinalIgnoreCase) ||
            host.EndsWith(".home.arpa", StringComparison.OrdinalIgnoreCase))
        {
            return false;
        }

        if (IPAddress.TryParse(uri.DnsSafeHost, out var address) && !PublicAddress.IsPublic(address))
        {
            return false;
        }

        endpoint = uri.AbsoluteUri;
        return true;
    }

    internal static bool TryNormalizeP256dh(string? value, out string p256dh)
    {
        p256dh = "";
        if (!TryDecodeBase64Url(value, P256dhMaxLength, out var bytes) ||
            bytes.Length != UncompressedP256Length ||
            bytes[0] != 0x04)
        {
            return false;
        }

        try
        {
            using var ecdsa = ECDsa.Create(new ECParameters
            {
                Curve = ECCurve.NamedCurves.nistP256,
                Q = new ECPoint
                {
                    X = bytes.AsSpan(1, 32).ToArray(),
                    Y = bytes.AsSpan(33, 32).ToArray(),
                },
            });
        }
        catch (CryptographicException)
        {
            return false;
        }
        catch (ArgumentException)
        {
            return false;
        }

        p256dh = Base64UrlEncode(bytes);
        return true;
    }

    internal static bool TryNormalizeAuth(string? value, out string auth)
    {
        auth = "";
        if (!TryDecodeBase64Url(value, AuthMaxLength, out var bytes) || bytes.Length != AuthSecretLength)
        {
            return false;
        }

        auth = Base64UrlEncode(bytes);
        return true;
    }

    internal static bool TryDecodeBase64Url(string? value, int maximumLength, out byte[] bytes)
    {
        bytes = [];
        if (value is not { Length: > 0 } text || text.Length > maximumLength || text.Any(char.IsWhiteSpace))
        {
            return false;
        }

        var padding = 0;
        while (padding < text.Length && text[^(padding + 1)] == '=')
        {
            padding++;
        }

        var bodyLength = text.Length - padding;
        if (padding > 2 || bodyLength == 0 || bodyLength % 4 == 1)
        {
            return false;
        }

        var builder = new System.Text.StringBuilder(bodyLength + 4);
        for (var index = 0; index < bodyLength; index++)
        {
            var character = text[index];
            if (character is >= 'A' and <= 'Z' or >= 'a' and <= 'z' or >= '0' and <= '9')
            {
                builder.Append(character);
            }
            else if (character == '-')
            {
                builder.Append('+');
            }
            else if (character == '_')
            {
                builder.Append('/');
            }
            else
            {
                return false;
            }
        }

        while (builder.Length % 4 != 0)
        {
            builder.Append('=');
        }

        try
        {
            bytes = Convert.FromBase64String(builder.ToString());
            return true;
        }
        catch (FormatException)
        {
            return false;
        }
    }

    internal static string Base64UrlEncode(byte[] bytes) =>
        Convert.ToBase64String(bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_');
}
