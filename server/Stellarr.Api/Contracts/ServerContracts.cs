using System.Text.Json;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Contracts;

/// <param name="InitialServer">The server an empty instance starts with; ignored once it has an account.</param>
/// <param name="SetupCode">The code from the log that founds an empty instance; ignored once it has an account.</param>
public sealed record RegisterRequest(
    string Username,
    string DisplayName,
    string Email,
    string Password,
    string? InviteCode = null,
    CreateServerRequest? InitialServer = null,
    string? SetupCode = null);

public sealed record ServerDto(
    string Id,
    string Name,
    string Description,
    string? IconUrl,
    DateTime CreatedAt,
    bool IsPublic);

public sealed record ChannelDto(
    string Id,
    string Name,
    string Kind,
    int Position,
    string Topic,
    int SlowModeSeconds,
    int HideAfterInactivityHours,
    string AudioProfile,
    int UserLimit,
    string? SectionId);

public sealed record SectionDto(string Id, string Name, int Position);

/// <summary>A change to the server profile; a property left out stays as it is.</summary>
public sealed record UpdateServerRequest(string? Name, string? Description, bool? IsPublic);

public sealed record ServerIconDto(string? IconUrl);

/// <summary>
/// A new server and the few choices worth making before anybody is in it.
/// Everything after the description is optional and falls back to the usual
/// start, so a client that sends only a name still gets a working server.
/// </summary>
/// <param name="TextChannelName">The first text channel; named in the instance language when omitted.</param>
/// <param name="VoiceChannelName">The first voice channel; named in the instance language when omitted, none when empty.</param>
/// <param name="MentionsOnly">False makes every message notify by default.</param>
public sealed record CreateServerRequest(
    string Name,
    string? Description,
    bool IsPublic = false,
    string? TextChannelName = null,
    string? VoiceChannelName = null,
    bool? MentionsOnly = null,
    bool? VoiceJoinSound = null);

/// <summary>The server a person joined through an invite, and the channel the invite points to.</summary>
public sealed record JoinInviteResponseDto(
    string Id,
    string Name,
    string Description,
    string? IconUrl,
    DateTime CreatedAt,
    bool IsPublic,
    string? ChannelId,
    string? ChannelName);

public sealed record ServerInviteDto(
    string Id,
    string Url,
    string CreatedByName,
    int Uses,
    int? MaxUses,
    DateTime? ExpiresAt,
    string? ChannelId,
    string? ChannelName);

/// <param name="ChannelId">The channel a new member lands in; without one the invite is server-wide.</param>
public sealed record CreateInviteRequest(Guid? ChannelId);

/// <summary>
/// What anybody holding an invite link may learn before joining: where it
/// leads and who sent it. The sender appears as members see them, without
/// the username or id of their account, and nothing is said about the
/// server's members.
/// </summary>
/// <param name="Channel">The channel the invite points to; null for a server-wide invite.</param>
/// <param name="ExpiresAt">Null for an invite that does not expire.</param>
/// <remarks>The server's icon and the sender's avatar are null for a caller who is not signed in.</remarks>
public sealed record InvitePreviewDto(
    InvitePreviewServerDto Server,
    InvitePreviewChannelDto? Channel,
    InvitePreviewInviterDto Inviter,
    DateTime? ExpiresAt);

public sealed record InvitePreviewServerDto(string Id, string Name, string Description, string? IconUrl);

public sealed record InvitePreviewChannelDto(string Id, string Name);

public sealed record InvitePreviewInviterDto(string DisplayName, string AvatarColor, string? AvatarUrl);

public sealed record AuditEntryDto(string Id, string Summary, DateTime At);

public sealed record NotificationSettingsDto(
    bool AllMessages,
    bool MentionsOnly,
    bool VoiceJoinSound,
    IReadOnlyList<string> MutedChannelIds,
    bool ServerMuted);

/// <summary>A change to the notification defaults; a property left out stays as it is.</summary>
public sealed record UpdateNotificationSettingsRequest(
    bool? AllMessages,
    bool? MentionsOnly,
    bool? VoiceJoinSound,
    IReadOnlyList<Guid>? MutedChannelIds,
    bool? ServerMuted);

public sealed record TransferServerRequest(Guid UserId);

public static class ServerContractMappings
{
    public static ServerDto ToDto(this StellarrServer server) => new(
        server.Id.ToString(),
        server.Name,
        server.Description,
        server.IconUrl,
        server.CreatedAt.AsUtc(),
        server.IsPublic);

    public static ChannelDto ToDto(this Channel channel) => new(
        channel.Id.ToString(),
        channel.Name,
        channel.Kind == ChannelKind.Text ? "text" : "voice",
        channel.Position,
        channel.Topic,
        channel.SlowModeSeconds,
        channel.HideAfterInactivityHours,
        channel.AudioProfile,
        channel.UserLimit,
        channel.SectionId?.ToString());

    public static SectionDto ToDto(this Section section) => new(
        section.Id.ToString(),
        section.Name,
        section.Position);

    /// <remarks>The invite's channel must be loaded.</remarks>
    public static JoinInviteResponseDto ToJoinResponse(this StellarrServer server, ServerInvite invite) => new(
        server.Id.ToString(),
        server.Name,
        server.Description,
        server.IconUrl,
        server.CreatedAt.AsUtc(),
        server.IsPublic,
        invite.ChannelId?.ToString(),
        invite.Channel?.Name);

    /// <param name="withImages">
    /// Whether to name the icon and the avatar. They are served only to a
    /// session, so for anybody else their addresses would be broken images.
    /// </param>
    /// <remarks>The invite's author and channel must be loaded.</remarks>
    public static InvitePreviewDto ToPreview(this ServerInvite invite, StellarrServer server, bool withImages) => new(
        new InvitePreviewServerDto(
            server.Id.ToString(), server.Name, server.Description, withImages ? server.IconUrl : null),
        invite.Channel is { } channel ? new InvitePreviewChannelDto(channel.Id.ToString(), channel.Name) : null,
        new InvitePreviewInviterDto(
            invite.CreatedBy.DisplayName, invite.CreatedBy.AvatarColor, withImages ? invite.CreatedBy.AvatarUrl : null),
        invite.ExpiresAt?.AsUtc());

    /// <remarks>The invite's author and channel must be loaded.</remarks>
    public static ServerInviteDto ToDto(this ServerInvite invite, string origin) => new(
        invite.Id.ToString(),
        $"{origin}/invite/{invite.Code}",
        invite.CreatedBy.DisplayName,
        invite.Uses,
        invite.MaxUses,
        invite.ExpiresAt,
        invite.ChannelId?.ToString(),
        invite.Channel?.Name);

    public static AuditEntryDto ToDto(this ServerAuditEntry entry) => new(
        entry.Id.ToString(),
        entry.Summary,
        entry.CreatedAt);

    public static NotificationSettingsDto ToDto(this ServerAdminSettings settings) => new(
        settings.AllMessages,
        settings.MentionsOnly,
        settings.VoiceJoinSound,
        ReadMutedChannelIds(settings),
        settings.ServerMuted);

    private static string[] ReadMutedChannelIds(ServerAdminSettings settings)
    {
        try
        {
            return JsonSerializer.Deserialize<string[]>(settings.MutedChannelIdsJson) ?? [];
        }
        catch (JsonException)
        {
            // A damaged list must not make the settings unreadable.
            return [];
        }
    }
}
