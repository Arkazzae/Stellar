namespace Stellarr.Api.Domain;

/// <summary>A symmetric friendship, stored once with the user ids in canonical order.</summary>
public sealed class Friendship
{
    public Guid Id { get; set; }
    public Guid UserAId { get; set; }
    public Guid UserBId { get; set; }
    public DateTime CreatedAt { get; set; }
}

/// <summary>A pending, directed friend request; an answered request is deleted.</summary>
public sealed class FriendRequest
{
    public Guid Id { get; set; }
    public Guid SenderId { get; set; }
    public Guid ReceiverId { get; set; }

    /// <summary>
    /// Both user ids in a fixed order, whichever way the request points, so one
    /// unique index allows a single pending request per pair of people.
    /// </summary>
    public required string PairKey { get; set; }

    public DateTime CreatedAt { get; set; }
}

/// <summary>A directed block. Blocking also removes the friendship and pending requests.</summary>
public sealed class UserBlock
{
    public Guid BlockerId { get; set; }
    public Guid BlockedId { get; set; }
    public DateTime CreatedAt { get; set; }
}

/// <summary>A private conversation between exactly two people, in canonical user-id order.</summary>
public sealed class DirectConversation
{
    public Guid Id { get; set; }
    public Guid UserAId { get; set; }
    public Guid UserBId { get; set; }
    public DateTime CreatedAt { get; set; }

    /// <summary>Off, asked for by one of the two, or on once the other confirmed.</summary>
    public DirectEncryptionState EncryptionState { get; set; }

    /// <summary>Who asked for encryption, while it is <see cref="DirectEncryptionState.Requested"/>.</summary>
    public Guid? EncryptionRequestedById { get; set; }

    public DateTime? EncryptionRequestedAt { get; set; }

    /// <summary>When encryption was last switched on, while it is <see cref="DirectEncryptionState.On"/>.</summary>
    public DateTime? EncryptionEnabledAt { get; set; }

    /// <summary>
    /// The epoch new messages must be encrypted with. Null while encryption is
    /// not on, and right after it was switched on, until the first epoch.
    /// </summary>
    public Guid? CurrentEpochId { get; set; }

    public ICollection<DirectMessage> Messages { get; set; } = [];
}

/// <summary>
/// A message, or a line of the timeline such as encryption being switched on.
/// An encrypted message keeps its content empty and carries the ciphertext;
/// a timeline line has a <see cref="SystemKind"/> and its actor as the author.
/// </summary>
public sealed class DirectMessage
{
    public Guid Id { get; set; }
    public Guid ConversationId { get; set; }
    public Guid AuthorId { get; set; }
    public required string Content { get; set; }
    public DateTime CreatedAt { get; set; }

    /// <summary>The epoch an encrypted message was encrypted with; null for a plain message.</summary>
    public Guid? EncryptedEpochId { get; set; }

    public byte[]? EncryptedIv { get; set; }
    public byte[]? EncryptedCiphertext { get; set; }

    /// <summary>One of <see cref="DirectSystemKinds"/> for a timeline line, null for a message.</summary>
    public string? SystemKind { get; set; }

    public User Author { get; set; } = null!;
    public ICollection<DirectMessageAttachment> Attachments { get; set; } = [];
}

public sealed class DirectMessageAttachment
{
    public Guid DirectMessageId { get; set; }
    public Guid AttachmentId { get; set; }

    public DirectMessage DirectMessage { get; set; } = null!;
    public ImageAttachment Attachment { get; set; } = null!;
}

/// <summary>A person's read watermark in one conversation, from which its unread count follows.</summary>
public sealed class DirectConversationRead
{
    public Guid ConversationId { get; set; }
    public Guid UserId { get; set; }
    public DateTime LastReadAt { get; set; }
}
