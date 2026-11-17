namespace Stellarr.Api.Domain;

/// <summary>
/// A browser Push API subscription owned by one sign-in. The endpoint is the
/// push service's capability URL and the two keys are the browser's public
/// encryption material; none of them is ever written to a log.
/// </summary>
public sealed class PushSubscriptionRecord
{
    public Guid Id { get; set; }
    public Guid UserId { get; set; }

    /// <summary>The sign-in the browser made the subscription from.</summary>
    public Guid SessionId { get; set; }

    /// <summary>The push service endpoint, including its unguessable path.</summary>
    public required string Endpoint { get; set; }

    /// <summary>The browser's uncompressed P-256 public key, base64url encoded.</summary>
    public required string P256dh { get; set; }

    /// <summary>The 16-byte shared authentication secret, base64url encoded.</summary>
    public required string Auth { get; set; }

    public DateTime CreatedAt { get; set; }
}
