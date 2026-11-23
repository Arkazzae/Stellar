namespace Stellarr.Api.Domain;

/// <summary>
/// What a role lets its holders do. The first eight manage the server; the
/// ones after them are what an ordinary member does, each checked in exactly
/// one place. Administrator stands for all of them and is server-wide only.
/// </summary>
[Flags]
public enum ServerPrivileges : long
{
    None = 0,
    ManageServer = 1 << 0,
    ManageChannels = 1 << 1,
    ManageRoles = 1 << 2,
    ManageMembers = 1 << 3,
    ManageMessages = 1 << 4,
    ModerateMembers = 1 << 5,
    ManageBans = 1 << 6,
    ViewAuditLog = 1 << 7,

    /// <summary>Every permission everywhere; no override applies to its holders.</summary>
    Administrator = 1 << 8,

    /// <summary>Posting a message, replies included, in a text channel.</summary>
    SendMessages = 1 << 9,

    /// <summary>Posting a message that carries an attachment, a voice message included.</summary>
    AttachFiles = 1 << 10,

    /// <summary>Adding a reaction; taking one's own back needs nothing.</summary>
    AddReactions = 1 << 11,

    /// <summary>Taking a seat in a voice channel.</summary>
    Connect = 1 << 12,

    /// <summary>Creating an invite to the server or to a channel.</summary>
    CreateInvite = 1 << 13,

    /// <summary>
    /// Seeing a channel at all. Somebody who does not hold it in a channel
    /// holds nothing there, and the channel does not exist for them.
    /// </summary>
    ViewChannel = 1 << 14,

    All = ManageServer | ManageChannels | ManageRoles | ManageMembers |
        ManageMessages | ModerateMembers | ManageBans | ViewAuditLog |
        Administrator | SendMessages | AttachFiles | AddReactions | Connect | CreateInvite |
        ViewChannel,
}

public sealed class ServerRole
{
    /// <summary>
    /// What the base role of a new server lets everybody do: see the
    /// channels, talk, share, react and join voice. A server made before these
    /// permissions existed was given the same once, see
    /// <c>OrdinaryPermissionDefaults</c> and <c>ViewChannelDefaults</c>, and
    /// <c>scripts/seed-dev.py</c> writes the same number.
    /// </summary>
    public const ServerPrivileges BaseRoleDefaults =
        ServerPrivileges.ViewChannel | ServerPrivileges.SendMessages | ServerPrivileges.AttachFiles |
        ServerPrivileges.AddReactions | ServerPrivileges.Connect;

    public Guid Id { get; set; }
    public Guid ServerId { get; set; }
    public required string Name { get; set; }
    public required string Color { get; set; }
    public ServerPrivileges Permissions { get; set; }
    public int Position { get; set; }

    /// <summary>System roles cannot be removed; the owner role cannot be edited.</summary>
    public bool IsSystem { get; set; }

    public bool IsOwner { get; set; }
}
