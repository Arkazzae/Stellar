using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;

namespace Stellarr.Api.Auth;

/// <summary>
/// Whether a provider may create accounts, as the owner set it in Cockpit.
/// It is read when it is needed, never cached, so turning it off stops the
/// next sign-up at once.
/// </summary>
public static class ProviderSignUp
{
    public static Task<bool> AllowedAsync(AppDbContext db, string providerId, CancellationToken cancellationToken) =>
        db.IdentityProviderSettings.AsNoTracking()
            .AnyAsync(x => x.ProviderId == providerId && x.SignUpAllowed, cancellationToken);

    /// <summary>The ids of the providers that may create accounts.</summary>
    public static async Task<HashSet<string>> AllowingAsync(AppDbContext db, CancellationToken cancellationToken) =>
        (await db.IdentityProviderSettings.AsNoTracking()
            .Where(x => x.SignUpAllowed)
            .Select(x => x.ProviderId)
            .ToListAsync(cancellationToken))
        .ToHashSet(StringComparer.Ordinal);
}
