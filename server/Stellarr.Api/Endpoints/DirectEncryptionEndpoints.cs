using System.Security.Claims;
using Microsoft.AspNetCore.SignalR;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;
using Stellarr.Api.Messaging;
using Stellarr.Api.Realtime;
using Codes = Stellarr.Api.Messaging.EncryptionProblemCodes;

namespace Stellarr.Api.Endpoints;

/// <summary>
/// End-to-end encryption of one private conversation: the switch both people
/// have to agree on, the keys of both of them, and the epochs that new
/// messages are encrypted with. Like the messages, it is as good as absent
/// to anybody else, and closed while either person blocks the other.
/// </summary>
public static class DirectEncryptionEndpoints
{
    /// <summary>Both account keys and every device both people may have at once.</summary>
    private const int MaxEnvelopes = 2 * (EncryptionLimits.MaxActiveDevices + 1);
    private const int EpochPageSize = 200;

    public static IEndpointRouteBuilder MapDirectEncryptionEndpoints(this IEndpointRouteBuilder endpoints)
    {
        var encryption = endpoints.MapGroup("/api/social/conversations/{conversationId:guid}/encryption")
            .RequireAuthorization();
        encryption.MapPost("/request", RequestAsync);
        encryption.MapPost("/cancel", CancelAsync);
        encryption.MapPost("/confirm", ConfirmAsync);
        encryption.MapPost("/disable", DisableAsync);
        encryption.MapGet("/keys", GetKeysAsync);
        encryption.MapGet("/epochs", GetEpochsAsync);
        encryption.MapPost("/epochs", CreateEpochAsync);
        return endpoints;
    }

    /// <summary>What a switch does to a conversation: the line it adds, or why it is refused.</summary>
    private delegate IResult? Decide(DirectConversation conversation, Guid selfId, Messages messages);

    /// <summary>
    /// Asks the other person to switch encryption on. Needs an account key, and
    /// reaches the other person like a message does: somebody who takes direct
    /// messages only from friends takes requests only from them too.
    /// </summary>
    private static Task<IResult> RequestAsync(
        Guid conversationId,
        ClaimsPrincipal principal,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken) =>
        SwitchAsync(
            conversationId, principal, db, hub, messages, DirectSystemKinds.EncryptionRequested, requiresAccountKey: true,
            asksThePeer: true,
            (conversation, selfId, _) =>
            {
                if (conversation.EncryptionState != DirectEncryptionState.Off)
                {
                    return StateConflict(conversation, messages);
                }

                conversation.EncryptionState = DirectEncryptionState.Requested;
                conversation.EncryptionRequestedById = selfId;
                conversation.EncryptionRequestedAt = DateTime.UtcNow;
                return null;
            },
            cancellationToken);

    /// <summary>Takes back a request; only the person who sent it.</summary>
    private static Task<IResult> CancelAsync(
        Guid conversationId,
        ClaimsPrincipal principal,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken) =>
        SwitchAsync(
            conversationId, principal, db, hub, messages, DirectSystemKinds.EncryptionRequestCancelled, requiresAccountKey: false,
            asksThePeer: false,
            (conversation, selfId, _) =>
            {
                if (conversation.EncryptionState != DirectEncryptionState.Requested)
                {
                    return StateConflict(conversation, messages);
                }

                if (conversation.EncryptionRequestedById != selfId)
                {
                    return NotAllowed(messages, MessageKey.EncryptionCancelNotRequester);
                }

                SwitchOff(conversation);
                return null;
            },
            cancellationToken);

    /// <summary>
    /// Switches encryption on: only the person who was asked, and only with an
    /// account key. The first message after it starts the first epoch.
    /// </summary>
    private static Task<IResult> ConfirmAsync(
        Guid conversationId,
        ClaimsPrincipal principal,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken) =>
        SwitchAsync(
            conversationId, principal, db, hub, messages, DirectSystemKinds.EncryptionEnabled, requiresAccountKey: true,
            asksThePeer: false,
            (conversation, selfId, _) =>
            {
                if (conversation.EncryptionState != DirectEncryptionState.Requested)
                {
                    return StateConflict(conversation, messages);
                }

                if (conversation.EncryptionRequestedById == selfId)
                {
                    return NotAllowed(messages, MessageKey.EncryptionConfirmOwnRequest);
                }

                conversation.EncryptionState = DirectEncryptionState.On;
                conversation.EncryptionRequestedById = null;
                conversation.EncryptionRequestedAt = null;
                conversation.EncryptionEnabledAt = DateTime.UtcNow;
                conversation.CurrentEpochId = null;
                return null;
            },
            cancellationToken);

    /// <summary>
    /// Switches encryption off, or turns a pending request down: either person,
    /// at any time. Switching on again takes a request and a confirmation again.
    /// </summary>
    private static Task<IResult> DisableAsync(
        Guid conversationId,
        ClaimsPrincipal principal,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken) =>
        SwitchAsync(
            conversationId, principal, db, hub, messages, DirectSystemKinds.EncryptionDisabled, requiresAccountKey: false,
            asksThePeer: false,
            (conversation, _, _) =>
            {
                if (conversation.EncryptionState == DirectEncryptionState.Off)
                {
                    return StateConflict(conversation, messages);
                }

                SwitchOff(conversation);
                return null;
            },
            cancellationToken);

    /// <summary>
    /// Applies one switch in a write transaction, with the line it adds to the
    /// timeline, and tells both people.
    /// </summary>
    private static async Task<IResult> SwitchAsync(
        Guid conversationId,
        ClaimsPrincipal principal,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        string kind,
        bool requiresAccountKey,
        bool asksThePeer,
        Decide decide,
        CancellationToken cancellationToken)
    {
        var selfId = principal.RequireUserId();
        if (await RefuseChangeAsync(db, conversationId, selfId, cancellationToken) is { } refused)
        {
            return refused;
        }

        if (asksThePeer)
        {
            var found = await DirectMessageEndpoints.FindConversationAsync(db, conversationId, selfId, cancellationToken);
            if (!await DirectMessageEndpoints.PeerTakesMessagesFromAsync(
                    db, selfId, SocialEndpoints.PeerId(found!, selfId), cancellationToken))
            {
                return Results.Forbid();
            }
        }

        if (requiresAccountKey && !await db.AccountKeys.AnyAsync(x => x.UserId == selfId, cancellationToken))
        {
            return EncryptionEndpoints.AccountKeyMissing(messages);
        }

        DirectConversation conversation;
        DirectMessage entry;
        await using (var transaction = await db.Database.BeginTransactionAsync(cancellationToken))
        {
            conversation = await db.DirectConversations.SingleAsync(x => x.Id == conversationId, cancellationToken);
            if (decide(conversation, selfId, messages) is { } refusal)
            {
                return refusal;
            }

            var self = await db.Users.SingleAsync(x => x.Id == selfId, cancellationToken);
            entry = DirectEncryption.AddSystemEntry(db, conversationId, self, kind, DateTime.UtcNow);
            await db.SaveChangesAsync(cancellationToken);
            await transaction.CommitAsync(cancellationToken);
        }

        var people = new[] { AppHub.UserGroup(conversation.UserAId), AppHub.UserGroup(conversation.UserBId) };
        var state = conversation.ToEncryptionDto();
        await hub.Clients.Groups(people).SendAsync("DirectMessageCreated", entry.ToDto(), CancellationToken.None);
        await hub.Clients.Groups(people).SendAsync(
            "DirectEncryptionChanged",
            new DirectEncryptionChangedDto(conversationId.ToString(), state),
            CancellationToken.None);
        await SocialEndpoints.NotifyAsync(hub, conversation.UserAId, conversation.UserBId);
        return Results.Ok(state);
    }

    /// <summary>
    /// The keys of both people and the current epoch: everything a browser
    /// needs to encrypt the next message, or to see that a new epoch is due.
    /// </summary>
    private static async Task<IResult> GetKeysAsync(
        Guid conversationId,
        ClaimsPrincipal principal,
        AppDbContext db,
        CancellationToken cancellationToken)
    {
        var selfId = principal.RequireUserId();
        var conversation = await ReadableConversationAsync(db, conversationId, selfId, cancellationToken);
        if (conversation is null)
        {
            return Results.NotFound();
        }

        var peerId = SocialEndpoints.PeerId(conversation, selfId);
        var now = DateTime.UtcNow;
        var accountKeys = await db.AccountKeys.AsNoTracking()
            .Where(x => x.UserId == selfId || x.UserId == peerId)
            .ToDictionaryAsync(x => x.UserId, cancellationToken);
        var devices = await db.EncryptionDevices.AsNoTracking()
            .Active(now)
            .Where(x => x.UserId == selfId || x.UserId == peerId)
            .OrderBy(x => x.CreatedAt)
            .ThenBy(x => x.Id)
            .ToListAsync(cancellationToken);
        var sessionId = principal.FindSessionId();
        var currentDevice = devices.SingleOrDefault(x => x.SessionId == sessionId);
        var ownKeyIds = OwnKeyIds(accountKeys.GetValueOrDefault(selfId), currentDevice);

        PersonKeysDto Person(Guid userId) => new(
            userId.ToString(),
            accountKeys.GetValueOrDefault(userId)?.ToDto(),
            [.. devices.Where(x => x.UserId == userId).Select(x => x.ToKeyDto())]);

        var recipients = accountKeys.Values
            .Select(x => new RecipientKeyDto(x.KeyId.ToString(), DirectEncryption.AccountKind, x.UserId.ToString()))
            .Concat(devices.Select(x => new RecipientKeyDto(x.Id.ToString(), DirectEncryption.DeviceKind, x.UserId.ToString())))
            .OrderBy(x => x.UserId, StringComparer.Ordinal)
            .ThenBy(x => x.Kind, StringComparer.Ordinal)
            .ThenBy(x => x.KeyId, StringComparer.Ordinal)
            .ToArray();

        EpochDto? currentEpoch = null;
        if (conversation.CurrentEpochId is { } epochId)
        {
            var epoch = await db.EncryptionEpochs.AsNoTracking()
                .Include(x => x.Envelopes)
                .SingleAsync(x => x.Id == epochId, cancellationToken);
            currentEpoch = epoch.ToDto(epoch.Envelopes.Where(x => ownKeyIds.Contains(x.RecipientKeyId)));
        }

        return Results.Ok(new ConversationKeysDto(
            conversationId.ToString(),
            conversation.ToEncryptionDto(),
            Person(selfId),
            Person(peerId),
            currentDevice?.Id.ToString(),
            recipients,
            currentEpoch));
    }

    /// <summary>
    /// The conversation's epochs after a sequence number, from every period it
    /// was encrypted, each with the envelopes for the reader's account key and
    /// this browser's device. Envelopes for keys the reader no longer has are
    /// of no use to them and are left out.
    /// </summary>
    private static async Task<IResult> GetEpochsAsync(
        Guid conversationId,
        int? after,
        ClaimsPrincipal principal,
        AppDbContext db,
        CancellationToken cancellationToken)
    {
        var selfId = principal.RequireUserId();
        var conversation = await ReadableConversationAsync(db, conversationId, selfId, cancellationToken);
        if (conversation is null)
        {
            return Results.NotFound();
        }

        var now = DateTime.UtcNow;
        var accountKey = await db.AccountKeys.AsNoTracking().SingleOrDefaultAsync(x => x.UserId == selfId, cancellationToken);
        var device = await DirectEncryption.CurrentDeviceAsync(db, principal.FindSessionId(), now, cancellationToken);
        var ownKeyIds = OwnKeyIds(accountKey, device);
        var from = after ?? 0;
        var epochs = await db.EncryptionEpochs.AsNoTracking()
            .Include(x => x.Envelopes)
            .Where(x => x.ConversationId == conversationId && x.Sequence > from)
            .OrderBy(x => x.Sequence)
            .Take(EpochPageSize + 1)
            .AsSplitQuery()
            .ToListAsync(cancellationToken);
        return Results.Ok(new ConversationEpochsDto(
            conversation.CurrentEpochId?.ToString(),
            [.. epochs.Take(EpochPageSize).Select(x => x.ToDto(x.Envelopes.Where(e => ownKeyIds.Contains(e.RecipientKeyId))))],
            epochs.Count > EpochPageSize));
    }

    /// <summary>
    /// Starts a new epoch. It must follow the current one, so two browsers
    /// rotating at once cannot both win, and it must reach exactly the account
    /// keys and active devices of both people, so that no browser can leave a
    /// device out without anybody noticing. Sending the same epoch again
    /// answers with the stored one.
    /// </summary>
    private static async Task<IResult> CreateEpochAsync(
        Guid conversationId,
        CreateEpochRequest request,
        ClaimsPrincipal principal,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        if (!TryParseId(request.EpochId, out var epochId) ||
            request.Envelopes is not { Count: > 0 and <= MaxEnvelopes } envelopes ||
            envelopes.Any(x => !TryParseId(x.RecipientKeyId, out _) ||
                x.EphemeralPublicKey is null ||
                x.Iv is not { Length: EncryptionLimits.IvBytes } ||
                x.Ciphertext is not { Length: > EncryptionLimits.TagBytes and <= EncryptionLimits.MaxEnvelopeCiphertextBytes }) ||
            envelopes.Select(x => x.RecipientKeyId).Distinct(StringComparer.Ordinal).Count() != envelopes.Count)
        {
            return InvalidEpoch(messages);
        }

        var selfId = principal.RequireUserId();
        if (await RefuseChangeAsync(db, conversationId, selfId, cancellationToken) is { } refused)
        {
            return refused;
        }

        var wrapped = envelopes.ToDictionary(x => Guid.Parse(x.RecipientKeyId!));
        var now = DateTime.UtcNow;
        DirectConversation conversation;
        EncryptionEpoch epoch;
        await using (var transaction = await db.Database.BeginTransactionAsync(cancellationToken))
        {
            var stored = await db.EncryptionEpochs.AsNoTracking()
                .Include(x => x.Envelopes)
                .SingleOrDefaultAsync(x => x.Id == epochId, cancellationToken);
            if (stored is not null)
            {
                return stored.ConversationId == conversationId && stored.CreatedById == selfId
                    ? Results.Ok(stored.ToDto(stored.Envelopes.Where(x => x.RecipientUserId == selfId)))
                    : DirectEncryption.Problem(
                        messages, StatusCodes.Status409Conflict, Codes.EpochExists, MessageKey.EncryptionEpochExists);
            }

            conversation = await db.DirectConversations.SingleAsync(x => x.Id == conversationId, cancellationToken);
            if (conversation.EncryptionState != DirectEncryptionState.On)
            {
                return Off(conversation, messages);
            }

            if (conversation.CurrentEpochId != request.PreviousEpochId)
            {
                return DirectEncryption.Problem(
                    messages,
                    StatusCodes.Status409Conflict,
                    Codes.EpochStale,
                    MessageKey.EncryptionEpochStale,
                    new Dictionary<string, object?> { ["currentEpochId"] = conversation.CurrentEpochId?.ToString() });
            }

            var recipients = await DirectEncryption.RecipientsAsync(
                db, conversation.UserAId, conversation.UserBId, now, cancellationToken);
            if (CoverageProblem(recipients, wrapped.Keys, messages, MessageKey.EncryptionEpochCoverage, Codes.EpochCoverage) is { } uncovered)
            {
                return uncovered;
            }

            if (recipients.Any(x => !EncryptionAlgorithms.IsPublicKey(x.Algorithm, wrapped[x.KeyId].EphemeralPublicKey)))
            {
                return InvalidEpoch(messages);
            }

            var sequence = await db.EncryptionEpochs
                .Where(x => x.ConversationId == conversationId)
                .MaxAsync(x => (int?)x.Sequence, cancellationToken) ?? 0;
            epoch = new EncryptionEpoch
            {
                Id = epochId,
                ConversationId = conversationId,
                Sequence = sequence + 1,
                PreviousEpochId = request.PreviousEpochId,
                CreatedById = selfId,
                CreatedAt = now,
                Envelopes = [.. recipients.Select(recipient => new EncryptionEnvelope
                {
                    EpochId = epochId,
                    RecipientKeyId = recipient.KeyId,
                    RecipientUserId = recipient.UserId,
                    RecipientKind = recipient.Kind,
                    EphemeralPublicKey = wrapped[recipient.KeyId].EphemeralPublicKey!,
                    Iv = wrapped[recipient.KeyId].Iv!,
                    Ciphertext = wrapped[recipient.KeyId].Ciphertext!,
                })],
            };
            db.EncryptionEpochs.Add(epoch);
            conversation.CurrentEpochId = epochId;
            await db.SaveChangesAsync(cancellationToken);
            await transaction.CommitAsync(cancellationToken);
        }

        // Each person gets the envelopes for all of their own keys, so that
        // every one of their browsers can open the epoch without asking.
        foreach (var userId in new[] { conversation.UserAId, conversation.UserBId })
        {
            await hub.Clients.Group(AppHub.UserGroup(userId)).SendAsync(
                "DirectEncryptionEpochCreated",
                new DirectEpochCreatedDto(
                    conversationId.ToString(),
                    epoch.ToDto(epoch.Envelopes.Where(x => x.RecipientUserId == userId))),
                CancellationToken.None);
        }

        return Results.Created(
            $"/api/social/conversations/{conversationId}/encryption/epochs",
            epoch.ToDto(epoch.Envelopes.Where(x => x.RecipientUserId == selfId)));
    }

    /// <summary>
    /// A 409 naming the keys an epoch should have been wrapped for and was
    /// not, and those it was wrapped for that do not belong in it.
    /// </summary>
    internal static IResult? CoverageProblem(
        IReadOnlyCollection<EncryptionRecipient> required,
        IReadOnlyCollection<Guid> wrapped,
        Messages messages,
        MessageKey title,
        string code)
    {
        var (missing, extra) = DirectEncryption.Compare(required, wrapped);
        return missing.Length == 0 && extra.Length == 0
            ? null
            : DirectEncryption.Problem(
                messages,
                StatusCodes.Status409Conflict,
                code,
                title,
                new Dictionary<string, object?>
                {
                    ["missing"] = missing.Select(x => x.ToString()).ToArray(),
                    ["extra"] = extra.Select(x => x.ToString()).ToArray(),
                });
    }

    internal static IResult Off(DirectConversation conversation, Messages messages) => DirectEncryption.Problem(
        messages,
        StatusCodes.Status409Conflict,
        Codes.Off,
        MessageKey.EncryptionOff,
        new Dictionary<string, object?> { ["encryption"] = conversation.ToEncryptionDto() });

    private static IResult StateConflict(DirectConversation conversation, Messages messages) => DirectEncryption.Problem(
        messages,
        StatusCodes.Status409Conflict,
        Codes.StateConflict,
        MessageKey.EncryptionStateConflict,
        new Dictionary<string, object?> { ["encryption"] = conversation.ToEncryptionDto() });

    private static IResult NotAllowed(Messages messages, MessageKey title) =>
        DirectEncryption.Problem(messages, StatusCodes.Status403Forbidden, Codes.NotAllowed, title);

    private static IResult InvalidEpoch(Messages messages) =>
        Results.ValidationProblem(new Dictionary<string, string[]>
        {
            ["envelopes"] = [messages.Get(MessageKey.EncryptionEpochInvalid)],
        });

    private static void SwitchOff(DirectConversation conversation)
    {
        conversation.EncryptionState = DirectEncryptionState.Off;
        conversation.EncryptionRequestedById = null;
        conversation.EncryptionRequestedAt = null;
        conversation.EncryptionEnabledAt = null;
        conversation.CurrentEpochId = null;
    }

    private static HashSet<Guid> OwnKeyIds(AccountKey? accountKey, EncryptionDevice? device) =>
        [.. new[] { accountKey?.KeyId, device?.Id }.OfType<Guid>()];

    /// <summary>
    /// An id in the one form browsers bind keys to: a lowercase UUID with
    /// hyphens. Any other spelling would make the two sides derive different keys.
    /// </summary>
    internal static bool TryParseId(string? value, out Guid id) =>
        Guid.TryParseExact(value, "D", out id) && string.Equals(value, id.ToString(), StringComparison.Ordinal);

    /// <summary>The conversation when the reader is in it and neither person blocks the other.</summary>
    private static async Task<DirectConversation?> ReadableConversationAsync(
        AppDbContext db,
        Guid conversationId,
        Guid selfId,
        CancellationToken cancellationToken)
    {
        var conversation = await DirectMessageEndpoints.FindConversationAsync(db, conversationId, selfId, cancellationToken);
        return conversation is null ||
            await SocialEndpoints.IsBlockedAsync(db, selfId, SocialEndpoints.PeerId(conversation, selfId), cancellationToken)
                ? null
                : conversation;
    }

    /// <summary>
    /// The refusal of a change to a conversation, as for its messages: 404 for
    /// an outsider, 403 within a block. Null when the change may go ahead.
    /// </summary>
    private static async Task<IResult?> RefuseChangeAsync(
        AppDbContext db,
        Guid conversationId,
        Guid selfId,
        CancellationToken cancellationToken)
    {
        var conversation = await DirectMessageEndpoints.FindConversationAsync(db, conversationId, selfId, cancellationToken);
        if (conversation is null)
        {
            return Results.NotFound();
        }

        return await SocialEndpoints.IsBlockedAsync(db, selfId, SocialEndpoints.PeerId(conversation, selfId), cancellationToken)
            ? Results.Forbid()
            : null;
    }
}
