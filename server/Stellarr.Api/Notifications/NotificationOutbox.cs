using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth.Security;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Notifications;

/// <summary>
/// Writes the external deliveries a new mention or direct message asked for.
/// It runs inside the message's own transaction and only inserts rows; no
/// provider is contacted until <see cref="NotificationDeliveryDispatcher"/>
/// leases them after the commit.
///
/// Only a recipient who has opted in at this moment gets a row, and only for
/// destinations that are usable now: one row per live push subscription, or
/// one for the account's verified address. A later opt-in is never applied to
/// messages that were already sent.
/// </summary>
internal static class NotificationOutbox
{
    /// <summary>An undelivered activity notice stops being useful long before it is pruned.</summary>
    internal static readonly TimeSpan Lifetime = TimeSpan.FromDays(7);

    private const int MaxDestinationKeyLength = 320;

    internal static async Task EnqueueMentionsAsync(
        AppDbContext db,
        Message message,
        IReadOnlyList<UserNotification> notifications,
        CancellationToken cancellationToken)
    {
        foreach (var recipientId in notifications.Select(x => x.UserId).Distinct())
        {
            await EnqueueAsync(
                db,
                NotificationDeliveryKinds.Mention,
                message.Id,
                recipientId,
                message.CreatedAt,
                cancellationToken);
        }
    }

    internal static Task EnqueueDirectMessageAsync(
        AppDbContext db,
        DirectMessage message,
        Guid recipientId,
        CancellationToken cancellationToken) =>
        EnqueueAsync(
            db,
            NotificationDeliveryKinds.DirectMessage,
            message.Id,
            recipientId,
            message.CreatedAt,
            cancellationToken);

    private static async Task EnqueueAsync(
        AppDbContext db,
        string eventKind,
        Guid sourceMessageId,
        Guid recipientId,
        DateTime createdAt,
        CancellationToken cancellationToken)
    {
        var user = await db.Users.AsNoTracking()
            .Where(x => x.Id == recipientId)
            .Select(x => new { x.IsDisabled, x.SessionVersion })
            .SingleOrDefaultAsync(cancellationToken);
        if (user is null || user.IsDisabled)
        {
            return;
        }

        var preferences = await db.UserPreferences.AsNoTracking()
            .SingleOrDefaultAsync(x => x.UserId == recipientId, cancellationToken);
        if (preferences is null)
        {
            return;
        }

        if (preferences.EmailNotifications)
        {
            // The same check the rest of the account mail uses: an address
            // counts only while a confirmed e-mail method matches the
            // account's current one, and the method is not revoked.
            var verified = await SecurityEvents.VerifiedEmailAsync(db, recipientId, cancellationToken);
            if (verified is { Length: > 0 } email)
            {
                Add(
                    db,
                    eventKind,
                    sourceMessageId,
                    recipientId,
                    NotificationDeliveryChannels.Email,
                    Normalize(email),
                    pushSubscriptionId: null,
                    createdAt);
            }
        }

        if (!preferences.PushNotifications)
        {
            return;
        }

        var now = createdAt;
        var liveSessions = await db.UserSessions.AsNoTracking()
            .Where(x => x.UserId == recipientId &&
                x.RevokedAt == null &&
                x.ExpiresAt > now &&
                x.AbsoluteExpiresAt != null &&
                x.AbsoluteExpiresAt > now &&
                x.SecurityVersion == user.SessionVersion)
            .Select(x => x.Id)
            .ToArrayAsync(cancellationToken);
        if (liveSessions.Length == 0)
        {
            return;
        }

        var subscriptions = await db.PushSubscriptions.AsNoTracking()
            .Where(x => x.UserId == recipientId && liveSessions.Contains(x.SessionId))
            .Select(x => x.Id)
            .ToArrayAsync(cancellationToken);
        foreach (var subscriptionId in subscriptions)
        {
            Add(
                db,
                eventKind,
                sourceMessageId,
                recipientId,
                NotificationDeliveryChannels.Push,
                subscriptionId.ToString("N"),
                subscriptionId,
                createdAt);
        }
    }

    private static void Add(
        AppDbContext db,
        string eventKind,
        Guid sourceMessageId,
        Guid recipientId,
        string channel,
        string destinationKey,
        Guid? pushSubscriptionId,
        DateTime createdAt)
    {
        if (destinationKey.Length > MaxDestinationKeyLength)
        {
            return;
        }

        db.NotificationDeliveries.Add(new NotificationDelivery
        {
            Id = Guid.NewGuid(),
            SourceMessageId = sourceMessageId,
            RecipientId = recipientId,
            EventKind = eventKind,
            Channel = channel,
            DestinationKey = destinationKey,
            PushSubscriptionId = pushSubscriptionId,
            CreatedAt = createdAt,
            ExpiresAt = createdAt + Lifetime,
            Attempts = 0,
            NextAttemptAt = createdAt,
            Status = NotificationDeliveryStatuses.Pending,
        });
    }

    internal static string Normalize(string email) => email.Trim().ToUpperInvariant();
}
