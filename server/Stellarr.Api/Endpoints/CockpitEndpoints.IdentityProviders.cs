using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;

namespace Stellarr.Api.Endpoints;

/// <summary>A sign-in provider as the Integrations page lists it.</summary>
/// <param name="SignUp">Whether it may create accounts for people with an invite.</param>
public sealed record CockpitIdentityProviderDto(string Id, string DisplayName, bool SignUp);

public sealed record CockpitProviderSignUpRequest(bool? SignUp);

// The sign-in providers of the instance. Which ones exist is the environment's
// business; whether each may create accounts is the owner's, here.
public static partial class CockpitEndpoints
{
    private static async Task<IResult> GetIdentityProvidersAsync(
        AppDbContext db,
        ExternalOidcOptions oidc,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var allowing = await ProviderSignUp.AllowingAsync(db, cancellationToken);
        return Results.Ok(oidc.ConfiguredProviders()
            .Select(provider => new CockpitIdentityProviderDto(
                provider.Id,
                AccountSecurityEndpoints.ProviderName(provider, messages),
                allowing.Contains(provider.Id)))
            .ToArray());
    }

    /// <summary>
    /// Lets a provider create accounts for people with an invite, or stops it.
    /// A new way to get an account on the instance is a security change, so
    /// it asks for a fresh second step like the others and is audited.
    /// </summary>
    private static async Task<IResult> SetProviderSignUpAsync(
        string providerId,
        CockpitProviderSignUpRequest request,
        HttpContext http,
        AppDbContext db,
        ExternalOidcOptions oidc,
        Messages messages,
        CancellationToken cancellationToken)
    {
        if (oidc.Find(providerId) is not { } provider)
        {
            return Results.NotFound();
        }

        if (request.SignUp is not { } signUp)
        {
            return Results.ValidationProblem(new Dictionary<string, string[]>
            {
                ["signUp"] = [messages.Get(MessageKey.CockpitProviderSignUpMissing)],
            });
        }

        var setting = await db.IdentityProviderSettings
            .SingleOrDefaultAsync(x => x.ProviderId == provider.Id, cancellationToken);
        if (setting is null)
        {
            setting = new IdentityProviderSetting { ProviderId = provider.Id };
            db.IdentityProviderSettings.Add(setting);
        }

        var name = AccountSecurityEndpoints.ProviderName(provider, messages);
        if (setting.SignUpAllowed != signUp)
        {
            await StageAuditAsync(
                db,
                http.RequireInstanceServer().Id,
                http.User.RequireUserId(),
                messages.Get(
                    signUp ? MessageKey.AuditProviderSignUpAllowed : MessageKey.AuditProviderSignUpStopped,
                    ("provider", name)),
                cancellationToken);
        }

        setting.SignUpAllowed = signUp;
        setting.UpdatedAt = DateTime.UtcNow;
        if (!await db.TrySaveChangesAsync(cancellationToken))
        {
            return Results.Conflict();
        }

        return Results.Ok(new CockpitIdentityProviderDto(provider.Id, name, signUp));
    }
}
