using Stellarr.Api.Domain;

namespace Stellarr.Api.Contracts;

/// <param name="KeepSignedIn">False asks for a short-lived session; left out, the session is kept.</param>
/// <param name="LinkPending">
/// Links the identity a provider confirmed for this browser, which no account
/// is linked to, once this sign-in completes, second step and all.
/// </param>
public sealed record LoginRequest(string Username, string Password, bool? KeepSignedIn = null, bool LinkPending = false);

/// <summary>
/// A sign-in that asked to link the identity waiting for this browser, and
/// how the link came out, in <see cref="Auth.Security.IdentityLinkOutcomes"/>.
/// </summary>
public sealed record LinkedSignInDto(UserDto User, string IdentityLink);

public sealed record DesktopBrowserExchangeRequest(string RequestId, string Secret);

public sealed record SessionDto(UserDto? User);

public sealed record UserDto(
    string Id,
    string Username,
    string DisplayName,
    string AvatarColor,
    string Presence,
    // The profile, all optional. A client that predates these fields ignores
    // them; the web client treats null as "no upload" and picks a built-in
    // avatar and banner from the person's stable id.
    string? AvatarUrl = null,
    string? BannerUrl = null,
    string? CustomStatus = null,
    string? Bio = null,
    string? Pronouns = null,
    // Null for a person; "discord" for an account that only stands in for
    // the author of messages imported from Discord, which nobody can write
    // to or befriend. A client that predates it ignores it.
    string? Origin = null);

public sealed record TypingChangedDto(
    string ChannelId,
    string UserId,
    string DisplayName,
    DateTime ExpiresAt);

public static class AuthContractMappings
{
    public static UserDto ToDto(this User user, string presence = "offline") => new(
        user.Id.ToString(),
        user.Username,
        user.DisplayName,
        user.AvatarColor,
        presence,
        user.AvatarUrl,
        user.BannerUrl,
        user.CustomStatus,
        user.Bio,
        user.Pronouns,
        user.Origin);
}
