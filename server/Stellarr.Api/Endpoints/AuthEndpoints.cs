using Microsoft.AspNetCore.DataProtection;
using Microsoft.AspNetCore.Authentication;
using Microsoft.AspNetCore.Authentication.Cookies;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Auth.Security;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;
using Stellarr.Api.Realtime;

namespace Stellarr.Api.Endpoints;

public static class AuthEndpoints
{
    public static IEndpointRouteBuilder MapAuthEndpoints(this IEndpointRouteBuilder endpoints)
    {
        var group = endpoints.MapGroup("/api/auth");
        group.AddEndpointFilter(NoStoreAsync);

        group.MapPost("/login", LoginAsync).RequireRateLimiting(RateLimitPolicies.Login);
        group.MapPost("/logout", LogoutAsync).RequireAuthorization();
        group.MapGet("/oidc", GetOidcStatusAsync);
        group.MapGet("/oidc/pending", GetPendingIdentityAsync).RequireRateLimiting(RateLimitPolicies.LoginPoll);
        group.MapDelete("/oidc/pending", DropPendingIdentityAsync).RequireRateLimiting(RateLimitPolicies.AuthWrite);
        group.MapGet("/oidc/login", ChallengeOidc).RequireRateLimiting(RateLimitPolicies.Login);
        group.MapGet("/oidc/{id}/login", ChallengeNamedOidc).RequireRateLimiting(RateLimitPolicies.Login);
        group.MapGet("/oidc/link", (string? ticket, HttpContext context, ExternalOidcOptions options, IDataProtectionProvider protection) =>
            ChallengeLink(ExternalOidcOptions.DefaultProviderId, ticket, context, options, protection))
            .RequireRateLimiting(RateLimitPolicies.Login);
        group.MapGet("/oidc/{id}/link", ChallengeLink).RequireRateLimiting(RateLimitPolicies.Login);
        group.MapGet("/oidc/reauthenticate", (string? ticket, string? returnUrl, HttpContext context, ExternalOidcOptions options, IDataProtectionProvider protection) =>
            ChallengeReauthentication(ExternalOidcOptions.DefaultProviderId, ticket, returnUrl, context, options, protection))
            .RequireRateLimiting(RateLimitPolicies.Login);
        group.MapGet("/oidc/{id}/reauthenticate", ChallengeReauthentication).RequireRateLimiting(RateLimitPolicies.Login);

        return endpoints;
    }

    internal static async ValueTask<object?> NoStoreAsync(
        EndpointFilterInvocationContext context,
        EndpointFilterDelegate next)
    {
        context.HttpContext.Response.Headers.CacheControl = "no-store";
        context.HttpContext.Response.Headers.Pragma = "no-cache";
        return await next(context);
    }

    private static async Task<IResult> LoginAsync(
        LoginRequest request,
        AppDbContext db,
        PasswordService passwords,
        Messages messages,
        HttpContext context,
        CancellationToken cancellationToken)
    {
        // Either the username or the e-mail address, in any letter case.
        var identifier = (request.Username?.Trim() ?? "").ToUpperInvariant();
        var user = await db.Users.SingleOrDefaultAsync(
            x => x.NormalizedUsername == identifier || x.NormalizedEmail == identifier,
            cancellationToken);
        if (user is null || user.IsDisabled || !passwords.Verify(request.Password ?? "", user.PasswordHash))
        {
            if (user is not null)
            {
                SecurityEvents.Record(db, user.Id, SecurityEventKinds.LoginFailed, context, "password");
                await db.SaveChangesAsync(cancellationToken);
            }

            // The same answer for every kind of failure, slowed a little so
            // that how quickly it comes back says little about which one it was.
            await Task.Delay(Random.Shared.Next(80, 180), cancellationToken);
            return Results.Problem(
                statusCode: StatusCodes.Status401Unauthorized,
                title: messages.Get(MessageKey.SignInFailed));
        }

        var policy = await MfaPolicy.LoadAsync(db, cancellationToken);
        var shortLived = request.KeepSignedIn == false;
        // The identity to link is the one this browser waits with now; the
        // link itself waits for the whole sign-in, second step included.
        var pendingId = request.LinkPending
            ? (await PendingIdentities.FindAsync(db, context, cancellationToken))?.Id
            : null;
        var start = await AuthenticationFlow.StartAsync(
            db, context, user, "password", policy, messages, cancellationToken, shortLived: shortLived,
            pendingIdentityId: pendingId);
        if (start.RecoveryRequired)
        {
            return Results.Ok(new { status = "security_recovery_required" });
        }

        if (start.Session is { } session)
        {
            db.UserSessions.Add(session);
            SecurityEvents.Record(
                db, user.Id, SecurityEventKinds.LoginSucceeded, context, "password",
                await SecurityEvents.VerifiedEmailAsync(db, user.Id, cancellationToken));
            await db.SaveChangesAsync(cancellationToken);
            await SessionAuthentication.SignInAsync(context, user, session.Id, shortLived);
            if (!request.LinkPending)
            {
                return Results.Ok(user.ToDto("online"));
            }

            var linked = await IdentityLinking.LinkPendingAsync(db, context, user, pendingId, cancellationToken);
            return Results.Ok(new LinkedSignInDto(user.ToDto("online"), linked));
        }

        await db.SaveChangesAsync(cancellationToken);
        return Results.Ok(start.Challenge);
    }

    /// <summary>
    /// The providers the sign-in page offers, each with its button's name and
    /// address, and whether it may create accounts for people with an invite.
    /// <c>enabled</c> and <c>displayName</c> describe the first one, as
    /// clients from before several providers read them.
    /// </summary>
    private static async Task<IResult> GetOidcStatusAsync(
        ExternalOidcOptions options,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var allowing = await ProviderSignUp.AllowingAsync(db, cancellationToken);
        var providers = options.ConfiguredProviders()
            .Select(provider => new
            {
                id = provider.Id,
                displayName = string.IsNullOrWhiteSpace(provider.DisplayName)
                    ? messages.Get(MessageKey.OidcDefaultName)
                    : provider.DisplayName,
                loginUrl = provider.LoginPath,
                signUp = allowing.Contains(provider.Id),
            })
            .ToArray();
        return Results.Ok(new
        {
            enabled = providers.Length > 0,
            displayName = providers.FirstOrDefault()?.displayName ??
                (string.IsNullOrWhiteSpace(options.DisplayName)
                    ? messages.Get(MessageKey.OidcDefaultName)
                    : options.DisplayName),
            providers,
        });
    }

    /// <summary>
    /// The identity a provider confirmed that this browser waits with, as the
    /// sign-in page shows it: whose it is at which provider, and a login to
    /// suggest for a new account. It says nothing about the accounts here,
    /// not even whether the address belongs to one. 404 when there is none,
    /// it ran out or its provider is no longer configured.
    /// </summary>
    private static async Task<IResult> GetPendingIdentityAsync(
        HttpContext context,
        AppDbContext db,
        ExternalOidcOptions options,
        Messages messages,
        CancellationToken cancellationToken)
    {
        if (await PendingIdentities.FindAsync(db, context, cancellationToken) is not { } identity ||
            options.Find(identity.ProviderId) is not { } provider)
        {
            return Results.NotFound();
        }

        return Results.Ok(new PendingIdentityDto(
            new IdentityProviderDto(provider.Id, AccountSecurityEndpoints.ProviderName(provider, messages)),
            identity.DisplayName,
            identity.Email,
            PendingIdentities.SuggestUsername(identity),
            await ProviderSignUp.AllowedAsync(db, provider.Id, cancellationToken)));
    }

    /// <summary>Forgets the identity this browser waits with: the visitor went back to the sign-in page.</summary>
    private static async Task<IResult> DropPendingIdentityAsync(
        HttpContext context,
        AppDbContext db,
        CancellationToken cancellationToken)
    {
        await PendingIdentities.DropAsync(db, context, cancellationToken);
        return Results.NoContent();
    }

    /// <summary>The original address: the provider configured first.</summary>
    private static IResult ChallengeOidc(string? returnUrl, bool? keepSignedIn, ExternalOidcOptions options) =>
        Challenge(options.ConfiguredProviders() is [var first, ..] ? first : null, returnUrl, keepSignedIn);

    private static IResult ChallengeNamedOidc(
        string id,
        string? returnUrl,
        bool? keepSignedIn,
        ExternalOidcOptions options) =>
        Challenge(options.Find(id), returnUrl, keepSignedIn);

    /// <summary>
    /// Sends a signed-in account to the provider to link itself. The ticket
    /// must name this provider and this very session; the provider's answer
    /// then links instead of signing in.
    /// </summary>
    private static IResult ChallengeLink(
        string id,
        string? ticket,
        HttpContext context,
        ExternalOidcOptions options,
        IDataProtectionProvider protection)
    {
        if (options.Find(id) is not { } provider)
        {
            return Results.NotFound();
        }

        var request = IdentityLinks.ReadTicket(protection, ticket);
        if (request is not { Reauthenticate: false } || !TicketFitsSession(request, provider, context))
        {
            return Results.Redirect("/link-account?result=failed");
        }

        var properties = new AuthenticationProperties { RedirectUri = "/link-account?result=linked" };
        IdentityLinks.Remember(properties, request);
        return Results.Challenge(properties, [provider.Scheme]);
    }

    /// <summary>
    /// Sends a signed-in account to a provider it is linked to, to sign in
    /// there again as the proof a sensitive change asks for. The ticket is
    /// bound like a link's; the answer acts on this session and comes back to
    /// <paramref name="returnUrl"/> on this origin, through the page that says
    /// how it went.
    /// </summary>
    private static IResult ChallengeReauthentication(
        string id,
        string? ticket,
        string? returnUrl,
        HttpContext context,
        ExternalOidcOptions options,
        IDataProtectionProvider protection)
    {
        if (options.Find(id) is not { } provider)
        {
            return Results.NotFound();
        }

        var request = IdentityLinks.ReadTicket(protection, ticket);
        if (request is not { Reauthenticate: true } || !TicketFitsSession(request, provider, context))
        {
            return Results.Redirect(ExternalOidcAuthentication.ReauthenticationPage(confirmed: false, returnUrl));
        }

        var properties = new AuthenticationProperties { RedirectUri = ExternalOidcOptions.SafeReturnUrl(returnUrl) };
        IdentityLinks.Remember(properties, request);
        return Results.Challenge(properties, [provider.Scheme]);
    }

    /// <summary>A ticket names this provider and the very session presenting it.</summary>
    private static bool TicketFitsSession(IdentityLinkRequest request, ExternalOidcProvider provider, HttpContext context) =>
        request.ProviderId == provider.Id &&
        context.User.Identity?.IsAuthenticated == true &&
        context.User.RequireUserId() == request.UserId &&
        context.User.FindSessionId() == request.SessionId;

    private static IResult Challenge(ExternalOidcProvider? provider, string? returnUrl, bool? keepSignedIn)
    {
        if (provider is null)
        {
            return Results.NotFound();
        }

        var properties = new AuthenticationProperties { RedirectUri = ExternalOidcOptions.SafeReturnUrl(returnUrl) };
        // Kept in the protected state, so the choice comes back with the provider's answer.
        ExternalOidcAuthentication.RememberShortLived(properties, keepSignedIn == false);
        return Results.Challenge(properties, [provider.Scheme]);
    }

    private static async Task<IResult> LogoutAsync(
        HttpContext context,
        AppDbContext db,
        HubConnectionRegistry connections,
        CancellationToken cancellationToken)
    {
        if (context.User.FindSessionId() is { } sessionId)
        {
            var now = DateTime.UtcNow;
            var changed = await db.UserSessions.Where(x => x.Id == sessionId && x.RevokedAt == null)
                .ExecuteUpdateAsync(setters => setters.SetProperty(x => x.RevokedAt, now), cancellationToken);
            if (changed == 1)
            {
                SecurityEvents.Record(
                    db, context.User.RequireUserId(), SecurityEventKinds.SessionRevoked, context, "logout");
                await db.SaveChangesAsync(cancellationToken);
            }

            connections.AbortSession(sessionId);
        }

        await context.SignOutAsync(CookieAuthenticationDefaults.AuthenticationScheme);
        return Results.NoContent();
    }
}
