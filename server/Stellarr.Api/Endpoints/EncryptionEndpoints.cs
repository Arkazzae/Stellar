using System.Security.Claims;
using System.Security.Cryptography;
using Microsoft.AspNetCore.SignalR;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Auth.Security;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;
using Stellarr.Api.Messaging;
using Stellarr.Api.Realtime;
using Codes = Stellarr.Api.Messaging.EncryptionProblemCodes;

namespace Stellarr.Api.Endpoints;

/// <summary>
/// The account's side of end-to-end encryption: its key, the PIN-protected
/// backup of that key and the browsers that hold keys. Browsers do all the
/// cryptography; the server keeps public keys and opaque bytes, and counts
/// PIN attempts.
/// </summary>
public static class EncryptionEndpoints
{
    public static IEndpointRouteBuilder MapEncryptionEndpoints(this IEndpointRouteBuilder endpoints)
    {
        var encryption = endpoints.MapGroup("/api/me/encryption").RequireAuthorization();
        encryption.MapGet("", GetAsync);
        encryption.MapPost("/account-key", SetUpAsync).RequireRateLimiting(RateLimitPolicies.SecurityWrite);
        encryption.MapPost("/account-key/reset", ResetAsync)
            .RequireRateLimiting(RateLimitPolicies.Login)
            .AddEndpointFilter<FreshSecondStepFilter>();
        encryption.MapGet("/backup", GetBackupAsync);
        encryption.MapPut("/backup", ReplaceBackupAsync)
            .RequireRateLimiting(RateLimitPolicies.Login)
            .AddEndpointFilter<FreshSecondStepFilter>();
        encryption.MapPost("/backup/unlock", UnlockAsync)
            .RequireRateLimiting(RateLimitPolicies.PinUnlock)
            .AddEndpointFilter<AccountAttemptLimitFilter>();
        encryption.MapGet("/devices", ListDevicesAsync);
        encryption.MapPost("/devices", RegisterDeviceAsync);
        encryption.MapDelete("/devices/{deviceId:guid}", RevokeDeviceAsync);
        return endpoints;
    }

    private static async Task<IResult> GetAsync(
        ClaimsPrincipal principal,
        AppDbContext db,
        CancellationToken cancellationToken)
    {
        var userId = principal.RequireUserId();
        var key = await db.AccountKeys.AsNoTracking().SingleOrDefaultAsync(x => x.UserId == userId, cancellationToken);
        var backup = await db.AccountKeyBackups.AsNoTracking().SingleOrDefaultAsync(x => x.UserId == userId, cancellationToken);
        var sessionId = principal.FindSessionId();
        var device = (await OwnDevicesAsync(db, userId, sessionId, cancellationToken)).SingleOrDefault(x => x.Current);
        return Results.Ok(new EncryptionAccountDto(key?.ToDto(), backup?.ToStatusDto(), device));
    }

    /// <summary>
    /// First set-up: the account's public key and the backup of its private
    /// key, together. An account has one key; a forgotten PIN is a reset.
    /// </summary>
    private static async Task<IResult> SetUpAsync(
        SetUpAccountKeyRequest request,
        ClaimsPrincipal principal,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        if ((InvalidKey(request.Algorithm, request.PublicKey, messages) ?? InvalidBackup(request.Backup, messages)) is { } invalid)
        {
            return invalid;
        }

        var userId = principal.RequireUserId();
        if (await db.AccountKeys.AnyAsync(x => x.UserId == userId, cancellationToken))
        {
            return AccountKeyExists(messages);
        }

        var now = DateTime.UtcNow;
        var key = new AccountKey
        {
            UserId = userId,
            KeyId = Guid.NewGuid(),
            Algorithm = request.Algorithm!,
            PublicKey = request.PublicKey!,
            CreatedAt = now,
        };
        db.AccountKeys.Add(key);
        var backup = NewBackup(userId, request.Backup!, now);
        db.AccountKeyBackups.Add(backup);
        if (!await db.TrySaveChangesAsync(cancellationToken))
        {
            return AccountKeyExists(messages);
        }

        await DirectEncryption.PublishAccountChangedAsync(hub, userId);
        return Results.Created("/api/me/encryption", new EncryptionAccountDto(key.ToDto(), backup.ToStatusDto(), null));
    }

    /// <summary>What a browser derives the PIN's keys with, before it tries one.</summary>
    private static async Task<IResult> GetBackupAsync(
        ClaimsPrincipal principal,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var userId = principal.RequireUserId();
        var backup = await db.AccountKeyBackups.AsNoTracking().SingleOrDefaultAsync(x => x.UserId == userId, cancellationToken);
        return backup is null
            ? BackupMissing(messages)
            : Results.Ok(new KeyBackupParametersDto(
                backup.Salt,
                backup.Iterations,
                backup.ToStatusDto().RemainingAttempts));
    }

    /// <summary>
    /// One PIN attempt. A right PIN hands out the backup and clears the count
    /// of wrong ones; the tenth wrong one in a row deletes the backup. The
    /// account key stays, so browsers that hold it keep working and can store
    /// a backup under a new PIN.
    /// </summary>
    private static async Task<IResult> UnlockAsync(
        UnlockBackupRequest request,
        ClaimsPrincipal principal,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        if (request.AuthKey is not { Length: EncryptionLimits.AuthKeyBytes })
        {
            return Invalid("authKey", messages.Get(MessageKey.EncryptionAuthKeyInvalid));
        }

        var userId = principal.RequireUserId();
        var digest = SHA256.HashData(request.AuthKey);

        // The comparison and the count share one write transaction, so that
        // attempts sent at once are counted one after another.
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        var backup = await db.AccountKeyBackups.AsNoTracking().SingleOrDefaultAsync(x => x.UserId == userId, cancellationToken);
        if (backup is null)
        {
            return BackupMissing(messages);
        }

        var backups = db.AccountKeyBackups.Where(x => x.UserId == userId);
        if (CryptographicOperations.FixedTimeEquals(digest, backup.Verifier))
        {
            var keyId = await db.AccountKeys.AsNoTracking()
                .Where(x => x.UserId == userId)
                .Select(x => x.KeyId)
                .SingleAsync(cancellationToken);
            await backups.ExecuteUpdateAsync(setters => setters.SetProperty(x => x.FailedAttempts, 0), cancellationToken);
            await transaction.CommitAsync(cancellationToken);
            return Results.Ok(new UnlockedBackupDto(keyId.ToString(), backup.Iv, backup.Ciphertext));
        }

        await backups.ExecuteUpdateAsync(
            setters => setters.SetProperty(x => x.FailedAttempts, x => x.FailedAttempts + 1),
            cancellationToken);
        var failed = await backups.Select(x => x.FailedAttempts).SingleAsync(cancellationToken);
        var remaining = Math.Max(0, AccountKeyBackup.MaxFailedAttempts - failed);
        if (remaining == 0)
        {
            await backups.ExecuteDeleteAsync(cancellationToken);
        }

        await transaction.CommitAsync(cancellationToken);
        return remaining == 0
            ? DirectEncryption.Problem(
                messages,
                StatusCodes.Status410Gone,
                Codes.BackupDeleted,
                MessageKey.EncryptionBackupDeleted,
                new Dictionary<string, object?> { ["remainingAttempts"] = 0 })
            : DirectEncryption.Problem(
                messages,
                StatusCodes.Status403Forbidden,
                Codes.PinIncorrect,
                MessageKey.EncryptionPinIncorrect,
                new Dictionary<string, object?> { ["remainingAttempts"] = remaining });
    }

    /// <summary>
    /// A new PIN: a browser that holds the account key encrypts it again. It
    /// needs the account password and, where the account has one, a fresh
    /// second step, like a password change; it does not need the old PIN, so
    /// it also restores a backup that wrong PINs deleted.
    /// </summary>
    private static async Task<IResult> ReplaceBackupAsync(
        ReplaceBackupRequest request,
        ClaimsPrincipal principal,
        AppDbContext db,
        PasswordService passwords,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        if (InvalidBackup(request.Backup, messages) is { } invalid)
        {
            return invalid;
        }

        var user = await db.Users.AsNoTracking().SingleAsync(x => x.Id == principal.RequireUserId(), cancellationToken);
        if (request.CurrentPassword is null || !passwords.Verify(request.CurrentPassword, user.PasswordHash))
        {
            return PasswordIncorrect(messages);
        }

        var key = await db.AccountKeys.AsNoTracking().SingleOrDefaultAsync(x => x.UserId == user.Id, cancellationToken);
        if (key is null)
        {
            return AccountKeyMissing(messages);
        }

        // The server cannot see what the backup holds; it can at least refuse a
        // browser that encrypted a key the account no longer has, or that never
        // registered a device and so cannot have unlocked or made the key.
        if (!string.Equals(request.AccountKeyId, key.KeyId.ToString(), StringComparison.Ordinal))
        {
            return DirectEncryption.Problem(
                messages,
                StatusCodes.Status409Conflict,
                Codes.AccountKeyChanged,
                MessageKey.EncryptionAccountKeyChanged,
                new Dictionary<string, object?> { ["accountKeyId"] = key.KeyId.ToString() });
        }

        var now = DateTime.UtcNow;
        if (await DirectEncryption.CurrentDeviceAsync(db, principal.FindSessionId(), now, cancellationToken) is null)
        {
            return DeviceMissing(messages);
        }

        await using (var transaction = await db.Database.BeginTransactionAsync(cancellationToken))
        {
            await db.AccountKeyBackups.Where(x => x.UserId == user.Id).ExecuteDeleteAsync(cancellationToken);
            db.AccountKeyBackups.Add(NewBackup(user.Id, request.Backup!, now));
            await db.SaveChangesAsync(cancellationToken);
            await transaction.CommitAsync(cancellationToken);
        }

        var backup = await db.AccountKeyBackups.AsNoTracking().SingleAsync(x => x.UserId == user.Id, cancellationToken);
        await DirectEncryption.PublishAccountChangedAsync(hub, user.Id);
        return Results.Ok(backup.ToStatusDto());
    }

    /// <summary>
    /// For a forgotten PIN that no browser can stand in for: a new account key
    /// and its backup replace the old ones. Everything encrypted for the old
    /// key is lost. Every other device loses its keys, and the people in the
    /// account's encrypted conversations see that the keys changed.
    /// </summary>
    private static async Task<IResult> ResetAsync(
        ResetAccountKeyRequest request,
        ClaimsPrincipal principal,
        AppDbContext db,
        PasswordService passwords,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        if ((InvalidKey(request.Algorithm, request.PublicKey, messages) ?? InvalidBackup(request.Backup, messages)) is { } invalid)
        {
            return invalid;
        }

        var user = await db.Users.SingleAsync(x => x.Id == principal.RequireUserId(), cancellationToken);
        if (request.CurrentPassword is null || !passwords.Verify(request.CurrentPassword, user.PasswordHash))
        {
            return PasswordIncorrect(messages);
        }

        if (!await db.AccountKeys.AnyAsync(x => x.UserId == user.Id, cancellationToken))
        {
            return AccountKeyMissing(messages);
        }

        var now = DateTime.UtcNow;
        var sessionId = principal.FindSessionId();
        var keyId = Guid.NewGuid();
        int revoked;
        List<(DirectConversation Conversation, DirectMessage? Entry)> changes;
        await using (var transaction = await db.Database.BeginTransactionAsync(cancellationToken))
        {
            await db.AccountKeys.Where(x => x.UserId == user.Id).ExecuteUpdateAsync(
                setters => setters
                    .SetProperty(x => x.KeyId, keyId)
                    .SetProperty(x => x.Algorithm, request.Algorithm!)
                    .SetProperty(x => x.PublicKey, request.PublicKey!)
                    .SetProperty(x => x.CreatedAt, now),
                cancellationToken);
            await db.AccountKeyBackups.Where(x => x.UserId == user.Id).ExecuteDeleteAsync(cancellationToken);
            db.AccountKeyBackups.Add(NewBackup(user.Id, request.Backup!, now));
            revoked = await db.EncryptionDevices
                .Where(x => x.UserId == user.Id && x.RevokedAt == null && x.SessionId != sessionId)
                .ExecuteUpdateAsync(setters => setters.SetProperty(x => x.RevokedAt, now), cancellationToken);
            changes = (await DirectEncryption.EncryptedConversationsOfAsync(db, user.Id, cancellationToken))
                .Select(conversation => (conversation, (DirectMessage?)DirectEncryption.AddSystemEntry(
                    db, conversation.Id, user, DirectSystemKinds.KeysReset, now)))
                .ToList();
            await db.SaveChangesAsync(cancellationToken);
            await transaction.CommitAsync(cancellationToken);
        }

        var key = await db.AccountKeys.AsNoTracking().SingleAsync(x => x.UserId == user.Id, cancellationToken);
        var backup = await db.AccountKeyBackups.AsNoTracking().SingleAsync(x => x.UserId == user.Id, cancellationToken);
        var told = await DirectEncryption.PublishKeyChangesAsync(hub, db, changes, user.Id);
        await DirectEncryption.PublishAccountChangedAsync(hub, user.Id);
        await SocialEndpoints.NotifyAsync(hub, [user.Id, .. told]);
        return Results.Ok(new ResetAccountKeyDto(key.ToDto(), backup.ToStatusDto(), revoked));
    }

    private static async Task<IResult> ListDevicesAsync(
        ClaimsPrincipal principal,
        AppDbContext db,
        CancellationToken cancellationToken) =>
        Results.Ok(new OwnDevicesDto(
            await OwnDevicesAsync(db, principal.RequireUserId(), principal.FindSessionId(), cancellationToken)));

    /// <summary>
    /// Registers the key of the browser asking, under its login session. It
    /// needs no PIN: a device without one still takes part in new messages.
    /// The same key again is a no-op, so a browser may register on every
    /// start; a different key replaces the session's device.
    /// </summary>
    private static async Task<IResult> RegisterDeviceAsync(
        RegisterDeviceRequest request,
        ClaimsPrincipal principal,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        if (InvalidKey(request.Algorithm, request.PublicKey, messages) is { } invalidKey)
        {
            return invalidKey;
        }

        var label = request.Label?.Trim() ?? "";
        if (label.Length is < 1 or > EncryptionLimits.MaxDeviceLabelLength)
        {
            return Invalid("label", messages.Get(
                MessageKey.EncryptionDeviceLabelInvalid, ("max", EncryptionLimits.MaxDeviceLabelLength)));
        }

        var userId = principal.RequireUserId();
        var sessionId = principal.FindSessionId()
            ?? throw new InvalidOperationException("Authenticated principal has no session id.");
        if (!await db.AccountKeys.AnyAsync(x => x.UserId == userId, cancellationToken))
        {
            return AccountKeyMissing(messages);
        }

        var now = DateTime.UtcNow;
        EncryptionDevice device;
        List<(DirectConversation Conversation, DirectMessage? Entry)> changes;
        await using (var transaction = await db.Database.BeginTransactionAsync(cancellationToken))
        {
            var existing = await db.EncryptionDevices
                .SingleOrDefaultAsync(x => x.SessionId == sessionId && x.RevokedAt == null, cancellationToken);
            if (existing is not null &&
                existing.Algorithm == request.Algorithm &&
                existing.PublicKey.AsSpan().SequenceEqual(request.PublicKey!))
            {
                existing.Label = label;
                existing.LastSeenAt = now;
                await db.SaveChangesAsync(cancellationToken);
                await transaction.CommitAsync(cancellationToken);
                return Results.Ok(existing.ToOwnDto(sessionId, now));
            }

            if (existing is null &&
                await db.EncryptionDevices.Active(now).CountAsync(x => x.UserId == userId, cancellationToken) >=
                EncryptionLimits.MaxActiveDevices)
            {
                return DirectEncryption.Problem(
                    messages,
                    StatusCodes.Status409Conflict,
                    Codes.DeviceLimit,
                    MessageKey.EncryptionDeviceLimit,
                    new Dictionary<string, object?> { ["max"] = EncryptionLimits.MaxActiveDevices });
            }

            if (existing is not null)
            {
                existing.RevokedAt = now;
                await db.SaveChangesAsync(cancellationToken);
            }

            device = new EncryptionDevice
            {
                Id = Guid.NewGuid(),
                UserId = userId,
                SessionId = sessionId,
                Algorithm = request.Algorithm!,
                PublicKey = request.PublicKey!,
                Label = label,
                CreatedAt = now,
                LastSeenAt = now,
            };
            db.EncryptionDevices.Add(device);
            var user = await db.Users.SingleAsync(x => x.Id == userId, cancellationToken);
            changes = (await DirectEncryption.EncryptedConversationsOfAsync(db, userId, cancellationToken))
                .Select(conversation => (conversation, (DirectMessage?)DirectEncryption.AddSystemEntry(
                    db, conversation.Id, user, DirectSystemKinds.DeviceAdded, now)))
                .ToList();
            await db.SaveChangesAsync(cancellationToken);
            await transaction.CommitAsync(cancellationToken);
        }

        var told = await DirectEncryption.PublishKeyChangesAsync(hub, db, changes, userId);
        await DirectEncryption.PublishAccountChangedAsync(hub, userId);
        if (told.Length > 0)
        {
            await SocialEndpoints.NotifyAsync(hub, [userId, .. told]);
        }

        return Results.Created("/api/me/encryption/devices", device.ToOwnDto(sessionId, now));
    }

    /// <summary>
    /// Removes the keys of one of the account's other browsers and signs that
    /// browser out. The browser asking removes its own keys by signing out.
    /// </summary>
    private static async Task<IResult> RevokeDeviceAsync(
        Guid deviceId,
        HttpContext context,
        AppDbContext db,
        HubConnectionRegistry connections,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var userId = context.User.RequireUserId();
        var now = DateTime.UtcNow;
        var device = await db.EncryptionDevices.AsNoTracking().Active(now)
            .SingleOrDefaultAsync(x => x.Id == deviceId && x.UserId == userId, cancellationToken);
        if (device is null)
        {
            return Results.NotFound();
        }

        if (device.SessionId == context.User.FindSessionId())
        {
            return DirectEncryption.Problem(
                messages, StatusCodes.Status409Conflict, Codes.CurrentDevice, MessageKey.EncryptionDeviceIsCurrent);
        }

        await using (var transaction = await db.Database.BeginTransactionAsync(cancellationToken))
        {
            await db.EncryptionDevices.Where(x => x.Id == deviceId)
                .ExecuteUpdateAsync(setters => setters.SetProperty(x => x.RevokedAt, now), cancellationToken);
            var signedOut = await db.UserSessions.Where(x => x.Id == device.SessionId && x.RevokedAt == null)
                .ExecuteUpdateAsync(setters => setters.SetProperty(x => x.RevokedAt, now), cancellationToken);
            if (signedOut == 1)
            {
                SecurityEvents.Record(db, userId, SecurityEventKinds.SessionRevoked, context, "encryption keys removed");
                await db.SaveChangesAsync(cancellationToken);
            }

            await transaction.CommitAsync(cancellationToken);
        }

        connections.AbortSession(device.SessionId);
        var conversations = await DirectEncryption.EncryptedConversationsOfAsync(db, userId, cancellationToken);
        await DirectEncryption.PublishKeyChangesAsync(hub, db, conversations.Select(x => (x, (DirectMessage?)null)), userId);
        await DirectEncryption.PublishAccountChangedAsync(hub, userId);
        return Results.NoContent();
    }

    private static async Task<OwnDeviceDto[]> OwnDevicesAsync(
        AppDbContext db,
        Guid userId,
        Guid? currentSessionId,
        CancellationToken cancellationToken)
    {
        var rows = await db.EncryptionDevices.AsNoTracking()
            .Active(DateTime.UtcNow)
            .Where(x => x.UserId == userId)
            .OrderBy(x => x.CreatedAt)
            .Select(x => new { Device = x, x.Session.LastSeenAt })
            .ToListAsync(cancellationToken);
        return [.. rows.Select(x => x.Device.ToOwnDto(currentSessionId, x.LastSeenAt))];
    }

    private static AccountKeyBackup NewBackup(Guid userId, KeyBackupRequest backup, DateTime now) => new()
    {
        UserId = userId,
        Salt = backup.Salt!,
        Iterations = backup.Iterations!.Value,
        Verifier = backup.Verifier!,
        Iv = backup.Iv!,
        Ciphertext = backup.Ciphertext!,
        FailedAttempts = 0,
        UpdatedAt = now,
    };

    private static IResult? InvalidKey(string? algorithm, byte[]? publicKey, Messages messages) =>
        EncryptionAlgorithms.IsPublicKey(algorithm, publicKey)
            ? null
            : Invalid("publicKey", messages.Get(MessageKey.EncryptionKeyInvalid));

    private static IResult? InvalidBackup(KeyBackupRequest? backup, Messages messages) =>
        backup is
        {
            Salt.Length: >= EncryptionLimits.MinSaltBytes and <= EncryptionLimits.MaxSaltBytes,
            Iterations: >= EncryptionLimits.MinIterations and <= EncryptionLimits.MaxIterations,
            Verifier.Length: EncryptionLimits.VerifierBytes,
            Iv.Length: EncryptionLimits.IvBytes,
            Ciphertext.Length: > EncryptionLimits.TagBytes and <= EncryptionLimits.MaxBackupCiphertextBytes,
        }
            ? null
            : Invalid("backup", messages.Get(MessageKey.EncryptionBackupInvalid));

    internal static IResult AccountKeyMissing(Messages messages) => DirectEncryption.Problem(
        messages, StatusCodes.Status409Conflict, Codes.AccountKeyMissing, MessageKey.EncryptionAccountKeyMissing);

    private static IResult AccountKeyExists(Messages messages) => DirectEncryption.Problem(
        messages, StatusCodes.Status409Conflict, Codes.AccountKeyExists, MessageKey.EncryptionAccountKeyExists);

    private static IResult BackupMissing(Messages messages) => DirectEncryption.Problem(
        messages, StatusCodes.Status404NotFound, Codes.BackupMissing, MessageKey.EncryptionBackupMissing);

    private static IResult PasswordIncorrect(Messages messages) => DirectEncryption.Problem(
        messages, StatusCodes.Status403Forbidden, Codes.PasswordIncorrect, MessageKey.EncryptionPasswordIncorrect);

    private static IResult DeviceMissing(Messages messages) => DirectEncryption.Problem(
        messages, StatusCodes.Status409Conflict, Codes.DeviceMissing, MessageKey.EncryptionDeviceMissing);

    private static IResult Invalid(string field, string message) =>
        Results.ValidationProblem(new Dictionary<string, string[]> { [field] = [message] });
}
