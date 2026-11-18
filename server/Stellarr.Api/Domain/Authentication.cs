namespace Stellarr.Api.Domain;

/// <summary>
/// A change of the account's address, waiting for the new address to prove
/// it receives mail. Only a digest of the link's token is stored.
/// </summary>
public sealed class EmailChangeRequest
{
    public Guid Id { get; set; }
    public Guid UserId { get; set; }
    /// <summary>Missing bindings identify legacy links, which cannot be confirmed.</summary>
    public Guid? SessionId { get; set; }
    public int? SecurityVersion { get; set; }
    public required string NewEmail { get; set; }
    public required string NormalizedNewEmail { get; set; }
    public required string TokenHash { get; set; }
    public DateTime CreatedAt { get; set; }
    public DateTime ExpiresAt { get; set; }
    public DateTime? UsedAt { get; set; }

    public User User { get; set; } = null!;
}

/// <summary>A one-time password recovery credential; only its SHA-256 digest is stored.</summary>
public sealed class PasswordResetToken
{
    public Guid Id { get; set; }
    public Guid UserId { get; set; }
    public required string TokenHash { get; set; }
    public DateTime CreatedAt { get; set; }
    public DateTime ExpiresAt { get; set; }
    public DateTime? UsedAt { get; set; }

    public User User { get; set; } = null!;
}

/// <summary>
/// A link between a local account and an OpenID Connect subject, made by
/// the account itself after a fresh proof, or by an operator. Issuer and
/// subject are the only stable identity key; the email address is neither
/// stored nor used to link accounts automatically.
/// </summary>
public sealed class ExternalIdentity
{
    public Guid Id { get; set; }
    public Guid UserId { get; set; }
    public required string Issuer { get; set; }
    public required string Subject { get; set; }

    /// <summary>The configured provider the link was made through, for its name; null for older links.</summary>
    public string? ProviderId { get; set; }

    public DateTime CreatedAt { get; set; }

    public User User { get; set; } = null!;
}

/// <summary>
/// An identity a provider confirmed that no account is linked to, kept for
/// a few minutes for the browser the provider sent back. That browser may
/// link it to the account it then signs in to with a password, or, with an
/// invite and a provider allowed to, make an account with it. Nothing links
/// it by its e-mail address. The browser holds the secret in an HttpOnly
/// cookie; only its digest is stored.
/// </summary>
public sealed class PendingExternalIdentity
{
    public Guid Id { get; set; }
    public required string SecretHash { get; set; }

    /// <summary>The configured provider it came back from.</summary>
    public required string ProviderId { get; set; }

    public required string Issuer { get; set; }
    public required string Subject { get; set; }

    /// <summary>The address as the provider gave it, and whether the provider said it is verified.</summary>
    public string? Email { get; set; }

    public bool EmailVerified { get; set; }

    /// <summary>The <c>name</c> the provider gave, to prefill a new account's display name.</summary>
    public string? DisplayName { get; set; }

    /// <summary>The provider's <c>preferred_username</c>, the first source of a suggested login.</summary>
    public string? PreferredUsername { get; set; }

    /// <summary>
    /// The provider's <c>amr</c>, comma-separated, and <c>auth_time</c>, so an
    /// account made from this identity signs in under the rules a sign-in
    /// through the provider follows, judged when the account is made.
    /// </summary>
    public string Amr { get; set; } = "";

    public string? AuthTime { get; set; }

    /// <summary>"Keep me signed in" was off when the person left for the provider.</summary>
    public bool ShortLived { get; set; }

    public DateTime CreatedAt { get; set; }
    public DateTime ExpiresAt { get; set; }
}

/// <summary>A revocable login, carried by one claim in the encrypted cookie.</summary>
public sealed class UserSession
{
    public Guid Id { get; set; }
    public Guid UserId { get; set; }
    public required string Device { get; set; }
    public required string IpAddress { get; set; }
    public DateTime CreatedAt { get; set; }
    public DateTime LastSeenAt { get; set; }
    public DateTime ExpiresAt { get; set; }

    /// <summary>The hard end of the session, which sliding renewal cannot pass.</summary>
    public DateTime? AbsoluteExpiresAt { get; set; }

    /// <summary>How the account proved itself first: password, oidc, passkey, desktop or registration.</summary>
    public string PrimarySource { get; set; } = "password";

    public DateTime? AuthenticatedAt { get; set; }

    /// <summary>When a real second factor was completed, not when trust was remembered.</summary>
    public DateTime? MfaCompletedAt { get; set; }

    /// <summary>The second-factor evidence, such as totp, email, recovery or trusted_device.</summary>
    public string? MfaEvidence { get; set; }

    /// <summary>The session version the assurance was recorded under.</summary>
    public int SecurityVersion { get; set; }

    /// <summary>
    /// Signed in without "keep me signed in": the cookie ends with the browser
    /// and the session after <see cref="Auth.SessionAuthentication.ShortLifetime"/>,
    /// even in a browser that restores its cookies.
    /// </summary>
    public bool ShortLived { get; set; }

    public DateTime? RevokedAt { get; set; }

    public User User { get; set; } = null!;
}

/// <summary>
/// A short-lived, single-use handoff from an authenticated system browser to
/// the cookie jar of a desktop client. Only a digest of the client's secret is
/// stored, and the browser never sees the secret or the final session cookie.
/// </summary>
public sealed class DesktopBrowserLogin
{
    public Guid Id { get; set; }
    public required string SecretHash { get; set; }

    /// <summary>The digest of the CSRF token in the explicit approval form.</summary>
    public string? ApprovalTokenHash { get; set; }

    /// <summary>The exact signed-in account and browser session the approval token is bound to.</summary>
    public Guid? ApprovalUserId { get; set; }
    public Guid? ApprovalSessionId { get; set; }

    /// <summary>The account that approved the login, or null while it is still pending.</summary>
    public Guid? UserId { get; set; }

    /// <summary>The browser session whose assurance is carried into the handoff.</summary>
    public Guid? ApprovedSessionId { get; set; }

    public string? PrimarySource { get; set; }
    public DateTime? MfaCompletedAt { get; set; }
    public string? MfaEvidence { get; set; }
    public int SecurityVersion { get; set; }

    public DateTime CreatedAt { get; set; }
    public DateTime ExpiresAt { get; set; }
    public DateTime? ApprovedAt { get; set; }
    public DateTime? RedeemedAt { get; set; }

    public User? User { get; set; }
}
