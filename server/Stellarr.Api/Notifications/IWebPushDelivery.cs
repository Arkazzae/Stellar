using Stellarr.Api.Domain;

namespace Stellarr.Api.Notifications;

/// <summary>The JSON body a service worker receives for one notification.</summary>
public sealed record NotificationPushPayload(
    string Id,
    string Title,
    string Body,
    string Url,
    string Tag);

/// <summary>What one delivery attempt means for the outbox that owns retries.</summary>
public enum PushDeliveryResult
{
    /// <summary>The push service accepted the notification, or the browser had no visible event to show.</summary>
    Delivered,

    /// <summary>The subscription or its keys will never work again; the outbox drops it.</summary>
    Gone,

    /// <summary>
    /// The provider refused this attempt, or it never arrived. The outbox
    /// bounds its retries; a subscription is kept unless the result is
    /// <see cref="Gone"/>.
    /// </summary>
    Retry,
}

/// <summary>Sends one already-persisted notification to one browser subscription.</summary>
public interface IWebPushDelivery
{
    Task<PushDeliveryResult> SendAsync(
        PushSubscriptionRecord subscription,
        NotificationPushPayload payload,
        CancellationToken cancellationToken);
}
