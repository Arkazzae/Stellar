using System.Security.Cryptography;
using WebPush;

namespace Stellarr.Api.Notifications;

/// <summary>
/// A validated VAPID key pair. Invalid or incomplete configuration produces an
/// unavailable instance rather than an exception at startup, so an installation
/// with a typo in one environment variable still serves every other feature.
/// </summary>
internal sealed class PushVapidConfiguration
{
    private const int PublicKeyBytes = 65;
    private const int PrivateKeyBytes = 32;

    private static readonly byte[] P256Order =
    [
        0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xbc, 0xe6, 0xfa, 0xad, 0xa7, 0x17, 0x9e, 0x84,
        0xf3, 0xb9, 0xca, 0xc2, 0xfc, 0x63, 0x25, 0x51,
    ];

    private PushVapidConfiguration(string subject, string publicKey, string privateKey)
    {
        Subject = subject;
        PublicKey = publicKey;
        PrivateKey = privateKey;
    }

    public string Subject { get; }
    public string PublicKey { get; }

    private string PrivateKey { get; }

    public VapidDetails Details => new(Subject, PublicKey, PrivateKey);

    public static PushVapidConfiguration? From(PushOptions options)
    {
        if (!options.Enabled ||
            !TryNormalizeSubject(options.Subject, out var subject) ||
            !TryNormalizePublicKey(options.PublicKey, out var publicKey) ||
            !TryNormalizePrivateKey(options.PrivateKey, out var privateKey))
        {
            return null;
        }

        return Matches(publicKey, privateKey)
            ? new PushVapidConfiguration(subject, publicKey, privateKey)
            : null;
    }

    private static bool TryNormalizeSubject(string? value, out string subject)
    {
        subject = "";
        var candidate = value?.Trim();
        if (candidate is not { Length: > 0 and <= 512 } ||
            !Uri.TryCreate(candidate, UriKind.Absolute, out var uri) ||
            uri.Scheme is not ("mailto" or "https") ||
            candidate.Any(char.IsWhiteSpace) ||
            (uri.Scheme == "mailto" && candidate["mailto:".Length..].Trim().Length == 0) ||
            (uri.Scheme == "https" && uri.Host.Length == 0))
        {
            return false;
        }

        subject = candidate;
        return true;
    }

    private static bool TryNormalizePublicKey(string? value, out string publicKey)
    {
        publicKey = "";
        if (!PushSubscriptionValidation.TryDecodeBase64Url(value, 128, out var bytes) ||
            bytes.Length != PublicKeyBytes ||
            bytes[0] != 0x04 ||
            !IsValidPoint(bytes.AsSpan(1, 32), bytes.AsSpan(33, 32)))
        {
            return false;
        }

        publicKey = PushSubscriptionValidation.Base64UrlEncode(bytes);
        return true;
    }

    private static bool TryNormalizePrivateKey(string? value, out string privateKey)
    {
        privateKey = "";
        if (!PushSubscriptionValidation.TryDecodeBase64Url(value, 64, out var bytes) ||
            bytes.Length != PrivateKeyBytes ||
            bytes.AsSpan().IndexOfAnyExcept((byte)0) < 0 ||
            !IsLessThanOrder(bytes))
        {
            return false;
        }

        try
        {
            using var ecdsa = ECDsa.Create(new ECParameters
            {
                Curve = ECCurve.NamedCurves.nistP256,
                D = bytes,
            });
            _ = ecdsa.ExportParameters(false).Q;
        }
        catch (CryptographicException)
        {
            return false;
        }
        catch (ArgumentException)
        {
            return false;
        }

        privateKey = PushSubscriptionValidation.Base64UrlEncode(bytes);
        return true;
    }

    private static bool Matches(string publicKey, string privateKey)
    {
        if (!PushSubscriptionValidation.TryDecodeBase64Url(publicKey, 128, out var publicBytes) ||
            !PushSubscriptionValidation.TryDecodeBase64Url(privateKey, 64, out var privateBytes))
        {
            return false;
        }

        try
        {
            using var ecdsa = ECDsa.Create(new ECParameters
            {
                Curve = ECCurve.NamedCurves.nistP256,
                D = privateBytes,
            });
            var q = ecdsa.ExportParameters(false).Q;
            return q.X is { Length: 32 } x &&
                q.Y is { Length: 32 } y &&
                x.AsSpan().SequenceEqual(publicBytes.AsSpan(1, 32)) &&
                y.AsSpan().SequenceEqual(publicBytes.AsSpan(33, 32));
        }
        catch (CryptographicException)
        {
            return false;
        }
        catch (ArgumentException)
        {
            return false;
        }
    }

    private static bool IsValidPoint(ReadOnlySpan<byte> x, ReadOnlySpan<byte> y)
    {
        try
        {
            using var ecdsa = ECDsa.Create(new ECParameters
            {
                Curve = ECCurve.NamedCurves.nistP256,
                Q = new ECPoint { X = x.ToArray(), Y = y.ToArray() },
            });
            return true;
        }
        catch (CryptographicException)
        {
            return false;
        }
        catch (ArgumentException)
        {
            return false;
        }
    }

    private static bool IsLessThanOrder(ReadOnlySpan<byte> value)
    {
        for (var index = 0; index < value.Length; index++)
        {
            if (value[index] != P256Order[index])
            {
                return value[index] < P256Order[index];
            }
        }

        return false;
    }
}
