using System.Security.Claims;
using Microsoft.AspNetCore.Mvc;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Options;
using Stellarr.Api.Auth;
using Stellarr.Api.Auth.Security;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Mail;
using Stellarr.Api.Secrets;

namespace Stellarr.Api.Notifications;

/// <summary>
/// The account-wide delivery settings behind the notification preferences:
/// whether mail is configured, whether this account has a verified address,
/// and whether browser push is available. These routes deliberately do not use
/// <c>ServerMembershipFilter</c>: notifications are addressed to the person,
/// not to the server they happen to be viewing.
/// </summary>
public static class PushNotificationEndpoints
{
    private const int MaxSubscriptionsPerUser = 10;

    public static IEndpointRouteBuilder MapPushNotificationEndpoints(this IEndpointRouteBuilder endpoints)
    {
        var account = endpoints.MapGroup("/api/me").RequireAuthorization();
        account.MapGet("/notification-delivery", GetDeliveryCapabilitiesAsync);
        account.MapPut("/push-subscriptions", UpsertSubscriptionAsync);
        account.MapDelete("/push-subscriptions", DeleteSubscriptionAsync);
        return endpoints;
    }

    private static async Task<IResult> GetDeliveryCapabilitiesAsync(
        HttpContext http,
        AppDbContext db,
        IConfiguration configuration,
        IOptions<MailOptions> mailOptions,
        IOptions<PushOptions> pushOptions,
        InstanceSecrets secrets,
        CancellationToken cancellationToken)
    {
        if (await FindActiveSignInAsync(http, db, cancellationToken) is not { } signIn)
        {
            return Results.Unauthorized();
        }

        var mailing = await AccountMail.FindAsync(
            db,
            configuration,
            mailOptions.Value,
            secrets,
            cancellationToken);
        var verifiedEmail = await SecurityEvents.VerifiedEmailAsync(db, signIn.UserId, cancellationToken);
        var vapid = PushVapidConfiguration.From(pushOptions.Value);

        http.Response.Headers.CacheControl = "no-store";
        return Results.Ok(new NotificationDeliveryDto(
            EmailAvailable: mailing is not null,
            EmailVerified: verifiedEmail is not null,
            PushAvailable: vapid is not null,
            ApplicationServerKey: vapid?.PublicKey));
    }

    private static async Task<IResult> UpsertSubscriptionAsync(
        HttpContext http,
        [FromBody] RegisterPushSubscriptionRequest? request,
        AppDbContext db,
        TimeProvider time,
        CancellationToken cancellationToken)
    {
        if (await FindActiveSignInAsync(http, db, cancellationToken) is not { } signIn)
        {
            return Results.Unauthorized();
        }

        if (!PushSubscriptionValidation.TryValidate(
                request?.Endpoint,
                request?.Keys?.P256dh,
                request?.Keys?.Auth,
                out var validated))
        {
            return InvalidSubscription();
        }

        var subscriptions = db.PushSubscriptions;
        var existing = await subscriptions.SingleOrDefaultAsync(
            x => x.Endpoint == validated.Endpoint,
            cancellationToken);
        if (existing is not null)
        {
            // An endpoint belongs to the account that created it. It may move
            // to a newer sign-in of that same account only when the old one
            // has ended: a browser that signs in again after a revocation can
            // still receive its own notifications.
            if (existing.UserId != signIn.UserId)
            {
                return Results.Conflict();
            }

            if (existing.SessionId != signIn.SessionId)
            {
                if (await IsActiveSessionAsync(
                        db,
                        existing.UserId,
                        existing.SessionId,
                        time.GetUtcNow().UtcDateTime,
                        cancellationToken))
                {
                    return Results.Conflict();
                }

                existing.SessionId = signIn.SessionId;
            }

            existing.P256dh = validated.P256dh;
            existing.Auth = validated.Auth;
            await db.SaveChangesAsync(cancellationToken);
            return Results.Ok(new PushSubscriptionDto(existing.Id.ToString()));
        }

        var owned = await subscriptions.CountAsync(x => x.UserId == signIn.UserId, cancellationToken);
        if (owned >= MaxSubscriptionsPerUser)
        {
            return Results.Conflict();
        }

        var subscription = new PushSubscriptionRecord
        {
            Id = Guid.NewGuid(),
            UserId = signIn.UserId,
            SessionId = signIn.SessionId,
            Endpoint = validated.Endpoint,
            P256dh = validated.P256dh,
            Auth = validated.Auth,
            CreatedAt = time.GetUtcNow().UtcDateTime,
        };
        subscriptions.Add(subscription);
        // A concurrent request can win the endpoint between the lookup and the
        // save. Its owner is the only one allowed to have it; a duplicate of
        // this same sign-in still gets the row that won, not a conflict.
        if (!await db.TrySaveChangesAsync(cancellationToken))
        {
            var winner = await subscriptions.AsNoTracking().SingleOrDefaultAsync(
                x => x.Endpoint == validated.Endpoint,
                cancellationToken);
            return winner is not null &&
                winner.UserId == signIn.UserId &&
                winner.SessionId == signIn.SessionId
                ? Results.Ok(new PushSubscriptionDto(winner.Id.ToString()))
                : Results.Conflict();
        }

        return Results.Ok(new PushSubscriptionDto(subscription.Id.ToString()));
    }

    private static async Task<IResult> DeleteSubscriptionAsync(
        HttpContext http,
        [FromBody] DeletePushSubscriptionRequest? request,
        AppDbContext db,
        CancellationToken cancellationToken)
    {
        if (await FindActiveSignInAsync(http, db, cancellationToken) is not { } signIn)
        {
            return Results.Unauthorized();
        }

        if (!PushSubscriptionValidation.TryNormalizeEndpoint(request?.Endpoint, out var endpoint))
        {
            return InvalidSubscription();
        }

        var subscriptions = db.PushSubscriptions;
        var subscription = await subscriptions.SingleOrDefaultAsync(
            x => x.Endpoint == endpoint &&
                x.UserId == signIn.UserId &&
                x.SessionId == signIn.SessionId,
            cancellationToken);
        if (subscription is not null)
        {
            subscriptions.Remove(subscription);
            await db.SaveChangesAsync(cancellationToken);
        }

        // No distinction between "removed" and "not yours": both leave the
        // caller with no subscription at that endpoint, and the second tells
        // nobody which account owns it.
        return Results.NoContent();
    }

    /// <summary>
    /// Authentication already checks the cookie; this also checks that its
    /// session row still exists, belongs to the account and has not ended.
    /// </summary>
    private static async Task<(Guid UserId, Guid SessionId)?> FindActiveSignInAsync(
        HttpContext http,
        AppDbContext db,
        CancellationToken cancellationToken)
    {
        if (!Guid.TryParse(http.User.FindFirstValue(ClaimTypes.NameIdentifier), out var userId) ||
            http.User.FindSessionId() is not { } sessionId)
        {
            return null;
        }

        var now = DateTime.UtcNow;
        var active = await IsActiveSessionAsync(db, userId, sessionId, now, cancellationToken);
        return active ? (userId, sessionId) : null;
    }

    private static Task<bool> IsActiveSessionAsync(
        AppDbContext db,
        Guid userId,
        Guid sessionId,
        DateTime now,
        CancellationToken cancellationToken) =>
        db.UserSessions.AsNoTracking().AnyAsync(
            x => x.Id == sessionId &&
                x.UserId == userId &&
                x.RevokedAt == null &&
                x.ExpiresAt > now &&
                (x.AbsoluteExpiresAt == null || x.AbsoluteExpiresAt > now),
            cancellationToken);

    private static IResult InvalidSubscription() =>
        Results.ValidationProblem(new Dictionary<string, string[]>
        {
            ["subscription"] = ["The push subscription endpoint or its keys are not valid."],
        });
}
