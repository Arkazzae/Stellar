namespace Stellarr.Api.Domain;

/// <summary>
/// The public half of a person's account key. The private half never reaches
/// the server in the clear: it lives in <see cref="AccountKeyBackup"/>,
/// encrypted with a key derived from the PIN, and on the browsers that
/// unlocked or created it.
/// </summary>
public sealed class AccountKey
{
    public Guid UserId { get; set; }

    /// <summary>The key id that envelopes name; a reset gives the account a new one.</summary>
    public Guid KeyId { get; set; }

    /// <summary>See <see cref="EncryptionAlgorithms"/>.</summary>
    public required string Algorithm { get; set; }

    public required byte[] PublicKey { get; set; }
    public DateTime CreatedAt { get; set; }

    public User User { get; set; } = null!;
}

/// <summary>
/// The account's private key, encrypted in the browser with a key derived from
/// the PIN. The server only keeps what lets it count attempts: a digest of the
/// key the browser proves the PIN with, compared in constant time.
/// </summary>
public sealed class AccountKeyBackup
{
    /// <summary>Wrong PINs after which the backup is deleted.</summary>
    public const int MaxFailedAttempts = 10;

    public Guid UserId { get; set; }
    public required byte[] Salt { get; set; }
    public int Iterations { get; set; }

    /// <summary>SHA-256 of the authentication key the browser derives from the PIN.</summary>
    public required byte[] Verifier { get; set; }

    public required byte[] Iv { get; set; }
    public required byte[] Ciphertext { get; set; }

    /// <summary>Wrong PINs since the last right one; reset by a right one and by a new PIN.</summary>
    public int FailedAttempts { get; set; }

    public DateTime UpdatedAt { get; set; }

    public User User { get; set; } = null!;
}

/// <summary>
/// A browser's own key pair, registered under the login session it was made
/// in. It takes part in encrypted conversations only while that session is
/// active and the device itself has not been revoked; nothing is stamped on
/// the device when the session ends, see <c>EncryptionDevices.Active</c>.
/// </summary>
public sealed class EncryptionDevice
{
    /// <summary>Also the key id that envelopes name.</summary>
    public Guid Id { get; set; }

    public Guid UserId { get; set; }
    public Guid SessionId { get; set; }

    /// <summary>See <see cref="EncryptionAlgorithms"/>.</summary>
    public required string Algorithm { get; set; }

    public required byte[] PublicKey { get; set; }

    /// <summary>The browser and system summary the client sent, such as "Chrome, macOS".</summary>
    public required string Label { get; set; }

    public DateTime CreatedAt { get; set; }
    public DateTime LastSeenAt { get; set; }

    /// <summary>Set when the owner removed the keys or reset the account key.</summary>
    public DateTime? RevokedAt { get; set; }

    public UserSession Session { get; set; } = null!;
}

/// <summary>Where a private conversation stands with end-to-end encryption.</summary>
public enum DirectEncryptionState
{
    Off,
    Requested,
    On,
}

/// <summary>
/// One key of an encrypted conversation. The browser that made it picked its
/// id, because the id is part of what the key is wrapped and messages are
/// encrypted with. Epochs follow one another in <see cref="Sequence"/>.
/// </summary>
public sealed class EncryptionEpoch
{
    public Guid Id { get; set; }
    public Guid ConversationId { get; set; }

    /// <summary>1 for the first epoch of the conversation, then one more for each.</summary>
    public int Sequence { get; set; }

    /// <summary>The conversation's current epoch when this one was made; null right after switching on.</summary>
    public Guid? PreviousEpochId { get; set; }

    public Guid CreatedById { get; set; }
    public DateTime CreatedAt { get; set; }

    /// <summary>The key wrapped for every recipient it was made for: exactly its recipient set.</summary>
    public ICollection<EncryptionEnvelope> Envelopes { get; set; } = [];
}

/// <summary>The epoch key wrapped for one recipient key, opaque to the server.</summary>
public sealed class EncryptionEnvelope
{
    public Guid EpochId { get; set; }

    /// <summary>A device id or an account key id.</summary>
    public Guid RecipientKeyId { get; set; }

    public Guid RecipientUserId { get; set; }

    /// <summary>"account" or "device".</summary>
    public required string RecipientKind { get; set; }

    public required byte[] EphemeralPublicKey { get; set; }
    public required byte[] Iv { get; set; }
    public required byte[] Ciphertext { get; set; }
}

/// <summary>Timeline lines that a private conversation shows between messages.</summary>
public static class DirectSystemKinds
{
    public const string EncryptionRequested = "encryption_requested";
    public const string EncryptionRequestCancelled = "encryption_request_cancelled";
    public const string EncryptionEnabled = "encryption_enabled";
    public const string EncryptionDisabled = "encryption_disabled";
    public const string DeviceAdded = "device_added";
    public const string KeysReset = "keys_reset";
}

/// <summary>
/// The key agreement algorithms a stored public key may be for, each with the
/// length of its raw public key. Every key carries its tag, so another
/// algorithm is a new row here rather than a schema change.
/// </summary>
public static class EncryptionAlgorithms
{
    public const string X25519 = "x25519";

    private static readonly Dictionary<string, int> PublicKeyLengths = new(StringComparer.Ordinal)
    {
        [X25519] = 32,
    };

    public static bool IsKnown(string? algorithm) => algorithm is not null && PublicKeyLengths.ContainsKey(algorithm);

    /// <summary>Whether <paramref name="publicKey"/> is a raw public key of <paramref name="algorithm"/>.</summary>
    public static bool IsPublicKey(string? algorithm, byte[]? publicKey) =>
        algorithm is not null &&
        publicKey is not null &&
        PublicKeyLengths.TryGetValue(algorithm, out var length) &&
        publicKey.Length == length;
}

/// <summary>
/// The sizes the server accepts for what browsers encrypt. They follow from
/// the protocol (96-bit nonces, a SHA-256 verifier) and leave room around what
/// a browser actually sends, so that a wrong client is refused rather than stored.
/// </summary>
public static class EncryptionLimits
{
    /// <summary>The nonce of every AES-GCM encryption: messages, envelopes and the backup.</summary>
    public const int IvBytes = 12;

    /// <summary>The authentication tag AES-GCM appends to every ciphertext.</summary>
    public const int TagBytes = 16;

    public const int VerifierBytes = 32;
    public const int AuthKeyBytes = 32;
    public const int MinSaltBytes = 16;
    public const int MaxSaltBytes = 64;
    public const int MinIterations = 600_000;
    public const int MaxIterations = 10_000_000;

    /// <summary>A wrapped PKCS#8 private key.</summary>
    public const int MaxBackupCiphertextBytes = 1024;

    /// <summary>A wrapped 256-bit epoch key.</summary>
    public const int MaxEnvelopeCiphertextBytes = 256;

    /// <summary>
    /// An encrypted message: the longest text, escaped as JSON in the worst
    /// case, and the descriptions of its attachments.
    /// </summary>
    public const int MaxMessageCiphertextBytes = 256 * 1024;

    public const int MaxDeviceLabelLength = 100;

    /// <summary>
    /// Browsers of one account that hold keys at once. Every epoch is wrapped
    /// for each of them, so the bound keeps an epoch within reach of a request.
    /// </summary>
    public const int MaxActiveDevices = 100;
}
