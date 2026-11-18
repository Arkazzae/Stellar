using Microsoft.AspNetCore.DataProtection;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Auth.Security;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;

namespace Stellarr.Api.Endpoints;

/// <summary>
/// The identity providers an account signs in with. Linking adds a way in,
/// and with a provider whose second step the instance trusts, one that skips
/// the local second step, so it asks for the same fresh proof as adding a
/// method; so does unlinking, which is refused when the identity is the last
/// way the account signs in. An account without a password proves itself
/// with a fresh sign-in at one of its providers once its own is a few
/// minutes old.
/// </summary>
public static partial class AccountSecurityEndpoints
{
    /// <summary>Checks the proof and answers with the address that starts the link, ticket included.</summary>
    private static async Task<IResult> StartIdentityLinkAsync(
        string providerId,
        IdentityChangeRequest? request,
        HttpContext context,
        AppDbContext db,
        ExternalOidcOptions oidc,
        IDataProtectionProvider protection,
        PasswordService passwords,
        Messages messages,
        CancellationToken cancellationToken)
    {
        if (oidc.Find(providerId) is not { } provider)
        {
            return Results.NotFound();
        }

        var userId = context.User.RequireUserId();
        var user = await db.Users.FindAsync([userId], cancellationToken);
        if (user is null)
        {
            return Results.NotFound();
        }

        if (!await CanChangeSecurityAsync(context, db, user, request?.Password, passwords, cancellationToken))
        {
            return Results.Problem(
                statusCode: StatusCodes.Status403Forbidden,
                title: messages.Get(MessageKey.SecurityFreshMfaRequired));
        }

        var ticket = IdentityLinks.IssueTicket(
            protection,
            new IdentityLinkRequest(user.Id, context.User.FindSessionId()!.Value, user.SessionVersion, provider.Id));
        return Results.Ok(new { url = $"{provider.LinkPath}?ticket={Uri.EscapeDataString(ticket)}" });
    }

    /// <summary>
    /// The address that sends the account to a provider it is linked to, to
    /// sign in there again. It needs no proof: the round trip is the proof,
    /// and only for this session.
    /// </summary>
    private static async Task<IResult> StartReauthenticationAsync(
        string providerId,
        HttpContext context,
        AppDbContext db,
        ExternalOidcOptions oidc,
        IDataProtectionProvider protection,
        CancellationToken cancellationToken)
    {
        var userId = context.User.RequireUserId();
        var user = await db.Users.FindAsync([userId], cancellationToken);
        if (oidc.Find(providerId) is not { } provider ||
            user is null ||
            !(await WaysIn.ProvidersAsync(db, oidc, userId, cancellationToken)).Contains(provider.Id))
        {
            return Results.NotFound();
        }

        var ticket = IdentityLinks.IssueTicket(
            protection,
            new IdentityLinkRequest(
                user.Id, context.User.FindSessionId()!.Value, user.SessionVersion, provider.Id, Reauthenticate: true));
        return Results.Ok(new { url = $"{provider.ReauthenticatePath}?ticket={Uri.EscapeDataString(ticket)}" });
    }

    private static async Task<IResult> RemoveIdentityAsync(
        Guid identityId,
        IdentityChangeRequest? request,
        HttpContext context,
        AppDbContext db,
        ExternalOidcOptions oidc,
        PasswordService passwords,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var userId = context.User.RequireUserId();
        await using var transaction = await WaysIn.HoldAccountAsync(db, userId, cancellationToken);
        var user = await db.Users.FindAsync([userId], cancellationToken);
        if (user is null)
        {
            return Results.NotFound();
        }

        if (!await CanChangeSecurityAsync(context, db, user, request?.Password, passwords, cancellationToken))
        {
            return Results.Problem(
                statusCode: StatusCodes.Status403Forbidden,
                title: messages.Get(MessageKey.SecurityFreshMfaRequired));
        }

        var identity = await db.ExternalIdentities
            .SingleOrDefaultAsync(x => x.Id == identityId && x.UserId == user.Id, cancellationToken);
        if (identity is null)
        {
            return Results.NotFound();
        }

        if (!await WaysIn.RemainAsync(db, oidc, user, cancellationToken, withoutIdentity: identity.Id))
        {
            return Results.Problem(
                statusCode: StatusCodes.Status409Conflict,
                title: messages.Get(MessageKey.SecurityLastWayInIdentity));
        }

        db.ExternalIdentities.Remove(identity);
        SecurityEvents.Record(
            db, user.Id, SecurityEventKinds.IdentityUnlinked, context, IdentityName(identity, oidc, messages),
            await SecurityEvents.VerifiedEmailAsync(db, user.Id, cancellationToken));
        await db.SaveChangesAsync(cancellationToken);
        await transaction.CommitAsync(cancellationToken);
        return Results.NoContent();
    }

    private static async Task<LinkedIdentityDto[]> IdentitiesAsync(
        AppDbContext db,
        ExternalOidcOptions oidc,
        Messages messages,
        Guid userId,
        CancellationToken cancellationToken)
    {
        var identities = await db.ExternalIdentities.AsNoTracking()
            .Where(x => x.UserId == userId)
            .OrderBy(x => x.CreatedAt)
            .ToArrayAsync(cancellationToken);
        return identities
            .Select(x => new LinkedIdentityDto(
                x.Id.ToString(), WaysIn.ProviderOf(x), IdentityName(x, oidc, messages), x.CreatedAt.AsUtc()))
            .ToArray();
    }

    /// <summary>The provider's name when it is still configured, else the issuer's host.</summary>
    internal static string IdentityName(ExternalIdentity identity, ExternalOidcOptions oidc, Messages messages) =>
        oidc.Find(WaysIn.ProviderOf(identity)) is { } provider
            ? ProviderName(provider, messages)
            : Uri.TryCreate(identity.Issuer, UriKind.Absolute, out var issuer) ? issuer.Host : identity.Issuer;

    internal static string ProviderName(ExternalOidcProvider provider, Messages messages) =>
        string.IsNullOrWhiteSpace(provider.DisplayName)
            ? messages.Get(MessageKey.OidcDefaultName)
            : provider.DisplayName;
}
