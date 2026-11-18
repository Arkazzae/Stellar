using Stellarr.Api.Domain;

namespace Stellarr.Api.Contracts;

/// <param name="Username">With or without a leading "@"; a null is answered like an empty name.</param>
public sealed record CreateFriendRequestRequest(string? Username);

public sealed record OpenDirectConversationRequest(Guid UserId);

/// <param name="Content">
/// Required unless attachments are sent; a null is answered like an empty
/// text. Empty for an encrypted message, whose text is in the ciphertext.
/// </param>
/// <param name="ClientMessageId">
/// The identity the client picked for the message, so that a retried send is
/// recognised as the same send rather than stored twice. Required for an
/// encrypted message.
/// </param>
/// <param name="Encrypted">The ciphertext, in a conversation whose encryption is on.</param>
public sealed record CreateDirectMessageRequest(
    string? Content,
    IReadOnlyList<Guid>? AttachmentIds = null,
    Guid? ClientMessageId = null,
    EncryptedPayloadRequest? Encrypted = null);

public sealed record FriendDto(UserDto User, DateTime Since);

/// <param name="User">The other person, whichever way the request points.</param>
/// <param name="Direction">"incoming" when the viewer may answer it, "outgoing" when they sent it.</param>
public sealed record FriendRequestDto(string Id, UserDto User, string Direction, DateTime SentAt);

/// <param name="LastMessage">The newest message or timeline line; null while there is none.</param>
/// <param name="Unread">Messages of the peer after the viewer's read marker.</param>
/// <param name="UpdatedAt">The last message's time, or the conversation's own while it is empty.</param>
public sealed record DirectConversationDto(
    string Id,
    UserDto Peer,
    DirectMessageDto? LastMessage,
    int Unread,
    DateTime UpdatedAt,
    ConversationEncryptionDto Encryption);

/// <param name="Blocked">Only the people the viewer blocked, not those who blocked the viewer.</param>
public sealed record SocialStateDto(
    IReadOnlyList<FriendDto> Friends,
    IReadOnlyList<FriendRequestDto> Requests,
    IReadOnlyList<UserDto> Blocked,
    IReadOnlyList<DirectConversationDto> Conversations);

/// <param name="Result">
/// "sent", "already-sent", "already-friends", "blocked", "self" or "unknown-user".
/// </param>
public sealed record FriendRequestResultDto(string Result);

public sealed record OpenedConversationDto(string Id);

/// <summary>One page of a conversation, oldest message first.</summary>
/// <param name="NextCursor">Opaque; null once the first message has been reached.</param>
public sealed record DirectMessageHistoryPageDto(IReadOnlyList<DirectMessageDto> Messages, string? NextCursor);

/// <summary>The ciphertext of an encrypted message, as the author's browser made it.</summary>
public sealed record EncryptedPayloadDto(string EpochId, byte[] Iv, byte[] Ciphertext);

/// <summary>A timeline line: what happened, and who did it.</summary>
/// <param name="Kind">One of the <see cref="DirectSystemKinds"/>.</param>
public sealed record DirectSystemEntryDto(string Kind, string? ActorId);

/// <summary>
/// A message of a private conversation, in the shape of a channel message so
/// that the same transcript shows both, with what only private ones have.
/// </summary>
/// <param name="ChannelId">The conversation.</param>
/// <param name="Content">Empty for an encrypted message and for a timeline line.</param>
/// <param name="Attachments">For an encrypted message, the opaque uploads it uses.</param>
/// <param name="Encrypted">Set for an encrypted message.</param>
/// <param name="System">Set for a timeline line, whose author is its actor.</param>
public sealed record DirectMessageDto(
    string Id,
    string ChannelId,
    UserDto Author,
    string Content,
    DateTime CreatedAt,
    IReadOnlyList<AttachmentDto> Attachments,
    IReadOnlyList<MessageEmbedDto> Embeds,
    DateTime? EditedAt,
    DateTime? PinnedAt,
    MessageReferenceDto? ReplyTo,
    IReadOnlyList<MessageReactionDto>? Reactions,
    EncryptedPayloadDto? Encrypted,
    DirectSystemEntryDto? System);

public static class SocialContractMappings
{
    /// <remarks>The message's author and attachments must be loaded.</remarks>
    public static DirectMessageDto ToDto(this DirectMessage message) => new(
        message.Id.ToString(),
        message.ConversationId.ToString(),
        message.Author.ToDto("online"),
        message.Content,
        message.CreatedAt.AsUtc(),
        message.Attachments.Select(x => x.Attachment.ToDto()).ToArray(),
        [],
        null,
        null,
        null,
        null,
        message is { EncryptedEpochId: { } epochId, EncryptedIv: { } iv, EncryptedCiphertext: { } ciphertext }
            ? new EncryptedPayloadDto(epochId.ToString(), iv, ciphertext)
            : null,
        message.SystemKind is { } kind ? new DirectSystemEntryDto(kind, message.AuthorId.ToString()) : null);
}
