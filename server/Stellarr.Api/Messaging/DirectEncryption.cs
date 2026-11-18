using Microsoft.AspNetCore.SignalR;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;
using Stellarr.Api.Realtime;

namespace Stellarr.Api.Messaging;

/// <summary>A key an epoch has to be wrapped for: a device or an account key of one of the two people.</summary>
internal sealed record EncryptionRecipient(Guid KeyId, string Kind, Guid UserId, string Algorithm);

/// <summary>
/// What the encryption of private conversations shares between the account,
/// conversation and message routes: which devices count, which keys an epoch
/// must reach, and the timeline lines that tell both people what changed.
/// </summary>
internal static class DirectEncryption
{
    internal const string AccountKind = "account";
    internal const string DeviceKind = "device";

    /// <summary>
    /// Devices that take part in encryption: not revoked, and bound to a
    /// session that still signs its browser in, by the same rules as the
    /// session cookie. A sign-out, a revoked or expired session, a password
    /// change and a disabled account all end a device without a write here.
    /// </summary>
    internal static IQueryable<EncryptionDevice> Active(this IQueryable<EncryptionDevice> devices, DateTime now) =>
        devices.Where(device =>
            device.RevokedAt == null &&
            device.Session.RevokedAt == null &&
            device.Session.ExpiresAt > now &&
            device.Session.AbsoluteExpiresAt != null &&
            device.Session.AbsoluteExpiresAt > now &&
            device.Session.SecurityVersion == device.Session.User.SessionVersion &&
            !device.Session.User.IsDisabled);

    /// <summary>The active device of a login session, if it registered one.</summary>
    internal static Task<EncryptionDevice?> CurrentDeviceAsync(
        AppDbContext db,
        Guid? sessionId,
        DateTime now,
        CancellationToken cancellationToken) =>
        sessionId is { } id
            ? db.EncryptionDevices.AsNoTracking().Active(now).SingleOrDefaultAsync(x => x.SessionId == id, cancellationToken)
            : Task.FromResult<EncryptionDevice?>(null);

    /// <summary>
    /// Exactly the keys a new epoch of a conversation between the two people
    /// must be wrapped for: both account keys and every active device of both.
    /// </summary>
    internal static async Task<IReadOnlyList<EncryptionRecipient>> RecipientsAsync(
        AppDbContext db,
        Guid firstUserId,
        Guid secondUserId,
        DateTime now,
        CancellationToken cancellationToken)
    {
        var accounts = await db.AccountKeys.AsNoTracking()
            .Where(x => x.UserId == firstUserId || x.UserId == secondUserId)
            .Select(x => new EncryptionRecipient(x.KeyId, AccountKind, x.UserId, x.Algorithm))
            .ToListAsync(cancellationToken);
        var devices = await db.EncryptionDevices.AsNoTracking()
            .Active(now)
            .Where(x => x.UserId == firstUserId || x.UserId == secondUserId)
            .Select(x => new EncryptionRecipient(x.Id, DeviceKind, x.UserId, x.Algorithm))
            .ToListAsync(cancellationToken);
        return [.. accounts, .. devices];
    }

    /// <summary>The keys of a set that are missing from or extra to the keys that were wrapped for.</summary>
    internal static (Guid[] Missing, Guid[] Extra) Compare(
        IReadOnlyCollection<EncryptionRecipient> required,
        IReadOnlyCollection<Guid> wrapped)
    {
        var requiredIds = required.Select(x => x.KeyId).ToHashSet();
        return (
            [.. requiredIds.Where(id => !wrapped.Contains(id)).Order()],
            [.. wrapped.Where(id => !requiredIds.Contains(id)).Distinct().Order()]);
    }

    /// <summary>The conversations with encryption on that include the person.</summary>
    internal static Task<List<DirectConversation>> EncryptedConversationsOfAsync(
        AppDbContext db,
        Guid userId,
        CancellationToken cancellationToken) =>
        db.DirectConversations.AsNoTracking()
            .Where(x => (x.UserAId == userId || x.UserBId == userId) && x.EncryptionState == DirectEncryptionState.On)
            .ToListAsync(cancellationToken);

    /// <summary>Stages a timeline line by <paramref name="actor"/>; the caller saves it.</summary>
    internal static DirectMessage AddSystemEntry(
        AppDbContext db,
        Guid conversationId,
        User actor,
        string kind,
        DateTime createdAt)
    {
        var entry = new DirectMessage
        {
            Id = Guid.NewGuid(),
            ConversationId = conversationId,
            AuthorId = actor.Id,
            Author = actor,
            Content = "",
            CreatedAt = createdAt,
            SystemKind = kind,
        };
        db.DirectMessages.Add(entry);
        return entry;
    }

    /// <summary>
    /// Tells both people of each conversation about lines that were saved, and
    /// that the actor's keys changed, so that the next message starts a new epoch.
    /// A conversation closed by a block hears nothing: the line is in its
    /// history for when the block is lifted, but a new sign-in is not news to
    /// somebody who was blocked or who blocked the person.
    /// </summary>
    /// <returns>The other people who were told.</returns>
    internal static async Task<Guid[]> PublishKeyChangesAsync(
        IHubContext<AppHub> hub,
        AppDbContext db,
        IEnumerable<(DirectConversation Conversation, DirectMessage? Entry)> changes,
        Guid userId)
    {
        var told = new List<Guid>();
        foreach (var (conversation, entry) in changes)
        {
            var peerId = conversation.UserAId == userId ? conversation.UserBId : conversation.UserAId;
            if (await db.UserBlocks.AsNoTracking().AnyAsync(
                    x => (x.BlockerId == userId && x.BlockedId == peerId) || (x.BlockerId == peerId && x.BlockedId == userId),
                    CancellationToken.None))
            {
                continue;
            }

            told.Add(peerId);
            var people = new[] { AppHub.UserGroup(conversation.UserAId), AppHub.UserGroup(conversation.UserBId) };
            if (entry is not null)
            {
                await hub.Clients.Groups(people).SendAsync("DirectMessageCreated", entry.ToDto(), CancellationToken.None);
            }

            await hub.Clients.Groups(people).SendAsync(
                "DirectEncryptionKeysChanged",
                new DirectKeysChangedDto(conversation.Id.ToString(), userId.ToString()),
                CancellationToken.None);
        }

        return [.. told];
    }

    /// <summary>Tells the account's own browsers to read their encryption state again.</summary>
    internal static Task PublishAccountChangedAsync(IHubContext<AppHub> hub, Guid userId) =>
        hub.Clients.Group(AppHub.UserGroup(userId))
            .SendAsync("EncryptionAccountChanged", new { }, CancellationToken.None);

    /// <summary>
    /// A refusal the client acts on by its <paramref name="code"/>, with a
    /// title it may show, and anything else it needs to recover.
    /// </summary>
    internal static IResult Problem(
        Messages messages,
        int statusCode,
        string code,
        MessageKey title,
        IDictionary<string, object?>? extensions = null)
    {
        var all = new Dictionary<string, object?>(extensions ?? new Dictionary<string, object?>()) { ["code"] = code };
        return Results.Problem(statusCode: statusCode, title: messages.Get(title), extensions: all);
    }
}

/// <summary>The problem codes of encryption refusals, which clients branch on.</summary>
internal static class EncryptionProblemCodes
{
    internal const string AccountKeyMissing = "account-key-missing";
    internal const string AccountKeyExists = "account-key-exists";
    internal const string AccountKeyChanged = "account-key-changed";
    internal const string BackupMissing = "backup-missing";
    internal const string BackupDeleted = "backup-deleted";
    internal const string PinIncorrect = "pin-incorrect";
    internal const string PasswordIncorrect = "password-incorrect";
    internal const string DeviceMissing = "device-missing";
    internal const string DeviceLimit = "device-limit";
    internal const string CurrentDevice = "current-device";
    internal const string StateConflict = "encryption-state";
    internal const string NotAllowed = "encryption-not-allowed";
    internal const string Required = "encryption-required";
    internal const string Off = "encryption-off";
    internal const string EpochStale = "epoch-stale";
    internal const string EpochCoverage = "epoch-coverage";
    internal const string EpochExists = "epoch-exists";
    internal const string EpochNotCurrent = "epoch-not-current";
    internal const string EpochOutdated = "epoch-outdated";
    internal const string AttachmentKind = "attachment-kind-mismatch";
}
