using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Auth;

/// <summary>
/// What a member may do on a server: the privileges of the member's role,
/// narrowed or widened by the overrides set on a section or a channel.
/// </summary>
public static class ServerAccess
{
    /// <summary>Names the server a request is about; without it the caller's first server is used.</summary>
    public const string HeaderName = "X-Stellarr-Server";

    internal const string ContextItemKey = "Stellarr.ActiveServerId";

    private static readonly (ServerPrivileges Privilege, string Name)[] Names =
    [
        (ServerPrivileges.ManageServer, "manageServer"),
        (ServerPrivileges.ManageChannels, "manageChannels"),
        (ServerPrivileges.ManageRoles, "manageRoles"),
        (ServerPrivileges.ManageMembers, "manageMembers"),
        (ServerPrivileges.ManageMessages, "manageMessages"),
        (ServerPrivileges.ModerateMembers, "moderateMembers"),
        (ServerPrivileges.ManageBans, "manageBans"),
        (ServerPrivileges.ViewAuditLog, "viewAuditLog"),
    ];

    /// <summary>The wire names of the privileges set in <paramref name="privileges"/>.</summary>
    public static IReadOnlyList<string> ToNames(ServerPrivileges privileges) =>
        Names.Where(x => privileges.HasFlag(x.Privilege)).Select(x => x.Name).ToArray();

    /// <summary>Reads wire names back; false, with nothing granted, when any of them is unknown.</summary>
    public static bool TryParse(IEnumerable<string>? names, out ServerPrivileges privileges)
    {
        var parsed = ServerPrivileges.None;
        foreach (var name in names ?? [])
        {
            var match = Names.FirstOrDefault(x => x.Name == name);
            if (match.Name is null)
            {
                privileges = ServerPrivileges.None;
                return false;
            }

            parsed |= match.Privilege;
        }

        privileges = parsed;
        return true;
    }

    /// <summary>The role's server-wide rights, through the shared authorization policy.</summary>
    public static async Task<ServerPrivileges> GetAsync(
        AppDbContext db, Guid userId, Guid serverId, CancellationToken cancellationToken = default) =>
        await ServerAuthorization.EffectiveAsync(db, userId, PermissionScope.Server(serverId), cancellationToken)
            ?? ServerPrivileges.None;

    public static Task<bool> HasAsync(
        AppDbContext db, Guid userId, Guid serverId, ServerPrivileges privilege,
        CancellationToken cancellationToken = default) =>
        ServerAuthorization.AllowsAsync(db, userId, PermissionScope.Server(serverId), privilege, cancellationToken);

    public static Task<bool> HasAsync(
        AppDbContext db, Guid userId, Guid serverId, Guid channelId, ServerPrivileges privilege,
        CancellationToken cancellationToken = default) =>
        ServerAuthorization.AllowsAsync(db, userId, PermissionScope.Channel(serverId, channelId), privilege, cancellationToken);

    public static Task<bool> HasForSectionAsync(
        AppDbContext db, Guid userId, Guid serverId, Guid sectionId, ServerPrivileges privilege,
        CancellationToken cancellationToken = default) =>
        ServerAuthorization.AllowsAsync(db, userId, PermissionScope.Section(serverId, sectionId), privilege, cancellationToken);

    /// <summary>The role people get when they join: the system role below the owner's that ranks last.</summary>
    public static async Task<ServerRole?> FindMemberRoleAsync(
        AppDbContext db,
        Guid serverId,
        CancellationToken cancellationToken = default) =>
        await db.ServerRoles.AsNoTracking()
            .Where(x => x.ServerId == serverId && x.IsSystem && !x.IsOwner)
            .OrderByDescending(x => x.Position)
            .FirstOrDefaultAsync(cancellationToken);

    /// <summary>The server the request is about; only available behind <see cref="ServerMembershipFilter"/>.</summary>
    public static Guid RequireActiveServerId(this HttpContext context) =>
        context.Items.TryGetValue(ContextItemKey, out var value) && value is Guid serverId
            ? serverId
            : throw new InvalidOperationException(
                "The active server is unavailable before ServerMembershipFilter runs.");

}
