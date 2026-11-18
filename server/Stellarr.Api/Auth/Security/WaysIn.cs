using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Storage;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Auth.Security;

/// <summary>
/// The ways an account signs in on its own: its password, a passkey, or an
/// identity at a configured provider. An account made through a provider
/// starts with that identity alone, so removing it, or the passkey that
/// replaced it, could leave an account nobody can open but through a reset
/// by mail. The settings and Cockpit ask here first and refuse.
/// </summary>
public static class WaysIn
{
    /// <summary>
    /// Whether the account still has a way in once the identity or the method
    /// named is gone. A second-step method other than a passkey signs nobody in
    /// on its own, and an identity at a provider that is no longer configured
    /// has no button to press.
    /// </summary>
    public static async Task<bool> RemainAsync(
        AppDbContext db,
        ExternalOidcOptions oidc,
        User user,
        CancellationToken cancellationToken,
        Guid? withoutIdentity = null,
        Guid? withoutMethod = null)
    {
        if (user.PasswordHash is not null)
        {
            return true;
        }

        var passkey = await db.MfaMethods.AsNoTracking().AnyAsync(
            x => x.UserId == user.Id &&
                x.Id != withoutMethod &&
                x.Type == MfaMethodTypes.Passkey &&
                x.ConfirmedAt != null &&
                x.RevokedAt == null,
            cancellationToken);
        if (passkey)
        {
            return true;
        }

        return (await ProvidersAsync(db, oidc, user.Id, cancellationToken, withoutIdentity)).Count > 0;
    }

    /// <summary>
    /// The configured providers the account signs in through, each once and
    /// in the order they are configured. They are also the only ones that
    /// can confirm it is the account's owner with a fresh sign-in, so the
    /// settings offer exactly these.
    /// </summary>
    public static async Task<IReadOnlyList<string>> ProvidersAsync(
        AppDbContext db,
        ExternalOidcOptions oidc,
        Guid userId,
        CancellationToken cancellationToken,
        Guid? withoutIdentity = null)
    {
        var linked = (await db.ExternalIdentities.AsNoTracking()
                .Where(x => x.UserId == userId && x.Id != withoutIdentity)
                .Select(x => x.ProviderId)
                .ToListAsync(cancellationToken))
            .Select(ProviderOf)
            .ToHashSet(StringComparer.Ordinal);
        return oidc.ConfiguredProviders().Select(x => x.Id).Where(linked.Contains).ToArray();
    }

    /// <summary>
    /// The provider an identity signs in through: the one it was linked
    /// through, or the original one when the link names none.
    /// </summary>
    public static string ProviderOf(ExternalIdentity identity) => ProviderOf(identity.ProviderId);

    private static string ProviderOf(string? providerId) => providerId ?? ExternalOidcOptions.DefaultProviderId;

    /// <summary>
    /// Starts the transaction a removal that asks <see cref="RemainAsync"/>
    /// runs in, and holds the account's row until it ends. Two removals at
    /// once would otherwise each count the other's way in as left, and both
    /// go: the second now waits, and then counts what the first left.
    /// </summary>
    public static async Task<IDbContextTransaction> HoldAccountAsync(
        AppDbContext db,
        Guid userId,
        CancellationToken cancellationToken)
    {
        var transaction = await db.Database.BeginTransactionAsync(
            System.Data.IsolationLevel.Serializable, cancellationToken);
        await db.Database.ExecuteSqlInterpolatedAsync(
            $"UPDATE \"Users\" SET \"SessionVersion\" = \"SessionVersion\" WHERE \"Id\" = {userId};",
            cancellationToken);
        return transaction;
    }
}
