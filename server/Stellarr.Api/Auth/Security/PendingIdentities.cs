using System.Globalization;
using System.Text;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Auth.Security;

/// <summary>
/// The identity a provider confirmed that no account is linked to, waiting
/// for the browser it came back to. It lasts <see cref="Lifetime"/> and one
/// use: the row is bound to the browser by a secret in an HttpOnly cookie,
/// and it is deleted when it is taken or dropped, so a copy of the cookie
/// carried elsewhere opens nothing.
/// </summary>
public static class PendingIdentities
{
    public static readonly TimeSpan Lifetime = TimeSpan.FromMinutes(10);

    /// <summary>
    /// Keeps the identity for this browser, in place of anything it waited
    /// with before, and clears out identities nobody came back for.
    /// </summary>
    public static async Task KeepAsync(
        AppDbContext db,
        HttpContext context,
        PendingExternalIdentity identity,
        CancellationToken cancellationToken)
    {
        var now = DateTime.UtcNow;
        var previous = SecurityCookies.ReadPendingIdentity(context)?.Id;
        await db.PendingExternalIdentities
            .Where(x => x.ExpiresAt <= now || x.Id == previous)
            .ExecuteDeleteAsync(cancellationToken);

        var secret = SecurityTokens.Random(32);
        identity.Id = Guid.NewGuid();
        identity.SecretHash = SecurityTokens.Hash(secret);
        identity.CreatedAt = now;
        identity.ExpiresAt = now + Lifetime;
        db.PendingExternalIdentities.Add(identity);
        await db.SaveChangesAsync(cancellationToken);
        SecurityCookies.SetPendingIdentity(context, identity.Id, secret, identity.ExpiresAt);
    }

    /// <summary>The identity this browser waits with, left in place; null when there is none or it ran out.</summary>
    public static async Task<PendingExternalIdentity?> FindAsync(
        AppDbContext db,
        HttpContext context,
        CancellationToken cancellationToken)
    {
        if (SecurityCookies.ReadPendingIdentity(context) is not { } cookie)
        {
            return null;
        }

        var now = DateTime.UtcNow;
        var identity = await db.PendingExternalIdentities.AsNoTracking()
            .SingleOrDefaultAsync(x => x.Id == cookie.Id && x.ExpiresAt > now, cancellationToken);
        return identity is not null && SecurityTokens.Matches(cookie.Secret, identity.SecretHash) ? identity : null;
    }

    /// <summary>
    /// The identity this browser waits with, taken: only one request can take
    /// it, and the cookie goes with it. With <paramref name="expectedId"/>,
    /// only that identity is taken, as a challenge that carries it asks.
    /// </summary>
    public static async Task<PendingExternalIdentity?> TakeAsync(
        AppDbContext db,
        HttpContext context,
        CancellationToken cancellationToken,
        Guid? expectedId = null)
    {
        var identity = await FindAsync(db, context, cancellationToken);
        SecurityCookies.ClearPendingIdentity(context);
        if (identity is null || (expectedId is { } expected && identity.Id != expected))
        {
            return null;
        }

        var taken = await db.PendingExternalIdentities
            .Where(x => x.Id == identity.Id)
            .ExecuteDeleteAsync(cancellationToken);
        return taken == 1 ? identity : null;
    }

    /// <summary>Forgets the identity this browser waits with, if any.</summary>
    public static async Task DropAsync(AppDbContext db, HttpContext context, CancellationToken cancellationToken)
    {
        if (SecurityCookies.ReadPendingIdentity(context) is { } cookie)
        {
            var identity = await FindAsync(db, context, cancellationToken);
            if (identity is not null)
            {
                await db.PendingExternalIdentities
                    .Where(x => x.Id == cookie.Id)
                    .ExecuteDeleteAsync(cancellationToken);
            }
        }

        SecurityCookies.ClearPendingIdentity(context);
    }

    /// <summary>
    /// A login to offer for an account made from the identity: the provider's
    /// own login name when it makes one, else the name, else the part of the
    /// address before the @. Whether it is free is not asked, as that would
    /// tell anyone who has an account; a taken one is refused when sent.
    /// </summary>
    public static string SuggestUsername(PendingExternalIdentity identity) =>
        new[] { identity.PreferredUsername, identity.DisplayName, identity.Email?.Split('@')[0] }
            .Select(PlainLogin)
            .FirstOrDefault(login => login.Length >= 3) ?? "";

    /// <summary>
    /// The web's suggestion rule: lower case, the diacritics gone, spaces as
    /// dots, nothing but ASCII letters, digits and single separators between
    /// them, at most 32 characters.
    /// </summary>
    internal static string PlainLogin(string? text)
    {
        var decomposed = (text ?? "").ToLowerInvariant().Replace('\u0142', 'l').Normalize(NormalizationForm.FormD);
        var login = new StringBuilder();
        char? separator = null;
        foreach (var c in decomposed.Trim())
        {
            if (CharUnicodeInfo.GetUnicodeCategory(c) == UnicodeCategory.NonSpacingMark)
            {
                continue;
            }

            if (c is (>= 'a' and <= 'z') or (>= '0' and <= '9'))
            {
                if (separator is { } between && login.Length > 0)
                {
                    login.Append(between);
                }

                separator = null;
                login.Append(c);
            }
            else if (char.IsWhiteSpace(c) || c is '.' or '_' or '-')
            {
                // A run of separators counts as its first one.
                separator ??= char.IsWhiteSpace(c) ? '.' : c;
            }
        }

        var result = login.ToString();
        return result.Length <= 32 ? result : result[..32].TrimEnd('.', '_', '-');
    }
}
