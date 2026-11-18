using System.Security.Claims;
using Microsoft.AspNetCore.SignalR;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;
using Stellarr.Api.Messaging;
using Stellarr.Api.Notifications;
using Stellarr.Api.Realtime;
using Codes = Stellarr.Api.Messaging.EncryptionProblemCodes;

namespace Stellarr.Api.Endpoints;

/// <summary>
/// Private conversations between two people, with the messages in them. Like
/// the friend routes they belong to the instance, not to a server.
/// </summary>
public static class DirectMessageEndpoints
{
    private const int DefaultPageSize = 50;
    private const int MaxPageSize = 100;
    private const int MaxAttachments = 10;

    /// <summary>
    /// An encrypted video brings its poster, and an encrypted image its preview,
    /// as an upload of its own, since the server cannot make one from bytes it
    /// cannot read.
    /// </summary>
    private const int MaxEncryptedUploads = 2 * MaxAttachments;

    public static IEndpointRouteBuilder MapDirectMessageEndpoints(this IEndpointRouteBuilder endpoints)
    {
        var conversations = endpoints.MapGroup("/api/social/conversations").RequireAuthorization();
        conversations.MapPost("", OpenAsync);
        conversations.MapGet("/{conversationId:guid}/messages", GetMessagesAsync);
        conversations.MapPost("/{conversationId:guid}/messages", CreateMessageAsync);
        conversations.MapPost("/{conversationId:guid}/read", MarkReadAsync);
        return endpoints;
    }

    /// <summary>What a sender asked to post, after trimming and de-duplication.</summary>
    private sealed record Submission(
        Guid ConversationId,
        Guid AuthorId,
        string Content,
        Guid[] AttachmentIds,
        EncryptedPayloadRequest? Encrypted)
    {
        /// <summary>
        /// Whether <paramref name="message"/> is what this submission would have
        /// produced. An encrypted message is the same send when the id the
        /// client picked, the author, the conversation and the uploads match:
        /// every encrypted message has empty content, and a browser that
        /// retries may encrypt again, with a new nonce or a newer epoch.
        /// </summary>
        public bool Produced(DirectMessage message) =>
            message.AuthorId == AuthorId &&
            message.ConversationId == ConversationId &&
            message.SystemKind is null &&
            (Encrypted is null
                ? message.EncryptedEpochId is null && string.Equals(message.Content, Content, StringComparison.Ordinal)
                : message.EncryptedEpochId is not null) &&
            message.Attachments.Select(x => x.AttachmentId).Order().SequenceEqual(AttachmentIds.Order());
    }

    /// <summary>
    /// Opens the conversation with somebody, or returns the one that is there:
    /// the pair has a single conversation, also when both ask at once.
    /// </summary>
    private static async Task<IResult> OpenAsync(
        OpenDirectConversationRequest request,
        ClaimsPrincipal principal,
        AppDbContext db,
        IHubContext<AppHub> hub,
        CancellationToken cancellationToken)
    {
        var selfId = principal.RequireUserId();
        if (selfId == request.UserId || !await db.Users.AnyAsync(x => x.Id == request.UserId, cancellationToken))
        {
            return Results.NotFound();
        }

        if (await SocialEndpoints.IsBlockedAsync(db, selfId, request.UserId, cancellationToken))
        {
            return Results.Forbid();
        }

        var (a, b) = SocialEndpoints.Pair(selfId, request.UserId);
        var conversation = await db.DirectConversations.AsNoTracking()
            .SingleOrDefaultAsync(x => x.UserAId == a && x.UserBId == b, cancellationToken);
        if (conversation is null)
        {
            conversation = new DirectConversation
            {
                Id = Guid.NewGuid(),
                UserAId = a,
                UserBId = b,
                CreatedAt = DateTime.UtcNow,
            };
            db.DirectConversations.Add(conversation);
            if (await db.TrySaveChangesAsync(cancellationToken))
            {
                await SocialEndpoints.NotifyAsync(hub, selfId, request.UserId);
            }
            else
            {
                conversation = await db.DirectConversations.AsNoTracking()
                    .SingleAsync(x => x.UserAId == a && x.UserBId == b, cancellationToken);
            }
        }

        return Results.Ok(new OpenedConversationDto(conversation.Id.ToString()));
    }

    /// <summary>
    /// One page of a conversation, oldest message first. A conversation with
    /// somebody blocked, in either direction, is as good as absent.
    /// </summary>
    private static async Task<IResult> GetMessagesAsync(
        Guid conversationId,
        string? cursor,
        int? limit,
        ClaimsPrincipal principal,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        if (limit is < 1 or > MaxPageSize)
        {
            return Invalid("limit", messages.Get(MessageKey.PageLimitRange, ("max", MaxPageSize)));
        }

        var selfId = principal.RequireUserId();
        var conversation = await FindConversationAsync(db, conversationId, selfId, cancellationToken);
        if (conversation is null ||
            await SocialEndpoints.IsBlockedAsync(db, selfId, SocialEndpoints.PeerId(conversation, selfId), cancellationToken))
        {
            return Results.NotFound();
        }

        HistoryCursor page;
        if (cursor is null)
        {
            page = new HistoryCursor(conversationId, DateTime.UtcNow);
        }
        else if (!HistoryCursor.TryDecode(cursor, out page) || page.ScopeId != conversationId)
        {
            return Invalid("cursor", messages.Get(MessageKey.DirectHistoryCursorInvalid));
        }

        var pageSize = limit ?? DefaultPageSize;
        var query = db.DirectMessages.AsNoTracking()
            .WithMessageDetails()
            .Where(x => x.ConversationId == conversationId && x.CreatedAt <= page.Snapshot);
        if (page.BeforeCreatedAt is { } before && page.BeforeId is { } beforeId)
        {
            query = query.Where(x => x.CreatedAt < before ||
                (x.CreatedAt == before && x.Id.CompareTo(beforeId) < 0));
        }

        var rows = await query
            .OrderByDescending(x => x.CreatedAt)
            .ThenByDescending(x => x.Id)
            .Take(pageSize + 1)
            .AsSplitQuery()
            .ToListAsync(cancellationToken);

        return Results.Ok(new DirectMessageHistoryPageDto(
            [.. rows.Take(pageSize).Reverse().Select(x => x.ToDto())],
            rows.Count > pageSize
                ? (page with { BeforeCreatedAt = rows[pageSize - 1].CreatedAt, BeforeId = rows[pageSize - 1].Id }).Encode()
                : null));
    }

    private static async Task<IResult> CreateMessageAsync(
        Guid conversationId,
        CreateDirectMessageRequest request,
        ClaimsPrincipal principal,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var selfId = principal.RequireUserId();
        var conversation = await FindConversationAsync(db, conversationId, selfId, cancellationToken);
        if (conversation is null)
        {
            return Results.NotFound();
        }

        var peerId = SocialEndpoints.PeerId(conversation, selfId);
        if (await SocialEndpoints.IsBlockedAsync(db, selfId, peerId, cancellationToken))
        {
            return Results.Forbid();
        }

        var encrypted = request.Encrypted;
        var maxAttachments = encrypted is null ? MaxAttachments : MaxEncryptedUploads;
        if (request.AttachmentIds?.Count > maxAttachments)
        {
            return Invalid("attachmentIds", messages.Get(MessageKey.AttachmentLimit, ("max", maxAttachments)));
        }

        if (request.ClientMessageId == Guid.Empty)
        {
            return Invalid("clientMessageId", messages.Get(MessageKey.MessageIdInvalid));
        }

        var submission = new Submission(
            conversationId,
            selfId,
            request.Content?.Trim() ?? "",
            request.AttachmentIds?.Distinct().ToArray() ?? [],
            encrypted);
        if (encrypted is not null)
        {
            // The text is in the ciphertext, which the server cannot check
            // against the limit; it only bounds the size.
            if (request.ClientMessageId is null ||
                !string.IsNullOrEmpty(request.Content) ||
                encrypted.EpochId is null ||
                encrypted.Iv is not { Length: EncryptionLimits.IvBytes } ||
                encrypted.Ciphertext is not { Length: > EncryptionLimits.TagBytes and <= EncryptionLimits.MaxMessageCiphertextBytes })
            {
                return Invalid("encrypted", messages.Get(MessageKey.EncryptionMessageInvalid));
            }
        }
        else if ((submission.Content.Length == 0 && submission.AttachmentIds.Length == 0) ||
            submission.Content.Length > MessageLimits.MaxContentLength)
        {
            return Invalid(
                "content",
                messages.Get(MessageKey.MessageContentInvalid, ("max", MessageLimits.MaxContentLength)));
        }

        // A retry of a send that went through is answered before anything that
        // could have changed since: the attachments it used, the peer's settings.
        if (await ReplayAsync(db, submission, request.ClientMessageId, cancellationToken) is { } replay)
        {
            return replay;
        }

        var attachments = submission.AttachmentIds.Length == 0
            ? []
            : await db.ImageAttachments
                .Where(x => submission.AttachmentIds.Contains(x.Id) && x.UploadedById == selfId)
                .ToListAsync(cancellationToken);
        if (attachments.Count != submission.AttachmentIds.Length)
        {
            return Invalid("attachmentIds", messages.Get(MessageKey.AttachmentsNotFound));
        }

        if (attachments.Any(x => x.IsEncrypted != (encrypted is not null)))
        {
            return DirectEncryption.Problem(
                messages, StatusCodes.Status409Conflict, Codes.AttachmentKind, MessageKey.EncryptionAttachmentKindMismatch);
        }

        if (!await PeerTakesMessagesFromAsync(db, selfId, peerId, cancellationToken))
        {
            return Results.Forbid();
        }

        var message = new DirectMessage
        {
            Id = request.ClientMessageId ?? Guid.NewGuid(),
            ConversationId = conversationId,
            AuthorId = selfId,
            Author = await db.Users.SingleAsync(x => x.Id == selfId, cancellationToken),
            Content = submission.Content,
            CreatedAt = DateTime.UtcNow,
            EncryptedEpochId = encrypted?.EpochId,
            EncryptedIv = encrypted?.Iv,
            EncryptedCiphertext = encrypted?.Ciphertext,
            Attachments = [.. attachments.Select(x => new DirectMessageAttachment { AttachmentId = x.Id, Attachment = x })],
        };
        try
        {
            // Whoever sent a message has read everything before it.
            await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);

            // Checked in the write transaction, so that no switch or new epoch
            // can come between the check and the message.
            if (await EncryptionRefusalAsync(db, conversationId, encrypted, messages, cancellationToken) is { } refused)
            {
                return refused;
            }

            db.DirectMessages.Add(message);
            await db.SaveChangesAsync(cancellationToken);
            await AdvanceReadAsync(db, conversationId, selfId, message.CreatedAt, cancellationToken);
            // Only the peer is a recipient. The row commits with the message,
            // and the replay path above never reaches this transaction.
            await NotificationOutbox.EnqueueDirectMessageAsync(db, message, peerId, cancellationToken);
            await db.SaveChangesAsync(cancellationToken);
            await transaction.CommitAsync(cancellationToken);
        }
        catch (DbUpdateException exception) when (
            request.ClientMessageId is not null && DatabaseErrors.IsUniqueConstraint(exception))
        {
            // Another request with the same client id stored it between the
            // check above and the insert.
            db.ChangeTracker.Clear();
            if (await ReplayAsync(db, submission, request.ClientMessageId, cancellationToken) is { } raced)
            {
                return raced;
            }

            throw;
        }

        // The message itself is delivered, not just a hint to reload: the open
        // transcript appends it at once while the rail refreshes its unread
        // count. It is stored by now, so a client that hangs up must not keep
        // the other side from hearing about it.
        var response = message.ToDto();
        await hub.Clients.Groups(AppHub.UserGroup(selfId), AppHub.UserGroup(peerId))
            .SendAsync("DirectMessageCreated", response, CancellationToken.None);
        await SocialEndpoints.NotifyAsync(hub, selfId, peerId);
        return Results.Created($"/api/social/conversations/{conversationId}/messages/{message.Id}", response);
    }

    private static async Task<IResult> MarkReadAsync(
        Guid conversationId,
        ClaimsPrincipal principal,
        AppDbContext db,
        IHubContext<AppHub> hub,
        CancellationToken cancellationToken)
    {
        var selfId = principal.RequireUserId();
        if (await FindConversationAsync(db, conversationId, selfId, cancellationToken) is null)
        {
            return Results.NotFound();
        }

        await AdvanceReadAsync(db, conversationId, selfId, DateTime.UtcNow, cancellationToken);
        await SocialEndpoints.NotifyAsync(hub, selfId);
        return Results.NoContent();
    }

    /// <summary>
    /// The answer to a send whose client id is already taken: the stored
    /// message when it is the same send again, a conflict when it is not.
    /// Null when nothing is stored under the id, or no id was given.
    /// </summary>
    private static async Task<IResult?> ReplayAsync(
        AppDbContext db,
        Submission submission,
        Guid? clientMessageId,
        CancellationToken cancellationToken)
    {
        if (clientMessageId is not { } id)
        {
            return null;
        }

        var existing = await db.DirectMessages.AsNoTracking()
            .WithMessageDetails()
            .SingleOrDefaultAsync(x => x.Id == id, cancellationToken);
        if (existing is null)
        {
            return null;
        }

        return submission.Produced(existing) ? Results.Ok(existing.ToDto()) : Results.Conflict();
    }

    /// <summary>
    /// Whether the peer accepts a message from this person. Somebody who turned
    /// direct messages off still hears from their friends.
    /// </summary>
    internal static async Task<bool> PeerTakesMessagesFromAsync(
        AppDbContext db,
        Guid senderId,
        Guid peerId,
        CancellationToken cancellationToken)
    {
        var preferences = await db.UserPreferences.AsNoTracking()
            .SingleOrDefaultAsync(x => x.UserId == peerId, cancellationToken);
        if (preferences?.AllowDirectMessages != false)
        {
            return true;
        }

        var (a, b) = SocialEndpoints.Pair(senderId, peerId);
        return await db.Friendships.AnyAsync(x => x.UserAId == a && x.UserBId == b, cancellationToken);
    }

    /// <summary>
    /// Why a message cannot go into the conversation as it stands, or null when
    /// it can. While encryption is on, a message must be encrypted with the
    /// current epoch, and that epoch must still reach exactly the account keys
    /// and active devices of both people: a device that was added or removed
    /// since asks for a new epoch first. While it is not on, nothing may be
    /// encrypted.
    /// </summary>
    private static async Task<IResult?> EncryptionRefusalAsync(
        AppDbContext db,
        Guid conversationId,
        EncryptedPayloadRequest? encrypted,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var conversation = await db.DirectConversations.AsNoTracking()
            .SingleAsync(x => x.Id == conversationId, cancellationToken);
        if (conversation.EncryptionState != DirectEncryptionState.On)
        {
            return encrypted is null ? null : DirectEncryptionEndpoints.Off(conversation, messages);
        }

        if (encrypted is null)
        {
            return DirectEncryption.Problem(
                messages,
                StatusCodes.Status409Conflict,
                Codes.Required,
                MessageKey.EncryptionRequired,
                new Dictionary<string, object?> { ["encryption"] = conversation.ToEncryptionDto() });
        }

        if (conversation.CurrentEpochId is not { } currentEpochId || currentEpochId != encrypted.EpochId)
        {
            return DirectEncryption.Problem(
                messages,
                StatusCodes.Status409Conflict,
                Codes.EpochNotCurrent,
                MessageKey.EncryptionEpochNotCurrent,
                new Dictionary<string, object?> { ["currentEpochId"] = conversation.CurrentEpochId?.ToString() });
        }

        var recipients = await DirectEncryption.RecipientsAsync(
            db, conversation.UserAId, conversation.UserBId, DateTime.UtcNow, cancellationToken);
        var wrapped = await db.EncryptionEnvelopes.AsNoTracking()
            .Where(x => x.EpochId == currentEpochId)
            .Select(x => x.RecipientKeyId)
            .ToListAsync(cancellationToken);
        return DirectEncryptionEndpoints.CoverageProblem(
            recipients, wrapped, messages, MessageKey.EncryptionEpochOutdated, Codes.EpochOutdated);
    }

    internal static Task<DirectConversation?> FindConversationAsync(
        AppDbContext db, Guid conversationId, Guid userId, CancellationToken cancellationToken) =>
        db.DirectConversations.AsNoTracking().SingleOrDefaultAsync(
            x => x.Id == conversationId && (x.UserAId == userId || x.UserBId == userId), cancellationToken);

    /// <summary>
    /// Sets the person's read marker in one statement, so that two requests
    /// racing for the row cannot fail on its primary key.
    /// </summary>
    private static Task<int> AdvanceReadAsync(
        AppDbContext db,
        Guid conversationId,
        Guid userId,
        DateTime lastReadAt,
        CancellationToken cancellationToken) =>
        db.Database.ExecuteSqlInterpolatedAsync(
            $"""
            INSERT INTO "DirectConversationReads" ("ConversationId", "UserId", "LastReadAt")
            VALUES ({conversationId}, {userId}, {lastReadAt})
            ON CONFLICT ("ConversationId", "UserId") DO UPDATE
            SET "LastReadAt" = excluded."LastReadAt";
            """,
            cancellationToken);

    private static IResult Invalid(string field, string message) =>
        Results.ValidationProblem(new Dictionary<string, string[]> { [field] = [message] });
}
