namespace Stellarr.Api.Domain;

public sealed class User
{
    public Guid Id { get; set; }
    public required string Username { get; set; }
    public required string NormalizedUsername { get; set; }
    public required string DisplayName { get; set; }

    /// <summary>
    /// Null for an account made through an identity provider, which signs in
    /// there until it sets a password of its own. A password sign-in to it
    /// fails the way a wrong password does.
    /// </summary>
    public string? PasswordHash { get; set; }

    public string? Email { get; set; }
    public string? NormalizedEmail { get; set; }

    /// <summary>
    /// The address, normalized, that an identity provider said was verified
    /// when the account was made through it. It counts as verified, like one
    /// confirmed by an e-mail code, for as long as it is still the account's
    /// address; a change of address clears it.
    /// </summary>
    public string? ProviderVerifiedEmail { get; set; }
    public bool IsDisabled { get; set; }

    /// <summary>Incremented when credentials change, which invalidates older cookies.</summary>
    public int SessionVersion { get; set; }

    /// <summary>
    /// Set by an operator CLI recovery. Ordinary primary flows cannot create a
    /// session or enrollment grant until a CLI credential has been redeemed and
    /// a replacement factor has been confirmed.
    /// </summary>
    public bool SecurityRecoveryRequired { get; set; }

    public required string AvatarColor { get; set; }
    public DateTime CreatedAt { get; set; }

    // The profile fields stay nullable: an account that never opened the
    // settings dialog is complete, and clients fall back to initials drawn on
    // AvatarColor.

    /// <summary>
    /// A path this server serves (<c>/assets/{id}</c> for an upload), never a
    /// foreign host, so a profile picture cannot point a reader's browser at
    /// somewhere we do not control. Built-in fallback art is derived from
    /// <see cref="Id"/> by the clients and is not stored.
    /// </summary>
    public string? AvatarUrl { get; set; }

    /// <summary>Same rules as <see cref="AvatarUrl"/>.</summary>
    public string? BannerUrl { get; set; }

    /// <summary>
    /// The picture the person set behind their own app, by the same rules as
    /// <see cref="AvatarUrl"/>. Unlike the avatar and the banner it is private:
    /// only its owner reads the file (by the uploader rule of
    /// <c>AttachmentAccess</c>), and the address goes into their own account
    /// and into nothing anybody else receives.
    /// </summary>
    public string? WallpaperUrl { get; set; }

    /// <summary>A short, public line about what the person is doing right now.</summary>
    public string? CustomStatus { get; set; }

    /// <summary>Free text the person wrote about themselves, stored verbatim.</summary>
    public string? Bio { get; set; }

    /// <summary>As typed ("ona/jej", "he/him"): printed, never parsed.</summary>
    public string? Pronouns { get; set; }

    /// <summary>
    /// Null for a person. Otherwise the account stands in for somebody whose
    /// messages an import brought from elsewhere, and this names where:
    /// <see cref="AccountOrigin.Discord"/>. Such an account has no password and
    /// no address, is disabled, belongs to no server, and nobody can find it,
    /// write to it or sign in as it. It is a column rather than a table of its
    /// own so that every place that maps an account, message authors included,
    /// can say so without another query.
    /// </summary>
    public string? Origin { get; set; }

    /// <summary>Whether this account only stands in for the author of imported messages.</summary>
    public bool IsPlaceholder => Origin is not null;

    public UserPreferenceSettings? Preferences { get; set; }
}

/// <summary>The values of <see cref="User.Origin"/>, as the API names them too.</summary>
public static class AccountOrigin
{
    /// <summary>The author of messages imported from a Discord server: <c>discord:{id}</c>.</summary>
    public const string Discord = "discord";

    /// <summary>The longest origin the column holds.</summary>
    public const int MaxLength = 16;
}

/// <summary>Account preferences that follow a person between devices.</summary>
public sealed class UserPreferenceSettings
{
    public Guid UserId { get; set; }
    /// <summary>Incoming calls pop up over other apps; mentions have their own switch.</summary>
    public bool DesktopNotifications { get; set; } = true;

    /// <summary>A mention pops up over other apps while the client is in the background.</summary>
    public bool MentionPopups { get; set; } = true;

    /// <summary>An incoming call rings while the client is open.</summary>
    public bool CallRingtone { get; set; } = true;
    public bool EmailNotifications { get; set; }
    public bool PushNotifications { get; set; }
    public bool MentionsOnly { get; set; } = true;
    public bool MessageSound { get; set; } = true;
    public bool JoinLeaveSound { get; set; } = true;
    public bool AllowDirectMessages { get; set; } = true;
    public bool AllowFriendRequests { get; set; } = true;
    public bool ShowPresence { get; set; } = true;

    /// <summary>The web client's "Copy ID" preference.</summary>
    public bool DeveloperMode { get; set; }

    public bool ReducedMotion { get; set; }

    /// <summary>JSON array of the text-channel ids this user muted.</summary>
    public string MutedChannelIdsJson { get; set; } = "[]";

    public int VadSensitivity { get; set; } = 60;

    /// <summary>
    /// Soundboard clip played when this person joins a voice room, or null to
    /// leave the shared arrival chime alone. Deliberately not a foreign key:
    /// the sound belongs to a server this account may later leave, and a
    /// selection that is no longer playable must read as silence instead of
    /// breaking the account it hangs off.
    /// </summary>
    public Guid? JoinSoundId { get; set; }

    /// <summary>
    /// The web client's own settings, one JSON object per section, written as
    /// the client sent them: how the app looks, the language it speaks, the
    /// call shortcuts, the voice processing, and what the client may share
    /// with outside services. The client validates every field on the way back
    /// in, so the server only checks the shape and the size. Null means no
    /// device has saved the section yet.
    /// </summary>
    public string? AppearanceSettingsJson { get; set; }

    /// <inheritdoc cref="AppearanceSettingsJson"/>
    public string? LanguageSettingsJson { get; set; }

    /// <inheritdoc cref="AppearanceSettingsJson"/>
    public string? KeybindSettingsJson { get; set; }

    /// <inheritdoc cref="AppearanceSettingsJson"/>
    public string? VoiceSettingsJson { get; set; }

    /// <inheritdoc cref="AppearanceSettingsJson"/>
    public string? PrivacySettingsJson { get; set; }

    public User User { get; set; } = null!;
}
