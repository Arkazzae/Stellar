namespace Stellarr.Api.Auth;

/// <summary>The resource whose permissions apply. A channel or section always names its server.</summary>
public readonly record struct PermissionScope
{
    private PermissionScope(Guid serverId, ScopeKind kind, Guid resourceId) =>
        (ServerId, Kind, ResourceId) = (serverId, kind, resourceId);

    public Guid ServerId { get; }
    internal ScopeKind Kind { get; }
    internal Guid ResourceId { get; }

    public static PermissionScope Server(Guid serverId) => new(serverId, ScopeKind.Server, serverId);
    public static PermissionScope Section(Guid serverId, Guid sectionId) => new(serverId, ScopeKind.Section, sectionId);
    public static PermissionScope Channel(Guid serverId, Guid channelId) => new(serverId, ScopeKind.Channel, channelId);

    internal enum ScopeKind { Server, Section, Channel }
}
