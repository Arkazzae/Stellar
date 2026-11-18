using Stellarr.Api.Domain;

namespace Stellarr.Api.Contracts;

// Every byte array travels as standard base64 with padding, every id as a
// lowercase UUID with hyphens: the ids are part of what browsers derive keys
// from and authenticate, so the form is fixed.

/// <summary>The PIN-protected copy of the account's private key, as the browser made it.</summary>
public sealed record KeyBackupRequest(
    byte[]? Salt,
    int? Iterations,
    byte[]? Verifier,
    byte[]? Iv,
    byte[]? Ciphertext);

/// <summary>First set-up of encryption for the account: its public key and the backup of its private key.</summary>
public sealed record SetUpAccountKeyRequest(string? Algorithm, byte[]? PublicKey, KeyBackupRequest? Backup);

/// <param name="AuthKey">The authentication key the browser derived from the PIN.</param>
public sealed record UnlockBackupRequest(byte[]? AuthKey);

/// <summary>A new PIN: the same account key, encrypted again by a browser that holds it.</summary>
/// <param name="AccountKeyId">The account key the backup holds, which must still be the account's.</param>
public sealed record ReplaceBackupRequest(string? CurrentPassword, string? AccountKeyId, KeyBackupRequest? Backup);

/// <summary>A new account key and its backup, for when nobody holds the old one any more.</summary>
public sealed record ResetAccountKeyRequest(
    string? CurrentPassword,
    string? Algorithm,
    byte[]? PublicKey,
    KeyBackupRequest? Backup);

/// <param name="Label">The browser and system, such as "Chrome, macOS".</param>
public sealed record RegisterDeviceRequest(string? Algorithm, byte[]? PublicKey, string? Label);

/// <param name="EpochId">Picked by the browser before wrapping, since the key is bound to it.</param>
/// <param name="PreviousEpochId">The conversation's current epoch, or null for the first after switching on.</param>
public sealed record CreateEpochRequest(
    string? EpochId,
    Guid? PreviousEpochId,
    IReadOnlyList<EnvelopeRequest>? Envelopes);

public sealed record EnvelopeRequest(
    string? RecipientKeyId,
    byte[]? EphemeralPublicKey,
    byte[]? Iv,
    byte[]? Ciphertext);

/// <summary>An encrypted message body: the epoch it was encrypted with and the result.</summary>
public sealed record EncryptedPayloadRequest(Guid? EpochId, byte[]? Iv, byte[]? Ciphertext);

public sealed record PublicKeyDto(string KeyId, string Algorithm, byte[] PublicKey, DateTime CreatedAt);

/// <param name="RemainingAttempts">Wrong PINs left before the backup is deleted.</param>
public sealed record KeyBackupStatusDto(int Iterations, int FailedAttempts, int RemainingAttempts, DateTime UpdatedAt);

/// <summary>What a browser needs before it can try a PIN.</summary>
public sealed record KeyBackupParametersDto(byte[] Salt, int Iterations, int RemainingAttempts);

/// <summary>The backup, handed out for a right PIN.</summary>
public sealed record UnlockedBackupDto(string AccountKeyId, byte[] Iv, byte[] Ciphertext);

/// <param name="Current">Whether this is the device of the session asking.</param>
public sealed record OwnDeviceDto(
    string Id,
    string Algorithm,
    byte[] PublicKey,
    string Label,
    DateTime CreatedAt,
    DateTime LastSeenAt,
    bool Current);

/// <summary>Encryption as far as the signed-in account is concerned.</summary>
/// <param name="AccountKey">Null until encryption was set up.</param>
/// <param name="Backup">Null when there is no backup: never made, or deleted after too many wrong PINs.</param>
/// <param name="Device">The device of the session asking, while it has one.</param>
public sealed record EncryptionAccountDto(PublicKeyDto? AccountKey, KeyBackupStatusDto? Backup, OwnDeviceDto? Device);

public sealed record OwnDevicesDto(IReadOnlyList<OwnDeviceDto> Devices);

/// <summary>Answer to a reset.</summary>
/// <param name="RevokedDevices">How many other devices lost their keys.</param>
public sealed record ResetAccountKeyDto(PublicKeyDto AccountKey, KeyBackupStatusDto Backup, int RevokedDevices);

/// <summary>Where a conversation stands with encryption.</summary>
/// <param name="State">"off", "requested" or "on".</param>
/// <param name="RequestedById">Who asked, while it is "requested".</param>
/// <param name="EnabledAt">When it was switched on, while it is "on".</param>
/// <param name="CurrentEpochId">The epoch new messages are encrypted with; null until the first after switching on.</param>
public sealed record ConversationEncryptionDto(
    string State,
    string? RequestedById,
    DateTime? RequestedAt,
    DateTime? EnabledAt,
    string? CurrentEpochId);

/// <summary>A peer's device as the other person sees it: no label, nothing about the session.</summary>
public sealed record DeviceKeyDto(string KeyId, string Algorithm, byte[] PublicKey, DateTime CreatedAt);

/// <param name="AccountKey">Null while the person has not set up encryption.</param>
/// <param name="Devices">The person's active devices, oldest first.</param>
public sealed record PersonKeysDto(string UserId, PublicKeyDto? AccountKey, IReadOnlyList<DeviceKeyDto> Devices);

/// <param name="Kind">"account" or "device".</param>
public sealed record RecipientKeyDto(string KeyId, string Kind, string UserId);

public sealed record EnvelopeDto(string RecipientKeyId, byte[] EphemeralPublicKey, byte[] Iv, byte[] Ciphertext);

/// <param name="Recipients">Exactly the keys the epoch was wrapped for.</param>
/// <param name="Envelopes">Only the envelopes for the reader's own keys.</param>
public sealed record EpochDto(
    string Id,
    int Sequence,
    string? PreviousEpochId,
    string CreatedById,
    DateTime CreatedAt,
    IReadOnlyList<RecipientKeyDto> Recipients,
    IReadOnlyList<EnvelopeDto> Envelopes);

/// <summary>Everything a browser needs to encrypt in a conversation.</summary>
/// <param name="CurrentDeviceKeyId">The device of the session asking, or null while it has none.</param>
/// <param name="Recipients">The keys a new epoch has to be wrapped for, right now.</param>
/// <param name="CurrentEpoch">The epoch to encrypt with, with the envelopes for the reader's keys.</param>
public sealed record ConversationKeysDto(
    string ConversationId,
    ConversationEncryptionDto Encryption,
    PersonKeysDto Self,
    PersonKeysDto Peer,
    string? CurrentDeviceKeyId,
    IReadOnlyList<RecipientKeyDto> Recipients,
    EpochDto? CurrentEpoch);

/// <summary>Epochs of a conversation after a sequence number, oldest first.</summary>
/// <param name="HasMore">Set when there are more; ask again after the last sequence of this page.</param>
public sealed record ConversationEpochsDto(string? CurrentEpochId, IReadOnlyList<EpochDto> Epochs, bool HasMore);

/// <summary>Pushed to both people when a conversation's encryption changes.</summary>
public sealed record DirectEncryptionChangedDto(string ConversationId, ConversationEncryptionDto Encryption);

/// <summary>Pushed to both people for a new epoch, each with the envelopes for their own keys.</summary>
public sealed record DirectEpochCreatedDto(string ConversationId, EpochDto Epoch);

/// <summary>
/// Pushed to both people of an encrypted conversation when one of them gained
/// or lost a device, or reset the account key: a new epoch is due.
/// </summary>
public sealed record DirectKeysChangedDto(string ConversationId, string UserId);

public static class EncryptionContractMappings
{
    public static string ToWire(this DirectEncryptionState state) => state switch
    {
        DirectEncryptionState.Requested => "requested",
        DirectEncryptionState.On => "on",
        _ => "off",
    };

    public static ConversationEncryptionDto ToEncryptionDto(this DirectConversation conversation) => new(
        conversation.EncryptionState.ToWire(),
        conversation.EncryptionRequestedById?.ToString(),
        conversation.EncryptionRequestedAt.AsUtc(),
        conversation.EncryptionEnabledAt.AsUtc(),
        conversation.CurrentEpochId?.ToString());

    public static PublicKeyDto ToDto(this AccountKey key) =>
        new(key.KeyId.ToString(), key.Algorithm, key.PublicKey, key.CreatedAt.AsUtc());

    public static KeyBackupStatusDto ToStatusDto(this AccountKeyBackup backup) => new(
        backup.Iterations,
        backup.FailedAttempts,
        Math.Max(0, AccountKeyBackup.MaxFailedAttempts - backup.FailedAttempts),
        backup.UpdatedAt.AsUtc());

    public static DeviceKeyDto ToKeyDto(this EncryptionDevice device) =>
        new(device.Id.ToString(), device.Algorithm, device.PublicKey, device.CreatedAt.AsUtc());

    /// <param name="sessionLastSeenAt">The device is in use whenever its session is.</param>
    public static OwnDeviceDto ToOwnDto(this EncryptionDevice device, Guid? currentSessionId, DateTime sessionLastSeenAt) => new(
        device.Id.ToString(),
        device.Algorithm,
        device.PublicKey,
        device.Label,
        device.CreatedAt.AsUtc(),
        (sessionLastSeenAt > device.LastSeenAt ? sessionLastSeenAt : device.LastSeenAt).AsUtc(),
        device.SessionId == currentSessionId);

    public static EnvelopeDto ToDto(this EncryptionEnvelope envelope) => new(
        envelope.RecipientKeyId.ToString(),
        envelope.EphemeralPublicKey,
        envelope.Iv,
        envelope.Ciphertext);

    /// <param name="envelopes">The envelopes the reader may have: those for their own keys.</param>
    public static EpochDto ToDto(this EncryptionEpoch epoch, IEnumerable<EncryptionEnvelope> envelopes) => new(
        epoch.Id.ToString(),
        epoch.Sequence,
        epoch.PreviousEpochId?.ToString(),
        epoch.CreatedById.ToString(),
        epoch.CreatedAt.AsUtc(),
        epoch.Envelopes
            .Select(x => new RecipientKeyDto(x.RecipientKeyId.ToString(), x.RecipientKind, x.RecipientUserId.ToString()))
            .OrderBy(x => x.UserId, StringComparer.Ordinal)
            .ThenBy(x => x.Kind, StringComparer.Ordinal)
            .ThenBy(x => x.KeyId, StringComparer.Ordinal)
            .ToArray(),
        envelopes.Select(x => x.ToDto()).ToArray());
}
