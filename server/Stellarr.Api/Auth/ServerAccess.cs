using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;

namespace Stellarr.Api.Auth;

/// <summary>
/// What a member may do on a server: the privileges of the base role and of
/// every role they hold, narrowed or widened by the overrides set on a
/// section or a channel. <see cref="PermissionEvaluator"/> decides.
/// </summary>
public static class ServerAccess
{
    /// <summary>Names the server a request is about; without it the caller's first server is used.</summary>
    public const string HeaderName = "X-Stellarr-Server";

    internal const string ContextItemKey = "Stellarr.ActiveServerId";

    /// <summary>
    /// In the order lists are written in. The newer names come after the
    /// older ones, so a list a client already knows keeps its order.
    /// </summary>
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
        (ServerPrivileges.Administrator, "administrator"),
        (ServerPrivileges.SendMessages, "sendMessages"),
        (ServerPrivileges.AttachFiles, "attachFiles"),
        (ServerPrivileges.AddReactions, "addReactions"),
        (ServerPrivileges.Connect, "connect"),
        (ServerPrivileges.CreateInvite, "createInvite"),
        (ServerPrivileges.ViewChannel, "viewChannel"),
    ];

    /// <summary>
    /// The 403 for a member whom one permission stops, in their language, with
    /// the permission's wire name so a client can tell which one it was.
    /// </summary>
    public static IResult Refusal(Messages messages, MessageKey reason, ServerPrivileges missing) =>
        Results.Problem(
            statusCode: StatusCodes.Status403Forbidden,
            title: messages.Get(reason),
            extensions: new Dictionary<string, object?> { ["permission"] = Array.Find(Names, x => x.Privilege == missing).Name });

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

    /// <summary>The member's server-wide rights, through the shared authorization policy.</summary>
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

    /// <summary>
    /// The base role, which every member holds without a row and which people
    /// join with: the system role below the owner's that ranks last.
    /// <c>MemberPermissionsSchema</c> picks it with the same rule in SQL.
    /// </summary>
    public static async Task<ServerRole?> FindMemberRoleAsync(
        AppDbContext db,
        Guid serverId,
        CancellationToken cancellationToken = default) =>
        await db.ServerRoles.AsNoTracking()
            .Where(x => x.ServerId == serverId && x.IsSystem && !x.IsOwner)
            .OrderByDescending(x => x.Position)
            .ThenBy(x => x.Id)
            .FirstOrDefaultAsync(cancellationToken);

    /// <summary>The server the request is about; only available behind <see cref="ServerMembershipFilter"/>.</summary>
    public static Guid RequireActiveServerId(this HttpContext context) =>
        context.Items.TryGetValue(ContextItemKey, out var value) && value is Guid serverId
            ? serverId
            : throw new InvalidOperationException(
                "The active server is unavailable before ServerMembershipFilter runs.");

}
