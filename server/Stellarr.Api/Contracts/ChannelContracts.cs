using Stellarr.Api.Auth;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Contracts;

public sealed record CreateChannelRequest(string? Name, string? Kind, Guid? SectionId = null);

/// <summary>Every field is optional; a field left out keeps its value.</summary>
public sealed record UpdateChannelRequest(
    string? Name,
    string? Topic,
    int? SlowModeSeconds,
    int? HideAfterInactivityHours,
    int? UserLimit);

/// <summary>
/// Every channel of the server in its new order. <c>Move</c>, when present,
/// also files one channel under a section in the same change, so a channel
/// dropped into another rail group never lands in one half of the move.
/// </summary>
public sealed record ReorderChannelsRequest(IReadOnlyList<Guid> ChannelIds, ChannelSectionMove? Move = null);

/// <summary>A null <c>SectionId</c> takes the channel out of its section.</summary>
public sealed record ChannelSectionMove(Guid ChannelId, Guid? SectionId);

/// <summary>
/// Hub payload: the server's channels in order, each with its section; on a
/// server where somebody is denied view, only those the recipient sees.
/// </summary>
public sealed record ChannelLayoutDto(string ServerId, IReadOnlyList<ChannelPlacementDto> Channels);

public sealed record ChannelPlacementDto(string Id, string? SectionId);

public sealed record CreateSectionRequest(string? Name);

public sealed record UpdateSectionRequest(string? Name);

public sealed record ReorderSectionsRequest(IReadOnlyList<Guid> SectionIds);

/// <summary>
/// Hub payload: the server's sections in their new order; on a server where
/// somebody is denied view, only those listed for the recipient.
/// </summary>
public sealed record SectionOrderDto(string ServerId, IReadOnlyList<string> SectionIds);

/// <summary>
/// Hub payload: the channels or sections the recipient sees on the server
/// have changed, and their client should read them again.
/// </summary>
public sealed record ChannelAccessChangedDto(string ServerId);

/// <summary>What one role is allowed and denied on top of its own permissions, in wire names.</summary>
public sealed record PermissionOverrideDto(
    string RoleId,
    IReadOnlyList<string> Allow,
    IReadOnlyList<string> Deny);

/// <summary>What one member is allowed and denied on top of what their roles give them there, in wire names.</summary>
public sealed record MemberPermissionOverrideDto(
    string UserId,
    IReadOnlyList<string> Allow,
    IReadOnlyList<string> Deny);

/// <summary>Both lists empty clears the override.</summary>
public sealed record SetPermissionOverrideRequest(IReadOnlyList<string>? Allow, IReadOnlyList<string>? Deny);

public static class PermissionOverrideContractMappings
{
    public static PermissionOverrideDto ToDto(this PermissionOverride permissionOverride) => new(
        permissionOverride.RoleId.ToString(),
        ServerAccess.ToNames(permissionOverride.AllowMask),
        ServerAccess.ToNames(permissionOverride.DenyMask));

    public static MemberPermissionOverrideDto ToDto(this MemberPermissionOverride permissionOverride) => new(
        permissionOverride.UserId.ToString(),
        ServerAccess.ToNames(permissionOverride.AllowMask),
        ServerAccess.ToNames(permissionOverride.DenyMask));
}
