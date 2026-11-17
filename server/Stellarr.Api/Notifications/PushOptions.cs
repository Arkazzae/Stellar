namespace Stellarr.Api.Notifications;

/// <summary>
/// Configuration of Web Push from the environment. The private VAPID key is a
/// secret: it is read here and never written to a log, an endpoint response or
/// an exception message.
/// </summary>
public sealed class PushOptions
{
    public const string SectionName = "Push";

    public bool Enabled { get; set; }

    /// <summary>A <c>mailto:</c> address or an HTTPS URL identifying the operator.</summary>
    public string Subject { get; set; } = "";

    /// <summary>The base64url-encoded uncompressed P-256 public key browsers subscribe with.</summary>
    public string PublicKey { get; set; } = "";

    /// <summary>The base64url-encoded 32-byte P-256 private key.</summary>
    public string PrivateKey { get; set; } = "";
}
