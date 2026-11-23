using System.Text.Json;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Options;
using Stellarr.Api.Auth;
using Stellarr.Api.Auth.Security;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;
using Stellarr.Api.Mail;
using Stellarr.Api.Messaging;
using Stellarr.Api.Secrets;

namespace Stellarr.Api.Notifications;

/// <summary>
/// Delivers the activity outbox. A row is claimed with a short lease, every
/// reason not to deliver it is checked again outside the message transaction,
/// and only then is the provider called. Providers are never called while a
/// message is being stored.
///
/// The dispatcher is public and takes its clock, so tests can run a batch at a
/// chosen instant instead of waiting for the background worker.
/// </summary>
public sealed class NotificationDeliveryDispatcher(
    AppDbContext db,
    IConfiguration configuration,
    IOptions<MailOptions> mailOptions,
    InstanceSecrets secrets,
    IEmailDelivery emailDelivery,
    IWebPushDelivery webPushDelivery,
    Messages messages,
    ILogger<NotificationDeliveryDispatcher> logger)
{
    internal const int BatchSize = 25;
    internal const int MaxAttempts = 5;
    internal static readonly TimeSpan LeaseDuration = TimeSpan.FromMinutes(2);
    internal static readonly TimeSpan RetryBase = TimeSpan.FromMinutes(2);
    internal static readonly TimeSpan RetryCap = TimeSpan.FromMinutes(60);
    internal static readonly TimeSpan MissingMailDelay = TimeSpan.FromHours(1);
    internal static readonly TimeSpan Retention = TimeSpan.FromDays(30);

    private static readonly Action<ILogger, Guid, Exception?> DeliveryFailed = LoggerMessage.Define<Guid>(
        LogLevel.Warning,
        new EventId(1, "ActivityNotificationDeliveryFailed"),
        "An activity notification delivery {DeliveryId} failed.");

    /// <summary>
    /// Processes at most <see cref="BatchSize"/> due rows. Rows without a
    /// usable mail setup are delayed, not discarded, and no provider is called
    /// for them. Returns how many destinations accepted a delivery.
    /// </summary>
    public async Task<int> DispatchAsync(DateTime utcNow, CancellationToken cancellationToken)
    {
        await CompleteExpiredAsync(utcNow, cancellationToken);
        await PruneAsync(utcNow, cancellationToken);

        var dueEmail = await db.NotificationDeliveries.AsNoTracking()
            .AnyAsync(x => x.Status == NotificationDeliveryStatuses.Pending &&
                x.Channel == NotificationDeliveryChannels.Email &&
                x.NextAttemptAt <= utcNow &&
                x.ExpiresAt > utcNow &&
                x.Attempts < MaxAttempts &&
                (x.LeaseUntil == null || x.LeaseUntil <= utcNow), cancellationToken);
        var mailing = dueEmail
            ? await AccountMail.FindAsync(db, configuration, mailOptions.Value, secrets, cancellationToken)
            : null;
        if (dueEmail && mailing is null)
        {
            // The operator has not configured a sender, or turned mailing off.
            // Keep the rows for later without attempting anything external.
            await db.NotificationDeliveries
                .Where(x => x.Status == NotificationDeliveryStatuses.Pending &&
                    x.Channel == NotificationDeliveryChannels.Email &&
                    x.NextAttemptAt <= utcNow)
                .ExecuteUpdateAsync(
                    setters => setters.SetProperty(x => x.NextAttemptAt, utcNow + MissingMailDelay),
                    cancellationToken);
        }

        var leaseId = Guid.NewGuid();
        var claimed = await ClaimAsync(utcNow, leaseId, cancellationToken);
        var delivered = 0;
        foreach (var item in claimed)
        {
            cancellationToken.ThrowIfCancellationRequested();
            try
            {
                var target = await PrepareAsync(item, utcNow, cancellationToken);
                if (target is null)
                {
                    Complete(item, NotificationDeliveryStatuses.Cancelled, utcNow);
                }
                else if (item.Channel == NotificationDeliveryChannels.Email && mailing is null)
                {
                    // A row claimed in the same batch before the setup vanished.
                    Retry(item, utcNow);
                }
                else
                {
                    var outcome = await DeliverAsync(item, target, mailing, cancellationToken);
                    if (outcome == DeliveryOutcome.Sent)
                    {
                        Complete(item, NotificationDeliveryStatuses.Sent, utcNow);
                        delivered++;
                    }
                    else if (outcome == DeliveryOutcome.Gone)
                    {
                        Complete(item, NotificationDeliveryStatuses.Cancelled, utcNow);
                    }
                    else
                    {
                        Retry(item, utcNow);
                    }
                }
            }
            catch (Exception exception) when (exception is not OperationCanceledException)
            {
                // The exception is deliberately not logged or stored: its
                // message can name a push endpoint or another destination.
                Retry(item, utcNow);
                DeliveryFailed(logger, item.Id, null);
            }

            await db.SaveChangesAsync(cancellationToken);
        }

        return delivered;
    }

    private async Task<NotificationDelivery[]> ClaimAsync(
        DateTime utcNow,
        Guid leaseId,
        CancellationToken cancellationToken)
    {
        var ids = await db.NotificationDeliveries.AsNoTracking()
            .Where(x => x.Status == NotificationDeliveryStatuses.Pending &&
                x.NextAttemptAt <= utcNow &&
                x.ExpiresAt > utcNow &&
                x.Attempts < MaxAttempts &&
                (x.LeaseUntil == null || x.LeaseUntil <= utcNow))
            .OrderBy(x => x.NextAttemptAt)
            .ThenBy(x => x.CreatedAt)
            .Take(BatchSize)
            .Select(x => x.Id)
            .ToArrayAsync(cancellationToken);

        var leaseUntil = utcNow + LeaseDuration;
        foreach (var id in ids)
        {
            // Only one dispatcher can turn the row into its own: the condition
            // is repeated in the statement that takes the lease.
            await db.NotificationDeliveries
                .Where(x => x.Id == id &&
                    x.Status == NotificationDeliveryStatuses.Pending &&
                    x.NextAttemptAt <= utcNow &&
                    x.ExpiresAt > utcNow &&
                    x.Attempts < MaxAttempts &&
                    (x.LeaseUntil == null || x.LeaseUntil <= utcNow))
                .ExecuteUpdateAsync(
                    setters => setters
                        .SetProperty(x => x.LeaseId, leaseId)
                        .SetProperty(x => x.LeaseUntil, leaseUntil)
                        .SetProperty(x => x.LastAttemptAt, utcNow)
                        .SetProperty(x => x.Attempts, x => x.Attempts + 1),
                    cancellationToken);
        }

        return await db.NotificationDeliveries
            .Where(x => x.LeaseId == leaseId)
            .OrderBy(x => x.NextAttemptAt)
            .ThenBy(x => x.CreatedAt)
            .ToArrayAsync(cancellationToken);
    }

    /// <summary>
    /// The checks that must hold at delivery time, not only when the message
    /// was stored. Null means the row should be completed as cancelled.
    /// </summary>
    private async Task<DeliveryTarget?> PrepareAsync(
        NotificationDelivery item,
        DateTime utcNow,
        CancellationToken cancellationToken)
    {
        if (item.ExpiresAt <= utcNow)
        {
            return null;
        }

        var recipient = await db.Users.AsNoTracking()
            .Where(x => x.Id == item.RecipientId)
            .Select(x => new { x.IsDisabled, x.NormalizedUsername, x.SessionVersion })
            .SingleOrDefaultAsync(cancellationToken);
        if (recipient is null || recipient.IsDisabled)
        {
            return null;
        }

        var preferences = await db.UserPreferences.AsNoTracking()
            .SingleOrDefaultAsync(x => x.UserId == item.RecipientId, cancellationToken);
        if (preferences is null)
        {
            return null;
        }

        var wantsChannel = item.Channel == NotificationDeliveryChannels.Email
            ? preferences.EmailNotifications
            : preferences.PushNotifications;
        if (!wantsChannel)
        {
            return null;
        }

        if (item.EventKind == NotificationDeliveryKinds.Mention)
        {
            if (!await MentionStillWantedAsync(item, recipient.NormalizedUsername, preferences, cancellationToken))
            {
                return null;
            }
        }
        else if (item.EventKind == NotificationDeliveryKinds.DirectMessage)
        {
            if (!await DirectMessageStillWantedAsync(item, preferences, cancellationToken))
            {
                return null;
            }
        }
        else
        {
            return null;
        }

        if (item.Channel == NotificationDeliveryChannels.Email)
        {
            // The account must still have the same verified address. Changing
            // or revoking it completes the old row without sending.
            var verified = await SecurityEvents.VerifiedEmailAsync(db, item.RecipientId, cancellationToken);
            return verified is { Length: > 0 } email &&
                string.Equals(NotificationOutbox.Normalize(email), item.DestinationKey, StringComparison.Ordinal)
                ? new DeliveryTarget(email, null)
                : null;
        }

        if (item.PushSubscriptionId is not { } subscriptionId)
        {
            return null;
        }

        var subscription = await db.PushSubscriptions.AsNoTracking()
            .SingleOrDefaultAsync(x => x.Id == subscriptionId && x.UserId == item.RecipientId, cancellationToken);
        if (subscription is null ||
            !string.Equals(item.DestinationKey, subscription.Id.ToString("N"), StringComparison.Ordinal))
        {
            return null;
        }

        var sessionIsLive = await db.UserSessions.AsNoTracking()
            .AnyAsync(x => x.Id == subscription.SessionId &&
                x.UserId == subscription.UserId &&
                x.RevokedAt == null &&
                x.ExpiresAt > utcNow &&
                x.AbsoluteExpiresAt != null &&
                x.AbsoluteExpiresAt > utcNow &&
                x.SecurityVersion == recipient.SessionVersion,
                cancellationToken);
        return sessionIsLive ? new DeliveryTarget(null, subscription) : null;
    }

    private async Task<bool> MentionStillWantedAsync(
        NotificationDelivery item,
        string normalizedUsername,
        UserPreferenceSettings preferences,
        CancellationToken cancellationToken)
    {
        var message = await db.Messages.AsNoTracking()
            .Where(x => x.Id == item.SourceMessageId && !x.IsSystem)
            .Select(x => new { x.ChannelId, x.AuthorId, x.Content, x.CreatedAt })
            .SingleOrDefaultAsync(cancellationToken);
        if (message is null || message.AuthorId == item.RecipientId)
        {
            return false;
        }

        // The recipient must still see the channel: one they lost since the
        // message was sent tells them nothing of it either.
        if (!await ChannelVisibility.ViewsAsync(db, item.RecipientId, message.ChannelId, cancellationToken))
        {
            return false;
        }

        if (await BlockedAsync(item.RecipientId, message.AuthorId, cancellationToken))
        {
            return false;
        }

        if (IsMuted(preferences.MutedChannelIdsJson, message.ChannelId))
        {
            return false;
        }

        // A message the recipient has already read must not pop up later.
        if (await db.ChannelReads.AsNoTracking().AnyAsync(
                x => x.ChannelId == message.ChannelId &&
                    x.UserId == item.RecipientId &&
                    x.LastReadAt >= message.CreatedAt,
                cancellationToken))
        {
            return false;
        }

        // An edit may have removed the name; only a mention still in the
        // message is worth an external notice.
        return MessageMentions.ExtractNormalizedUsernames(message.Content).Contains(normalizedUsername);
    }

    private async Task<bool> DirectMessageStillWantedAsync(
        NotificationDelivery item,
        UserPreferenceSettings preferences,
        CancellationToken cancellationToken)
    {
        var message = await db.DirectMessages.AsNoTracking()
            .Where(x => x.Id == item.SourceMessageId)
            .Select(x => new { x.ConversationId, x.AuthorId, x.CreatedAt })
            .SingleOrDefaultAsync(cancellationToken);
        if (message is null)
        {
            return false;
        }

        var conversation = await db.DirectConversations.AsNoTracking()
            .Where(x => x.Id == message.ConversationId)
            .Select(x => new { x.UserAId, x.UserBId })
            .SingleOrDefaultAsync(cancellationToken);
        if (conversation is null)
        {
            return false;
        }

        var recipient = item.RecipientId;
        if ((conversation.UserAId != recipient && conversation.UserBId != recipient) ||
            message.AuthorId == recipient)
        {
            return false;
        }

        var peerId = conversation.UserAId == recipient ? conversation.UserBId : conversation.UserAId;
        if (peerId != message.AuthorId)
        {
            return false;
        }

        if (await BlockedAsync(recipient, message.AuthorId, cancellationToken))
        {
            return false;
        }

        if (!preferences.AllowDirectMessages)
        {
            var (a, b) = recipient.CompareTo(message.AuthorId) < 0
                ? (recipient, message.AuthorId)
                : (message.AuthorId, recipient);
            if (!await db.Friendships.AsNoTracking()
                    .AnyAsync(x => x.UserAId == a && x.UserBId == b, cancellationToken))
            {
                return false;
            }
        }

        if (await db.DirectConversationReads.AsNoTracking().AnyAsync(
                x => x.ConversationId == message.ConversationId &&
                    x.UserId == recipient &&
                    x.LastReadAt >= message.CreatedAt,
                cancellationToken))
        {
            return false;
        }

        return true;
    }

    private Task<bool> BlockedAsync(Guid first, Guid second, CancellationToken cancellationToken) =>
        db.UserBlocks.AsNoTracking().AnyAsync(x =>
            (x.BlockerId == first && x.BlockedId == second) ||
            (x.BlockerId == second && x.BlockedId == first), cancellationToken);

    private async Task<DeliveryOutcome> DeliverAsync(
        NotificationDelivery item,
        DeliveryTarget target,
        AccountMailing? mailing,
        CancellationToken cancellationToken)
    {
        if (item.Channel == NotificationDeliveryChannels.Email)
        {
            if (mailing is null || target.EmailAddress is not { } address)
            {
                return DeliveryOutcome.Retry;
            }

            var email = item.EventKind == NotificationDeliveryKinds.Mention
                ? ActivityEmail.Mention(
                    mailing.Sender, address, mailing.PublicUrl, item.Id, messages)
                : ActivityEmail.DirectMessage(
                    mailing.Sender, address, mailing.PublicUrl, item.Id, messages);
            await emailDelivery.SendAsync(email, mailing.Transport, cancellationToken);
            return DeliveryOutcome.Sent;
        }

        if (target.Subscription is not { } subscription)
        {
            return DeliveryOutcome.Retry;
        }

        var url = PublicUrl();
        var payload = new NotificationPushPayload(
            item.Id.ToString("N"),
            messages.Get(item.EventKind == NotificationDeliveryKinds.Mention
                ? MessageKey.NotificationMentionPushTitle
                : MessageKey.NotificationDirectMessagePushTitle),
            messages.Get(item.EventKind == NotificationDeliveryKinds.Mention
                ? MessageKey.NotificationMentionBody
                : MessageKey.NotificationDirectMessageBody),
            url,
            Tag: item.Id.ToString("N"));
        var result = await webPushDelivery.SendAsync(subscription, payload, cancellationToken);
        if (result == PushDeliveryResult.Gone)
        {
            // The browser dropped this subscription. It is no longer a
            // destination for any later message either.
            await db.PushSubscriptions
                .Where(x => x.Id == subscription.Id)
                .ExecuteDeleteAsync(cancellationToken);
        }

        return result switch
        {
            PushDeliveryResult.Delivered => DeliveryOutcome.Sent,
            PushDeliveryResult.Gone => DeliveryOutcome.Gone,
            _ => DeliveryOutcome.Retry,
        };
    }

    private string PublicUrl()
    {
        var configured = (configuration["Operations:PublicUrl"] ?? "").Trim().TrimEnd('/');
        return configured.Length == 0 ? "/" : $"{configured}/";
    }

    private static bool IsMuted(string mutedJson, Guid channelId)
    {
        try
        {
            var muted = JsonSerializer.Deserialize<string[]>(mutedJson) ?? [];
            return muted.Any(value =>
                Guid.TryParse(value, out var id) && id == channelId);
        }
        catch (JsonException)
        {
            return false;
        }
    }

    private static void Complete(NotificationDelivery item, string status, DateTime utcNow)
    {
        item.Status = status;
        item.LastError = "";
        item.LeaseId = null;
        item.LeaseUntil = null;
        if (status == NotificationDeliveryStatuses.Sent)
        {
            item.SentAt = utcNow;
        }
    }

    private static void Retry(NotificationDelivery item, DateTime utcNow)
    {
        item.Status = item.Attempts >= MaxAttempts
            ? NotificationDeliveryStatuses.Failed
            : NotificationDeliveryStatuses.Pending;
        item.LastError = "delivery failed";
        item.NextAttemptAt = utcNow + Backoff(item.Attempts);
        item.LeaseId = null;
        item.LeaseUntil = null;
    }

    private static TimeSpan Backoff(int attempts)
    {
        var exponent = Math.Clamp(attempts - 1, 0, 10);
        var delay = TimeSpan.FromMinutes(RetryBase.TotalMinutes * Math.Pow(2, exponent));
        return delay > RetryCap ? RetryCap : delay;
    }

    private Task<int> CompleteExpiredAsync(DateTime utcNow, CancellationToken cancellationToken) =>
        db.NotificationDeliveries
            .Where(x => x.Status == NotificationDeliveryStatuses.Pending && x.ExpiresAt <= utcNow)
            .ExecuteUpdateAsync(
                setters => setters
                    .SetProperty(x => x.Status, NotificationDeliveryStatuses.Cancelled)
                    .SetProperty(x => x.LeaseId, (Guid?)null)
                    .SetProperty(x => x.LeaseUntil, (DateTime?)null),
                cancellationToken);

    private Task<int> PruneAsync(DateTime utcNow, CancellationToken cancellationToken) =>
        db.NotificationDeliveries
            .Where(x => x.Status != NotificationDeliveryStatuses.Pending &&
                x.CreatedAt < utcNow - Retention)
            .ExecuteDeleteAsync(cancellationToken);

    private enum DeliveryOutcome
    {
        Sent,
        Retry,
        Gone,
    }

    private sealed record DeliveryTarget(string? EmailAddress, PushSubscriptionRecord? Subscription);
}
