using Microsoft.AspNetCore.SignalR;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;
using Stellarr.Api.Realtime;

namespace Stellarr.Api.Endpoints;

/// <summary>
/// The roles of a server and who holds them. A member holds any number of
/// roles besides the base role, which everybody holds. Nobody can hand out,
/// or take away, a power they do not have themselves.
/// </summary>
public static class RoleEndpoints
{
    private const int MaxNameLength = 64;
    private const int MaxColorLength = 48;

    internal sealed record RoleDraft(string Name, string Color, ServerPrivileges Permissions);

    public static IEndpointRouteBuilder MapRoleEndpoints(this IEndpointRouteBuilder endpoints)
    {
        var admin = endpoints.MapGroup("/api/admin")
            .RequireAuthorization()
            .AddEndpointFilter<ServerMembershipFilter>();
        admin.MapGet("/roles", ListRolesAsync);
        // On the instance server, as in Cockpit, granting powers needs a fresh second step.
        admin.MapPost("/roles", CreateRoleAsync).AddEndpointFilter<InstanceServerFreshSecondStepFilter>();
        admin.MapPut("/roles/order", ReorderRolesAsync).AddEndpointFilter<InstanceServerFreshSecondStepFilter>();
        admin.MapPatch("/roles/{roleId:guid}", UpdateRoleAsync).AddEndpointFilter<InstanceServerFreshSecondStepFilter>();
        admin.MapDelete("/roles/{roleId:guid}", DeleteRoleAsync).AddEndpointFilter<InstanceServerFreshSecondStepFilter>();
        admin.MapGet("/members", ListMembersAsync);
        admin.MapPatch("/members/{userId:guid}/role", AssignRoleAsync).AddEndpointFilter<InstanceServerFreshSecondStepFilter>();
        admin.MapPut("/members/{userId:guid}/roles", SetRolesAsync).AddEndpointFilter<InstanceServerFreshSecondStepFilter>();
        return endpoints;
    }

    private static async Task<IResult> ListRolesAsync(HttpContext http, AppDbContext db, CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        var roles = await db.ServerRoles.AsNoTracking()
            .Where(x => x.ServerId == serverId)
            .OrderBy(x => x.Position)
            .ToListAsync(cancellationToken);
        var held = await HeldRoles.LoadAsync(db, serverId, cancellationToken);
        return Results.Ok(roles.Select(x => x.ToDto(held.CountHolders(x.Id))));
    }

    private static async Task<IResult> ListMembersAsync(
        HttpContext http,
        AppDbContext db,
        PresenceTracker presence,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        var viewerId = http.User.RequireUserId();
        var members = await db.ServerMembers.AsNoTracking()
            .Include(x => x.User)
            .Where(x => x.ServerId == serverId)
            .OrderBy(x => x.User.DisplayName)
            .ToListAsync(cancellationToken);
        var held = await HeldRoles.LoadAsync(db, serverId, cancellationToken);
        return Results.Ok(members.Select(x => new ServerMemberDto(
            x.User.ToDto(presence.PublicPresenceOf(x.UserId, viewerId)),
            x.RoleId.ToString(),
            x.JoinedAt,
            held.Of(x.UserId).Select(role => role.Id.ToString()).ToArray())));
    }

    private static async Task<IResult> CreateRoleAsync(
        CreateRoleRequest request,
        HttpContext http,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        var serverId = http.RequireActiveServerId();
        var held = await ServerAccess.GetAsync(db, http.User.RequireUserId(), serverId, cancellationToken);
        if (!held.HasFlag(ServerPrivileges.ManageRoles))
        {
            return Results.Forbid();
        }

        if (Validate(request.Name, request.Color, request.Permissions) is not { } draft)
        {
            return InvalidRole(messages);
        }

        if (await db.ServerRoles.AnyAsync(x => x.ServerId == serverId && x.Name == draft.Name, cancellationToken))
        {
            return Results.Conflict();
        }

        if (!await ServerAuthorization.CanCreateRoleAsync(db, http.User.RequireUserId(), serverId, draft.Permissions, cancellationToken))
        {
            return Results.Forbid();
        }

        var role = new ServerRole
        {
            Id = Guid.NewGuid(),
            ServerId = serverId,
            Name = draft.Name,
            Color = draft.Color,
            Permissions = draft.Permissions,
            Position = await RoleRanks.PlaceNewAsync(db, serverId, cancellationToken),
        };
        db.ServerRoles.Add(role);
        await ServerAudit.StageAsync(db, serverId, http.User.RequireUserId(), new AuditEvent(
            "role.create",
            messages.Get(MessageKey.AuditRoleCreated, ("name", role.Name)),
            "role", role.Id, role.Name,
            ServerAudit.Only(
                new AuditChange("color", To: role.Color, ToColor: role.Color),
                role.Permissions == ServerPrivileges.None
                    ? null
                    : new AuditChange("permissions", Added: ServerAccess.ToNames(role.Permissions)))), cancellationToken);
        if (!await db.TrySaveChangesAsync(cancellationToken))
        {
            return Results.Conflict();
        }

        await transaction.CommitAsync(cancellationToken);
        return Results.Created($"/api/admin/roles/{role.Id}", role.ToDto(0));
    }

    private static async Task<IResult> UpdateRoleAsync(
        Guid roleId,
        UpdateRoleRequest request,
        HttpContext http,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        var serverId = http.RequireActiveServerId();
        var held = await ServerAccess.GetAsync(db, http.User.RequireUserId(), serverId, cancellationToken);
        if (!held.HasFlag(ServerPrivileges.ManageRoles))
        {
            return Results.Forbid();
        }

        var viewChange = await ViewChange.BeginAsync(db, serverId, cancellationToken);
        var role = await db.ServerRoles.FirstOrDefaultAsync(
            x => x.Id == roleId && x.ServerId == serverId, cancellationToken);
        if (role is null)
        {
            return Results.NotFound();
        }

        if (role.IsOwner)
        {
            return Results.Conflict();
        }

        var draft = Validate(
            request.Name ?? role.Name,
            request.Color ?? role.Color,
            request.Permissions ?? ServerAccess.ToNames(role.Permissions));
        if (draft is null)
        {
            return InvalidRole(messages);
        }

        if (await db.ServerRoles.AnyAsync(
                x => x.ServerId == serverId && x.Id != role.Id && x.Name == draft.Name, cancellationToken))
        {
            return Results.Conflict();
        }

        if (!await ServerAuthorization.CanEditRoleAsync(db, http.User.RequireUserId(), role, draft.Permissions, cancellationToken))
        {
            return Results.Forbid();
        }

        var before = (role.Name, role.Color, role.Permissions);
        role.Name = draft.Name;
        role.Color = draft.Color;
        role.Permissions = draft.Permissions;
        var granted = ServerAccess.ToNames(role.Permissions & ~before.Permissions);
        var revoked = ServerAccess.ToNames(before.Permissions & ~role.Permissions);
        var changes = ServerAudit.Only(
            ServerAudit.Changed("name", before.Name, role.Name),
            before.Color == role.Color ? null : new AuditChange("color", before.Color, role.Color, FromColor: before.Color, ToColor: role.Color),
            granted.Count + revoked.Count == 0 ? null : new AuditChange("permissions", Added: granted, Removed: revoked));
        if (changes.Count > 0)
        {
            await ServerAudit.StageAsync(db, serverId, http.User.RequireUserId(), new AuditEvent(
                "role.update",
                messages.Get(MessageKey.AuditRoleUpdated, ("name", role.Name)),
                "role", role.Id, role.Name, changes), cancellationToken);
        }

        if (!await db.TrySaveChangesAsync(cancellationToken))
        {
            return Results.Conflict();
        }

        await transaction.CommitAsync(cancellationToken);
        // First, on its own: a caller who hangs up must not keep anybody from
        // learning that they see something else now.
        await viewChange.PublishAsync(hub.Clients, db);
        var holders = await HeldRoles.LoadAsync(db, serverId, cancellationToken);
        await NotifyAsync(hub, db, serverId, holders.Holders(role.Id), cancellationToken);
        return Results.Ok(role.ToDto(holders.CountHolders(role.Id)));
    }

    /// <summary>
    /// Sets the order of the roles between the owner role and the base role,
    /// which is their rank. Roles that keep their place keep their position,
    /// so ties the actor may not break stay ties; the roles that move are
    /// numbered after them, and the base role stays last. Who ranks a member
    /// can change with the order, so every top role is worked out again.
    /// </summary>
    private static async Task<IResult> ReorderRolesAsync(
        ReorderRolesRequest request,
        HttpContext http,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        var serverId = http.RequireActiveServerId();
        var actorId = http.User.RequireUserId();
        if (!(await ServerAccess.GetAsync(db, actorId, serverId, cancellationToken)).HasFlag(ServerPrivileges.ManageRoles))
        {
            return Results.Forbid();
        }

        var baseRoleId = (await ServerAccess.FindMemberRoleAsync(db, serverId, cancellationToken))?.Id;
        if (baseRoleId is null)
        {
            return NoMemberRole(messages);
        }

        var roles = await db.ServerRoles.Where(x => x.ServerId == serverId).ToListAsync(cancellationToken);
        var ranked = HeldRoles.Ranked(roles.Where(x => !x.IsOwner && x.Id != baseRoleId)).ToArray();
        var wanted = request.RoleIds ?? [];
        if (wanted.Count != ranked.Length || wanted.Distinct().Count() != wanted.Count ||
            !wanted.ToHashSet().SetEquals(ranked.Select(x => x.Id)))
        {
            return Results.ValidationProblem(new Dictionary<string, string[]>
            {
                ["roleIds"] = [messages.Get(MessageKey.RoleOrderIncomplete)],
            });
        }

        if (!await ServerAuthorization.CanReorderRolesAsync(db, actorId, serverId, ranked, wanted, cancellationToken))
        {
            return Results.Forbid();
        }

        var kept = 0;
        while (kept < ranked.Length && ranked[kept].Id == wanted[kept])
        {
            kept++;
        }

        if (kept == ranked.Length)
        {
            return Results.NoContent();
        }

        var byId = ranked.ToDictionary(x => x.Id);
        var owner = roles.Where(x => x.IsOwner).Select(x => (int?)x.Position).Min();
        var position = ranked.Take(kept).Select(x => (int?)x.Position).Append(owner).Max() ?? -1;
        foreach (var id in wanted.Skip(kept))
        {
            byId[id].Position = ++position;
        }

        var baseRole = roles.Single(x => x.Id == baseRoleId);
        if (baseRole.Position <= position)
        {
            baseRole.Position = position + 1;
        }

        await db.SaveChangesAsync(cancellationToken);
        await MemberRoles.RankAgainAsync(db, serverId, leaving: null, cancellationToken);
        await ServerAudit.StageAsync(db, serverId, actorId, new AuditEvent(
            "role.reorder", messages.Get(MessageKey.AuditRolesReordered)), cancellationToken);
        await db.SaveChangesAsync(cancellationToken);
        await transaction.CommitAsync(cancellationToken);
        return Results.NoContent();
    }

    private static async Task<IResult> DeleteRoleAsync(
        Guid roleId,
        HttpContext http,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        var serverId = http.RequireActiveServerId();
        var held = await ServerAccess.GetAsync(db, http.User.RequireUserId(), serverId, cancellationToken);
        if (!held.HasFlag(ServerPrivileges.ManageRoles))
        {
            return Results.Forbid();
        }

        var role = await db.ServerRoles.AsNoTracking().FirstOrDefaultAsync(
            x => x.Id == roleId && x.ServerId == serverId, cancellationToken);
        if (role is null)
        {
            return Results.NotFound();
        }

        if (role.IsSystem)
        {
            return Results.Conflict();
        }

        // Its holders keep the other roles they hold, and the plain member
        // role, rather than nothing.
        var fallback = await db.ServerRoles.AsNoTracking().SingleAsync(
            x => x.ServerId == serverId && x.IsSystem && !x.IsOwner, cancellationToken);
        if (!await ServerAuthorization.CanDeleteRoleAsync(db, http.User.RequireUserId(), role, fallback, cancellationToken))
        {
            return Results.Forbid();
        }

        var viewChange = await ViewChange.BeginAsync(db, serverId, cancellationToken);
        var holders = (await HeldRoles.LoadAsync(db, serverId, cancellationToken)).Holders(role.Id);
        // The top role is moved off the role first: a member may not point at
        // a role that is gone. The rows of those holding it go with the role.
        await MemberRoles.RankAgainAsync(db, serverId, leaving: role.Id, cancellationToken);
        await db.ServerRoles.Where(x => x.Id == role.Id).ExecuteDeleteAsync(cancellationToken);
        await ServerAudit.StageAsync(db, serverId, http.User.RequireUserId(), new AuditEvent(
            "role.delete",
            messages.Get(MessageKey.AuditRoleDeleted, ("name", role.Name)),
            "role", role.Id, role.Name,
            holders.Count == 0 ? null : [new AuditChange("holders", To: holders.Count.ToString(System.Globalization.CultureInfo.InvariantCulture))]),
            cancellationToken);
        await db.SaveChangesAsync(cancellationToken);
        await transaction.CommitAsync(cancellationToken);

        await viewChange.PublishAsync(hub.Clients, db);
        await NotifyAsync(hub, db, serverId, holders, cancellationToken);
        return Results.NoContent();
    }

    /// <summary>
    /// Gives the member exactly one role, or none besides the base role when
    /// the base role is named: what the role editor of a client that knows one
    /// role per member asks for.
    /// </summary>
    private static async Task<IResult> AssignRoleAsync(
        Guid userId,
        AssignRoleRequest request,
        HttpContext http,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        var serverId = http.RequireActiveServerId();
        var actorId = http.User.RequireUserId();
        var held = await ServerAccess.GetAsync(db, actorId, serverId, cancellationToken);
        if (!held.HasFlag(ServerPrivileges.ManageMembers) || actorId == userId)
        {
            return Results.Forbid();
        }

        var member = await db.ServerMembers.Include(x => x.Role).Include(x => x.User).FirstOrDefaultAsync(
            x => x.ServerId == serverId && x.UserId == userId, cancellationToken);
        var role = await db.ServerRoles.AsNoTracking().FirstOrDefaultAsync(
            x => x.ServerId == serverId && x.Id == request.RoleId, cancellationToken);
        if (member is null || role is null)
        {
            return Results.NotFound();
        }

        // The owner is neither demoted nor joined by anybody: handing over the
        // server is a transfer of its own.
        if (member.Role.IsOwner || role.IsOwner)
        {
            return Results.Conflict();
        }

        var (current, baseRole) = await HeldRoles.OfMemberAsync(db, member, cancellationToken);
        if (baseRole is null)
        {
            return NoMemberRole(messages);
        }

        IReadOnlyList<ServerRole> wanted = role.Id == baseRole.Id ? [] : [role];
        // As when every member held one role, the role named and the one the
        // member ranks by are checked even when they stay.
        var checkedRoles = Diff(current, wanted).Append(member.Role).Append(role).DistinctBy(x => x.Id).ToArray();
        if (!await ServerAuthorization.CanChangeRolesAsync(db, actorId, member, checkedRoles, cancellationToken))
        {
            return Results.Forbid();
        }

        var removedBesidesTop = current.Where(x => x.Id != member.RoleId && x.Id != role.Id).ToArray();
        if (member.RoleId != role.Id || removedBesidesTop.Length > 0)
        {
            await ServerAudit.StageAsync(db, serverId, actorId, new AuditEvent(
                "member.role",
                messages.Get(MessageKey.AuditRoleChanged, ("name", member.User.DisplayName), ("role", role.Name)),
                "member", member.UserId, member.User.DisplayName,
                ServerAudit.Only(
                    member.RoleId == role.Id
                        ? null
                        : new AuditChange("role", member.Role.Name, role.Name, FromColor: member.Role.Color, ToColor: role.Color),
                    removedBesidesTop.Length == 0
                        ? null
                        : new AuditChange("roles", Removed: removedBesidesTop.Select(x => x.Name).ToArray()))),
                cancellationToken);
        }

        var viewChange = await ViewChange.BeginAsync(db, serverId, cancellationToken);
        await MemberRoles.SetAsync(db, member, wanted, baseRole, cancellationToken);
        await db.SaveChangesAsync(cancellationToken);
        await transaction.CommitAsync(cancellationToken);
        await viewChange.PublishAsync(hub.Clients, db);
        await NotifyAsync(hub, db, serverId, [userId], cancellationToken);
        return Results.NoContent();
    }

    /// <summary>
    /// Sets every role the member holds besides the base role, which nobody
    /// needs to name. Each role given or taken away is checked as if it were
    /// the only one.
    /// </summary>
    private static async Task<IResult> SetRolesAsync(
        Guid userId,
        SetMemberRolesRequest request,
        HttpContext http,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        var serverId = http.RequireActiveServerId();
        var actorId = http.User.RequireUserId();
        var held = await ServerAccess.GetAsync(db, actorId, serverId, cancellationToken);
        if (!held.HasFlag(ServerPrivileges.ManageMembers) || actorId == userId)
        {
            return Results.Forbid();
        }

        var member = await db.ServerMembers.Include(x => x.Role).Include(x => x.User).FirstOrDefaultAsync(
            x => x.ServerId == serverId && x.UserId == userId, cancellationToken);
        var ids = (request.RoleIds ?? []).Distinct().ToArray();
        var roles = await db.ServerRoles.AsNoTracking()
            .Where(x => x.ServerId == serverId && ids.Contains(x.Id))
            .ToListAsync(cancellationToken);
        if (member is null || roles.Count != ids.Length)
        {
            return Results.NotFound();
        }

        if (member.Role.IsOwner || roles.Any(x => x.IsOwner))
        {
            return Results.Conflict();
        }

        var (current, baseRole) = await HeldRoles.OfMemberAsync(db, member, cancellationToken);
        if (baseRole is null)
        {
            return NoMemberRole(messages);
        }

        var wanted = roles.Where(x => x.Id != baseRole.Id).ToArray();
        var changed = Diff(current, wanted);
        if (!await ServerAuthorization.CanChangeRolesAsync(db, actorId, member, changed, cancellationToken))
        {
            return Results.Forbid();
        }

        if (changed.Length == 0)
        {
            return Results.NoContent();
        }

        var top = HeldRoles.Top(wanted, baseRole);
        await ServerAudit.StageAsync(db, serverId, actorId, new AuditEvent(
            "member.role",
            messages.Get(MessageKey.AuditRolesChanged, ("name", member.User.DisplayName)),
            "member", member.UserId, member.User.DisplayName,
            ServerAudit.Only(
                top.Id == member.RoleId
                    ? null
                    : new AuditChange("role", member.Role.Name, top.Name, FromColor: member.Role.Color, ToColor: top.Color),
                new AuditChange(
                    "roles",
                    Added: HeldRoles.Ranked(wanted.Where(x => current.All(held => held.Id != x.Id))).Select(x => x.Name).ToArray(),
                    Removed: HeldRoles.Ranked(current.Where(x => wanted.All(kept => kept.Id != x.Id))).Select(x => x.Name).ToArray()))),
            cancellationToken);
        var viewChange = await ViewChange.BeginAsync(db, serverId, cancellationToken);
        await MemberRoles.SetAsync(db, member, wanted, baseRole, cancellationToken);
        await db.SaveChangesAsync(cancellationToken);
        await transaction.CommitAsync(cancellationToken);
        await viewChange.PublishAsync(hub.Clients, db);
        await NotifyAsync(hub, db, serverId, [userId], cancellationToken);
        return Results.NoContent();
    }

    /// <summary>The roles in one list and not the other, either way.</summary>
    private static ServerRole[] Diff(IReadOnlyList<ServerRole> before, IReadOnlyList<ServerRole> after) =>
        before.Where(x => after.All(kept => kept.Id != x.Id))
            .Concat(after.Where(x => before.All(held => held.Id != x.Id)))
            .ToArray();

    private static IResult NoMemberRole(Messages messages) =>
        Results.Problem(
            statusCode: StatusCodes.Status503ServiceUnavailable,
            title: messages.Get(MessageKey.ServerHasNoMemberRole));

    internal static RoleDraft? Validate(string? name, string? color, IEnumerable<string>? permissionNames)
    {
        var trimmedName = name?.Trim() ?? "";
        var trimmedColor = color?.Trim() ?? "";
        return trimmedName.Length is >= 1 and <= MaxNameLength &&
            trimmedColor.Length is >= 1 and <= MaxColorLength &&
            ServerAccess.TryParse(permissionNames, out var permissions)
            ? new RoleDraft(trimmedName, trimmedColor, permissions)
            : null;
    }

    internal static IResult InvalidRole(Messages messages) =>
        Results.ValidationProblem(new Dictionary<string, string[]>
        {
            ["role"] = [messages.Get(MessageKey.RoleInvalid)],
        });

    /// <summary>
    /// Tells each member their own server-wide permissions, which every role
    /// they hold makes up: the same change reaches different people with
    /// different answers. Sent after the change is committed. Whoever the
    /// change showed a channel to or hid one from is told that apart, by
    /// <see cref="ViewChange"/>.
    /// </summary>
    internal static async Task NotifyAsync(
        IHubContext<AppHub> hub,
        AppDbContext db,
        Guid serverId,
        IEnumerable<Guid> userIds,
        CancellationToken cancellationToken)
    {
        var recipients = userIds.Distinct().ToArray();
        if (recipients.Length == 0)
        {
            return;
        }

        var held = await HeldRoles.LoadAsync(db, serverId, cancellationToken);
        foreach (var userId in recipients.Where(held.ByMember.ContainsKey))
        {
            await hub.Clients.Group(AppHub.UserGroup(userId)).SendAsync(
                "PermissionsChanged",
                new PermissionsChangedDto(serverId.ToString(), ServerAccess.ToNames(held.PermissionsOf(userId))),
                cancellationToken);
        }
    }
}
