namespace Stellarr.Api.Domain;

public sealed class StellarrServer
{
    public Guid Id { get; set; }
    public required string Name { get; set; }
    public string Description { get; set; } = "";
    public bool IsPublic { get; set; }
    public string? IconUrl { get; set; }
    public DateTime CreatedAt { get; set; }
    public bool SoundboardEveryoneCanPlay { get; set; } = true;
    public bool SoundboardRateLimited { get; set; } = true;
}

/// <summary>
/// Membership of one user on one server. The roles the member holds are rows
/// of <see cref="ServerMemberRole"/>; <see cref="RoleId"/> is the highest of
/// them, which ranks the member, or the base role when they hold none.
/// </summary>
public sealed class ServerMember
{
    public Guid ServerId { get; set; }
    public Guid UserId { get; set; }

    /// <summary>The member's top role: the highest role they hold, or the base role.</summary>
    public Guid RoleId { get; set; }

    /// <summary>
    /// One for the owner, null for everybody else. SQLite allows many nulls in
    /// a unique index, which makes this a durable one-owner-per-server slot.
    /// </summary>
    public int? OwnerSlot { get; set; }

    public DateTime JoinedAt { get; set; }
    public User User { get; set; } = null!;
    public ServerRole Role { get; set; } = null!;
}

/// <summary>
/// One role a member holds. The base role, which every member holds, has no
/// row; the owner role has one, for the owner only. The rows go with the
/// membership, so somebody who leaves and comes back holds nothing.
/// </summary>
public sealed class ServerMemberRole
{
    public Guid ServerId { get; set; }
    public Guid UserId { get; set; }
    public Guid RoleId { get; set; }
}

public sealed class ServerAdminSettings
{
    public Guid ServerId { get; set; }
    public bool AllMessages { get; set; }
    public bool MentionsOnly { get; set; } = true;
    public bool VoiceJoinSound { get; set; } = true;
    public string MutedChannelIdsJson { get; set; } = "[]";
    public bool ServerMuted { get; set; }
}
