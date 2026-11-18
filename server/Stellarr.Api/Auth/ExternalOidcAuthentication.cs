using Microsoft.AspNetCore.Authentication;
using Microsoft.AspNetCore.Authentication.Cookies;
using Microsoft.AspNetCore.Authentication.OpenIdConnect;
using Microsoft.EntityFrameworkCore;
using Microsoft.IdentityModel.Protocols.OpenIdConnect;
using Microsoft.IdentityModel.Tokens;
using Stellarr.Api.Auth.Security;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;

namespace Stellarr.Api.Auth;

/// <summary>
/// Lets a person sign in through the configured OpenID Connect providers. A
/// provider only proves who they are: an account is signed in when that
/// identity was linked to it, by the account itself after a fresh proof or by
/// an operator, never by matching an e-mail address. The result is the same
/// revocable session a password sign-in gets.
/// </summary>
public static class ExternalOidcAuthentication
{
    private const string NotLinked = "external_identity_not_linked";

    public static AuthenticationBuilder AddExternalOidc(
        this AuthenticationBuilder authentication,
        ExternalOidcOptions settings)
    {
        foreach (var provider in settings.ConfiguredProviders())
        {
            AddProvider(authentication, provider);
        }

        return authentication;
    }

    private static void AddProvider(AuthenticationBuilder authentication, ExternalOidcProvider provider)
    {
        var clientSecret = provider.ResolveClientSecret();
        authentication.AddOpenIdConnect(
            provider.Scheme,
            provider.DisplayName,
            options =>
            {
                options.SignInScheme = CookieAuthenticationDefaults.AuthenticationScheme;
                options.Authority = provider.Authority.TrimEnd('/');
                options.ClientId = provider.ClientId;
                options.ClientSecret = clientSecret;
                options.CallbackPath = provider.CallbackPath;
                options.ResponseType = OpenIdConnectResponseType.Code;
                options.UsePkce = true;
                options.MapInboundClaims = false;
                options.GetClaimsFromUserInfoEndpoint = true;
                options.SaveTokens = false;
                options.Scope.Clear();
                options.Scope.Add(OpenIdConnectScope.OpenIdProfile);
                options.Scope.Add(OpenIdConnectScope.Email);
                options.TokenValidationParameters = new TokenValidationParameters
                {
                    NameClaimType = "preferred_username",
                    RoleClaimType = "groups",
                };
                options.Events = new OpenIdConnectEvents
                {
                    OnTokenValidated = context => SignInLinkedAccountAsync(context, provider),
                    OnRedirectToIdentityProvider = context =>
                    {
                        // Only a provider whose second step counts here is asked
                        // to sign the person in again when its session is older,
                        // and every provider when that sign-in is the proof.
                        var reauthenticate = IdentityLinks.Recall(context.Properties) is { Reauthenticate: true };
                        if (provider.TrustProviderMfa || reauthenticate)
                        {
                            context.ProtocolMessage.MaxAge = ((int)ExternalOidcProvider.ProviderMfaMaxAge.TotalSeconds)
                                .ToString(System.Globalization.CultureInfo.InvariantCulture);
                        }

                        // A sign-in asked for as proof asks for the credentials
                        // again even on a new session at the provider. What it
                        // proves is still read from auth_time, since a provider
                        // may not honour the prompt.
                        if (reauthenticate)
                        {
                            context.ProtocolMessage.Prompt = OpenIdConnectPrompt.Login;
                        }

                        return Task.CompletedTask;
                    },
                    OnRemoteFailure = RedirectToLoginAsync,
                };
            });
    }

    private static async Task SignInLinkedAccountAsync(TokenValidatedContext context, ExternalOidcProvider provider)
    {
        var issuer = ExternalOidcOptions.CanonicalIssuer(context.SecurityToken.Issuer ?? "");
        var subject = context.Principal?.FindFirst("sub")?.Value?.Trim() ?? "";
        // A link started from the settings never signs anyone in, and neither
        // does a fresh sign-in asked for as proof: both act on the session that
        // asked.
        if (IdentityLinks.Recall(context.Properties) is { } link)
        {
            var valid = issuer.Length is > 0 and <= 512 && subject.Length is > 0 and <= 255;
            if (link.Reauthenticate)
            {
                var confirmed = valid && await ReauthenticateAsync(
                    context.HttpContext, provider, link, issuer, subject,
                    context.Principal?.FindFirst("auth_time")?.Value);
                context.Response.Redirect(ReauthenticationPage(confirmed, context.Properties?.RedirectUri));
            }
            else
            {
                var result = valid
                    ? await LinkIdentityAsync(context.HttpContext, provider, link, issuer, subject)
                    : IdentityLinkOutcomes.Failed;
                context.Response.Redirect($"/link-account?result={result}");
            }

            context.HandleResponse();
            return;
        }

        if (issuer.Length is 0 or > 512 || subject.Length is 0 or > 255)
        {
            context.Fail("external_identity_invalid");
            return;
        }

        var db = context.HttpContext.RequestServices.GetRequiredService<AppDbContext>();
        var cancellationToken = context.HttpContext.RequestAborted;
        var userId = await db.ExternalIdentities.AsNoTracking()
            .Where(identity => identity.Issuer == issuer && identity.Subject == subject)
            .Select(identity => (Guid?)identity.UserId)
            .SingleOrDefaultAsync(cancellationToken);
        if (userId is null)
        {
            context.Fail(NotLinked);
            return;
        }

        var user = await db.Users.SingleOrDefaultAsync(candidate => candidate.Id == userId, cancellationToken);
        if (user is null || user.IsDisabled)
        {
            context.Fail("external_identity_unavailable");
            return;
        }

        var messages = context.HttpContext.RequestServices.GetRequiredService<Messages>();
        var policy = await MfaPolicy.LoadAsync(db, cancellationToken);
        var shortLived = IsShortLived(context.Properties);
        // Off unless the operator trusts this provider's own second step.
        var secondStepAtProvider = provider.PassedSecondStep(
            context.Principal?.FindAll("amr").Select(claim => claim.Value) ?? [],
            context.Principal?.FindFirst("auth_time")?.Value,
            DateTimeOffset.UtcNow);
        var start = await AuthenticationFlow.StartAsync(
            db, context.HttpContext, user, "oidc", policy, messages, cancellationToken,
            shortLived: shortLived, secondStepAtProvider: secondStepAtProvider);
        if (start.RecoveryRequired)
        {
            // The recovery is finished on the sign-in page, which has to know
            // the choice made before leaving for the provider.
            context.Response.Redirect(shortLived
                ? "/login?security_recovery_required=1&keep_signed_in=0"
                : "/login?security_recovery_required=1");
            context.HandleResponse();
            return;
        }

        if (start.Session is not { } session)
        {
            await db.SaveChangesAsync(cancellationToken);
            var page = start.Challenge?.Status == "mfa_enrollment_required"
                ? "/login?mfa_enrollment=1"
                : "/login?mfa_required=1";
            var returnUrl = context.Properties?.RedirectUri;
            if (!string.IsNullOrWhiteSpace(returnUrl))
            {
                page += $"&returnUrl={Uri.EscapeDataString(returnUrl)}";
            }

            context.Response.Redirect(page);
            context.HandleResponse();
            return;
        }

        db.UserSessions.Add(session);
        SecurityEvents.Record(
            db, user.Id, SecurityEventKinds.LoginSucceeded, context.HttpContext, "oidc",
            await SecurityEvents.VerifiedEmailAsync(db, user.Id, cancellationToken));
        await db.SaveChangesAsync(cancellationToken);

        context.Principal = SessionAuthentication.CreatePrincipal(user, session.Id);
        context.Properties ??= new AuthenticationProperties();
        SessionAuthentication.ApplyLifetime(context.Properties, shortLived);
    }

    private const string ShortLivedItem = "stellarr.short-lived";

    internal static void RememberShortLived(AuthenticationProperties properties, bool shortLived)
    {
        if (shortLived)
        {
            properties.Items[ShortLivedItem] = "true";
        }
    }

    private static bool IsShortLived(AuthenticationProperties? properties) =>
        properties?.Items.TryGetValue(ShortLivedItem, out var value) == true && value == "true";

    /// <summary>
    /// Links the identity to the account the link was started for, once the
    /// session named in the protected state is found still able to ask.
    /// </summary>
    private static async Task<string> LinkIdentityAsync(
        HttpContext context,
        ExternalOidcProvider provider,
        IdentityLinkRequest link,
        string issuer,
        string subject)
    {
        var db = context.RequestServices.GetRequiredService<AppDbContext>();
        var user = await AskingAccountAsync(db, provider, link, context.RequestAborted);
        return user is null
            ? IdentityLinkOutcomes.Failed
            : await IdentityLinking.LinkAsync(db, context, user, provider, issuer, subject, context.RequestAborted);
    }

    /// <summary>
    /// Counts a fresh sign-in at the provider as the session's primary proof
    /// again, when the identity that came back is the account's own and the
    /// provider says the person signed in within the last few minutes (it was
    /// asked to with <c>max_age</c>, and has to answer with <c>auth_time</c>).
    /// The session counts as signed in when the provider says, not when it
    /// answered: a sign-in there three minutes old leaves two of the five
    /// fresh minutes. A provider that waved the person through on an older
    /// session of its own proves nothing new and is refused.
    /// </summary>
    private static async Task<bool> ReauthenticateAsync(
        HttpContext context,
        ExternalOidcProvider provider,
        IdentityLinkRequest link,
        string issuer,
        string subject,
        string? authTime)
    {
        var now = DateTime.UtcNow;
        if (ExternalOidcProvider.SignedInAt(authTime) is not { } at || !ExternalOidcProvider.SignedInRecently(authTime, now))
        {
            return false;
        }

        // A provider's clock a little ahead counts as now; one whose sign-in
        // is already past the fresh window confirms nothing it could be used for.
        var signedIn = at.UtcDateTime < now ? at.UtcDateTime : now;
        if (signedIn < now - MfaPolicy.FreshMfaWindow)
        {
            return false;
        }

        var db = context.RequestServices.GetRequiredService<AppDbContext>();
        var cancellationToken = context.RequestAborted;
        if (await AskingAccountAsync(db, provider, link, cancellationToken) is not { } user ||
            !await db.ExternalIdentities.AsNoTracking().AnyAsync(
                x => x.UserId == user.Id && x.Issuer == issuer && x.Subject == subject,
                cancellationToken))
        {
            return false;
        }

        var session = await db.UserSessions.SingleOrDefaultAsync(
            x => x.Id == link.SessionId &&
                x.UserId == user.Id &&
                x.RevokedAt == null &&
                x.SecurityVersion == user.SessionVersion,
            cancellationToken);
        if (session is null)
        {
            return false;
        }

        // Never earlier than a sign-in the session already counts.
        if ((session.AuthenticatedAt ?? session.CreatedAt) < signedIn)
        {
            session.AuthenticatedAt = signedIn;
            await db.SaveChangesAsync(cancellationToken);
        }

        return true;
    }

    /// <summary>
    /// The account a round trip from the settings was started for. The
    /// provider's answer is a cross-site POST without the session cookie, so
    /// the session named in the protected state is checked here instead:
    /// still active, on the account's current security version, and the
    /// account neither disabled nor waiting for an operator's recovery.
    /// </summary>
    private static async Task<User?> AskingAccountAsync(
        AppDbContext db,
        ExternalOidcProvider provider,
        IdentityLinkRequest link,
        CancellationToken cancellationToken)
    {
        var now = DateTime.UtcNow;
        var user = await db.Users.SingleOrDefaultAsync(x => x.Id == link.UserId, cancellationToken);
        var sessionActive = user is not null && await db.UserSessions.AsNoTracking().AnyAsync(
            x => x.Id == link.SessionId &&
                x.UserId == link.UserId &&
                x.RevokedAt == null &&
                x.ExpiresAt > now &&
                x.AbsoluteExpiresAt > now &&
                x.SecurityVersion == link.SecurityVersion,
            cancellationToken);
        return user is null || !sessionActive || user.IsDisabled || user.SecurityRecoveryRequired ||
            user.SessionVersion != link.SecurityVersion || link.ProviderId != provider.Id
                ? null
                : user;
    }

    /// <summary>Where a fresh sign-in asked for as proof ends, with the page it was asked from.</summary>
    internal static string ReauthenticationPage(bool confirmed, string? returnUrl)
    {
        var page = $"/link-account?result={(confirmed ? "confirmed" : "unconfirmed")}";
        var back = ExternalOidcOptions.SafeReturnUrl(returnUrl);
        return back == "/" ? page : $"{page}&returnUrl={Uri.EscapeDataString(back)}";
    }

    /// <summary>Sends a failed or cancelled sign-in back to the login page, saying roughly why.</summary>
    private static Task RedirectToLoginAsync(RemoteFailureContext context)
    {
        if (IdentityLinks.Recall(context.Properties) is { } link)
        {
            var outcome = context.Request.Query["error"] == "access_denied" ? "cancelled" : IdentityLinkOutcomes.Failed;
            context.Response.Redirect(link.Reauthenticate
                ? ReauthenticationPage(confirmed: false, context.Properties?.RedirectUri)
                : $"/link-account?result={outcome}");
            context.HandleResponse();
            return Task.CompletedTask;
        }

        var error = context.Failure?.Message.Contains(NotLinked, StringComparison.Ordinal) == true
            ? "not_linked"
            : context.Request.Query["error"] == "access_denied"
                ? "cancelled"
                : "failed";
        context.Response.Redirect($"/login?oidc_error={Uri.EscapeDataString(error)}");
        context.HandleResponse();
        return Task.CompletedTask;
    }
}
