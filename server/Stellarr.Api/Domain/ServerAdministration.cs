namespace Stellarr.Api.Domain;

/// <summary>
/// A persistent refusal of membership on one server. Removing a
/// <see cref="ServerMember"/> is only a kick; this row is what keeps a ban in
/// force across reconnects and restarts until a moderator lifts it.
/// </summary>
public sealed class ServerBan
{
    public Guid ServerId { get; set; }
    public Guid UserId { get; set; }
    public Guid BannedById { get; set; }
    public required string Reason { get; set; }
    public DateTime BannedAt { get; set; }

    public User User { get; set; } = null!;
    public User BannedBy { get; set; } = null!;
}

public sealed class ServerInvite
{
    public Guid Id { get; set; }
    public Guid ServerId { get; set; }
    public Guid CreatedById { get; set; }

    /// <summary>The channel a new member lands in, or null once that channel is gone.</summary>
    public Guid? ChannelId { get; set; }

    public required string Code { get; set; }
    public int Uses { get; set; }
    public int? MaxUses { get; set; }
    public DateTime? ExpiresAt { get; set; }
    public DateTime CreatedAt { get; set; }

    public User CreatedBy { get; set; } = null!;
    public Channel? Channel { get; set; }
}

public sealed class ServerAuditEntry
{
    public Guid Id { get; set; }
    public Guid ServerId { get; set; }
    public Guid ActorId { get; set; }

    /// <summary>
    /// The line in the server's language, as it was when written. Entries from
    /// before the structured fields have only this.
    /// </summary>
    public required string Summary { get; set; }

    public DateTime CreatedAt { get; set; }

    /// <summary>What was done, such as <c>role.update</c>; a client words it in its reader's language.</summary>
    public string? Action { get; set; }

    /// <summary>What it was done to: <c>role</c>, <c>member</c>, <c>channel</c> and so on.</summary>
    public string? TargetType { get; set; }

    public Guid? TargetId { get; set; }

    /// <summary>The target's name when it happened, which outlives a rename or a deletion.</summary>
    public string? TargetName { get; set; }

    /// <summary>The changed fields as JSON, see <c>AuditChange</c>.</summary>
    public string? ChangesJson { get; set; }

    public User Actor { get; set; } = null!;
}
