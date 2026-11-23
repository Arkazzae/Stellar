using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Endpoints;

/// <summary>
/// Handing a server on and deleting it: what its owner does from the server
/// settings and the operator does from Cockpit, written once for both.
/// </summary>
internal static class ServerOwnership
{
    /// <summary>
    /// Moves the owner slot and the owner role from <paramref name="owner"/> to
    /// <paramref name="successor"/>, who must both be tracked members of
    /// <paramref name="serverId"/>. The old owner keeps any other role they
    /// hold, and ranks by the highest of them, or else becomes a plain member;
    /// the successor keeps theirs too. Run it in a transaction: the slot is
    /// freed and saved before it is handed on, and that moment must not be
    /// seen. The successor's change is staged; the caller saves it with its
    /// audit line.
    /// </summary>
    internal static async Task HandOverAsync(
        AppDbContext db,
        Guid serverId,
        ServerMember owner,
        ServerMember successor,
        CancellationToken cancellationToken)
    {
        var memberRole = await ServerAccess.FindMemberRoleAsync(db, serverId, cancellationToken)
            ?? throw new InvalidOperationException("The server has no role for plain members.");

        // The owner slot is unique, so it is freed before it is handed on.
        var ownerRole = await db.ServerRoles.AsNoTracking()
            .SingleAsync(x => x.Id == owner.RoleId, cancellationToken);
        var (kept, _) = await HeldRoles.OfMemberAsync(db, owner, cancellationToken);
        await MemberRoles.SetAsync(db, owner, kept.Where(x => x.Id != ownerRole.Id), memberRole, cancellationToken);
        owner.OwnerSlot = null;
        await db.SaveChangesAsync(cancellationToken);

        var (held, _) = await HeldRoles.OfMemberAsync(db, successor, cancellationToken);
        await MemberRoles.SetAsync(db, successor, held.Append(ownerRole), memberRole, cancellationToken);
        successor.OwnerSlot = 1;
        // Nobody sets an override on the owner, who skips them; one left on the
        // successor would wake up again after a later hand-over.
        await db.MemberPermissionOverrides.Where(x => x.ServerId == serverId && x.UserId == successor.UserId)
            .ExecuteDeleteAsync(cancellationToken);
    }

    /// <summary>
    /// Deletes the server in one statement. Its roles, memberships, invites,
    /// emoji, soundboard clips, notifications and the like go with it by
    /// cascade; its channels, and everything written in them, by the triggers
    /// of <c>ServerContentSchema</c>, since a channel has no foreign key to its
    /// server. The emoji images are queued for deletion then. The uploads its
    /// messages, icon and clips used are left without a reference, and the
    /// maintenance worker removes them with their files once the orphan
    /// retention has passed.
    /// </summary>
    /// <returns>Whether there was such a server.</returns>
    internal static async Task<bool> DeleteAsync(
        AppDbContext db,
        Guid serverId,
        CancellationToken cancellationToken) =>
        await db.Servers.Where(x => x.Id == serverId).ExecuteDeleteAsync(cancellationToken) > 0;
}
