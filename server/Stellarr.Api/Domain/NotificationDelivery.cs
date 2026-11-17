namespace Stellarr.Api.Domain;

/// <summary>What kind of activity a delivery is about.</summary>
public static class NotificationDeliveryKinds
{
    public const string Mention = "mention";
    public const string DirectMessage = "direct-message";
}

/// <summary>Where a delivery is going, independently of every other destination.</summary>
public static class NotificationDeliveryChannels
{
    public const string Email = "email";
    public const string Push = "push";
}

/// <summary>The life of one delivery row. A completed row is kept for a while for auditing and pruning.</summary>
public static class NotificationDeliveryStatuses
{
    public const string Pending = "pending";
    public const string Sent = "sent";
    public const string Cancelled = "cancelled";
    public const string Failed = "failed";
}

/// <summary>
/// One external delivery that a mention or a direct message asked for. A row
/// names exactly one destination: an e-mail address or one push subscription,
/// so a retry for one cannot resend another that already succeeded.
///
/// The row is written in the same transaction as the message and its inbox
/// entries, but nothing leaves the process there. A leased dispatcher reads it
/// later, rechecks every reason not to deliver it, and records the outcome.
/// Delivery is at least once: a crash after a provider accepted a message
/// leaves the row pending, and its stable e-mail idempotency key means the
/// provider can recognize the repeat.
/// </summary>
public sealed class NotificationDelivery
{
    public Guid Id { get; set; }

    /// <summary>The <see cref="Message.Id"/> or <see cref="DirectMessage.Id"/> that caused this row.</summary>
    public Guid SourceMessageId { get; set; }

    public Guid RecipientId { get; set; }

    /// <summary>One of <see cref="NotificationDeliveryKinds"/>.</summary>
    public required string EventKind { get; set; }

    /// <summary>One of <see cref="NotificationDeliveryChannels"/>.</summary>
    public required string Channel { get; set; }

    /// <summary>
    /// What makes this destination unique: the normalized address for e-mail,
    /// the subscription id for push. A row per destination means one failed
    /// delivery never resends a successful sibling.
    /// </summary>
    public required string DestinationKey { get; set; }

    /// <summary>The push subscription this row belongs to, or null for e-mail.</summary>
    public Guid? PushSubscriptionId { get; set; }

    public DateTime CreatedAt { get; set; }

    /// <summary>After this the row is completed as cancelled instead of delivered late.</summary>
    public DateTime ExpiresAt { get; set; }

    /// <summary>How many times a dispatcher has claimed the row.</summary>
    public int Attempts { get; set; }

    public DateTime NextAttemptAt { get; set; }

    public DateTime? LastAttemptAt { get; set; }

    public DateTime? SentAt { get; set; }

    /// <summary>One of <see cref="NotificationDeliveryStatuses"/>.</summary>
    public required string Status { get; set; }

    /// <summary>A lease held by one dispatcher while it works on the row.</summary>
    public Guid? LeaseId { get; set; }

    public DateTime? LeaseUntil { get; set; }

    /// <summary>A fixed, non-private note about the last failure; never an endpoint, key or body.</summary>
    public string LastError { get; set; } = "";

    public User Recipient { get; set; } = null!;
}
