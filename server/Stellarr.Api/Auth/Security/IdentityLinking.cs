using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Auth.Security;

/// <summary>How a link came out, in the words the web's link page reads.</summary>
public static class IdentityLinkOutcomes
{
    public const string Linked = "linked";

    /// <summary>The identity belongs to another account.</summary>
    public const string Taken = "taken";

    /// <summary>The account already has another identity at this provider.</summary>
    public const string Other = "other";

    /// <summary>Nothing could be linked: the round trip, the session or the waiting identity was gone.</summary>
    public const string Failed = "failed";
}

/// <summary>
/// The one place an identity at a provider is linked to an account, whichever
/// way the person got here: a link from the settings, a password sign-in that
/// asked to link the identity the provider just confirmed, or an account made
/// through the provider. The rules are the same everywhere: an identity
/// belongs to one account, and an account has one identity per provider.
/// </summary>
public static class IdentityLinking
{
    /// <summary>
    /// Links the identity and saves, or says why not. A link that loses a race
    /// to the same identity reads as taken; nothing else this context tracked
    /// is kept, so a caller saves what it must keep before it asks.
    /// </summary>
    public static async Task<string> LinkAsync(
        AppDbContext db,
        HttpContext context,
        User user,
        ExternalOidcProvider provider,
        string issuer,
        string subject,
        CancellationToken cancellationToken)
    {
        var existing = await db.ExternalIdentities.AsNoTracking()
            .Where(x => (x.Issuer == issuer && x.Subject == subject) || (x.UserId == user.Id && x.Issuer == issuer))
            .ToListAsync(cancellationToken);
        if (existing.Any(x => x.Issuer == issuer && x.Subject == subject && x.UserId == user.Id))
        {
            return IdentityLinkOutcomes.Linked;
        }

        if (existing.Count > 0)
        {
            return existing.Any(x => x.UserId != user.Id) ? IdentityLinkOutcomes.Taken : IdentityLinkOutcomes.Other;
        }

        Stage(db, context, user.Id, provider, issuer, subject,
            await SecurityEvents.VerifiedEmailAsync(db, user.Id, cancellationToken));
        try
        {
            await db.SaveChangesAsync(cancellationToken);
        }
        catch (DbUpdateException exception) when (DatabaseErrors.IsUniqueConstraint(exception))
        {
            // Linked by a request that won the race.
            db.ChangeTracker.Clear();
            return IdentityLinkOutcomes.Taken;
        }

        return IdentityLinkOutcomes.Linked;
    }

    /// <summary>
    /// Links the identity waiting for this browser to the account that has
    /// just signed in, as the sign-in asked. It runs once the session exists,
    /// so a refusal never undoes the sign-in; the waiting identity is used up
    /// either way. <paramref name="pendingId"/> is the identity the sign-in
    /// saw: one that replaced it since is not linked in its place.
    /// </summary>
    public static async Task<string> LinkPendingAsync(
        AppDbContext db,
        HttpContext context,
        User user,
        Guid? pendingId,
        CancellationToken cancellationToken)
    {
        if (pendingId is null ||
            await PendingIdentities.TakeAsync(db, context, cancellationToken, pendingId) is not { } pending ||
            context.RequestServices.GetRequiredService<ExternalOidcOptions>().Find(pending.ProviderId) is not { } provider)
        {
            SecurityCookies.ClearPendingIdentity(context);
            return IdentityLinkOutcomes.Failed;
        }

        return await LinkAsync(db, context, user, provider, pending.Issuer, pending.Subject, cancellationToken);
    }

    /// <summary>
    /// Adds the link and its security event without saving, for an account
    /// that is being created with it and cannot have a link yet.
    /// </summary>
    public static void Stage(
        AppDbContext db,
        HttpContext context,
        Guid userId,
        ExternalOidcProvider provider,
        string issuer,
        string subject,
        string? alertRecipient)
    {
        db.ExternalIdentities.Add(new ExternalIdentity
        {
            Id = Guid.NewGuid(),
            UserId = userId,
            Issuer = issuer,
            Subject = subject,
            ProviderId = provider.Id,
            CreatedAt = DateTime.UtcNow,
        });
        SecurityEvents.Record(
            db, userId, SecurityEventKinds.IdentityLinked, context,
            string.IsNullOrWhiteSpace(provider.DisplayName) ? provider.Id : provider.DisplayName,
            alertRecipient);
    }
}
