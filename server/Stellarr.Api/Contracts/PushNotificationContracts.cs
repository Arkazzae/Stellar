namespace Stellarr.Api.Contracts;

/// <summary>
/// What the account's notification settings can offer right now. The public
/// key is the browser's subscription key; the private VAPID key is never part
/// of this response.
/// </summary>
public sealed record NotificationDeliveryDto(
    bool EmailAvailable,
    bool EmailVerified,
    bool PushAvailable,
    string? ApplicationServerKey);

public sealed record PushSubscriptionKeysDto(string? P256dh, string? Auth);

public sealed record RegisterPushSubscriptionRequest(string? Endpoint, PushSubscriptionKeysDto? Keys);

public sealed record DeletePushSubscriptionRequest(string? Endpoint);

public sealed record PushSubscriptionDto(string Id);
