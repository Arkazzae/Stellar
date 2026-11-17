using System.Security.Cryptography;

namespace Stellarr.Api.Tests;

/// <summary>Key material for Web Push tests, generated locally and never sent anywhere.</summary>
internal static class PushTestKeys
{
    internal static (string PublicKey, string PrivateKey) Vapid()
    {
        using var ecdsa = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        var parameters = ecdsa.ExportParameters(includePrivateParameters: true);
        return (
            Base64Url(UncompressedPoint(parameters.Q.X!, parameters.Q.Y!)),
            Base64Url(PadLeft(parameters.D!, 32)));
    }

    internal static (string P256dh, string Auth, byte[] AuthBytes) Subscription()
    {
        using var ecdsa = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        var parameters = ecdsa.ExportParameters(includePrivateParameters: true);
        var auth = RandomNumberGenerator.GetBytes(16);
        return (
            Base64Url(UncompressedPoint(parameters.Q.X!, parameters.Q.Y!)),
            Base64Url(auth),
            auth);
    }

    internal static string Base64Url(byte[] bytes) =>
        Convert.ToBase64String(bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_');

    private static byte[] UncompressedPoint(byte[] x, byte[] y)
    {
        var point = new byte[65];
        point[0] = 0x04;
        x.CopyTo(point, 1);
        y.CopyTo(point, 33);
        return point;
    }

    private static byte[] PadLeft(byte[] value, int length)
    {
        if (value.Length == length)
        {
            return value;
        }

        var padded = new byte[length];
        value.CopyTo(padded, length - value.Length);
        return padded;
    }
}
