using Stellarr.Api.Domain;

namespace Stellarr.Api.Integrations.Discord;

// What an import reads from Discord, named as Stellarr names it where it has
// the thing. Ids are Discord's own, as text: snowflakes for a server the bot
// read, the template's placeholder numbers ("0", "1", ...) for a template.
// Nothing here is Stellarr's yet; turning it into Stellarr's records is the
// import's job, and permission bits stay Discord's raw bit sets until then.

/// <summary>Where a snapshot was read from, which says how much of it there can be.</summary>
public enum SnapshotSource
{
    /// <summary>A template link: roles, channels and their permissions only.</summary>
    Template,

    /// <summary>A bot in the server: everything the bot may see.</summary>
    Bot,
}

/// <summary>
/// One Discord server's structure and people. Messages are not part of it:
/// there can be millions, so they are read channel by channel as a stream.
/// </summary>
public sealed record ServerSnapshot
{
    public required SnapshotSource Source { get; init; }

    /// <summary>The server's id; for a template, that of the server it was made from.</summary>
    public required string SourceId { get; init; }

    public required string Name { get; init; }
    public string? Description { get; init; }

    /// <summary>The icon on Discord's CDN, or null when the server has none.</summary>
    public string? IconUrl { get; init; }

    /// <summary>Discord's categories.</summary>
    public IReadOnlyList<SectionSnapshot> Sections { get; init; } = [];

    public IReadOnlyList<ChannelSnapshot> Channels { get; init; } = [];
    public IReadOnlyList<RoleSnapshot> Roles { get; init; } = [];
    public IReadOnlyList<EmojiSnapshot> Emoji { get; init; } = [];
    public IReadOnlyList<StickerSnapshot> Stickers { get; init; } = [];
    public IReadOnlyList<MemberSnapshot> Members { get; init; } = [];
    public IReadOnlyList<ThreadSnapshot> Threads { get; init; } = [];

    /// <summary>
    /// What the bot could not read, and why. Everything else in the snapshot
    /// is complete; a part listed here is missing, not empty.
    /// </summary>
    public IReadOnlyList<SnapshotGap> Gaps { get; init; } = [];
}

/// <summary>A part of a server the reader was refused, such as one channel's private threads.</summary>
public sealed record SnapshotGap(string What, string? ChannelId, DiscordFailureKind Kind, int? DiscordCode);

/// <summary>A Discord category: a named group of channels.</summary>
public sealed record SectionSnapshot(
    string Id,
    string Name,
    int Position,
    IReadOnlyList<PermissionOverwriteSnapshot> Overwrites);

/// <summary>
/// What kind of Discord channel a channel was. Stellarr has text and voice;
/// this says which of Discord's flavours of them it came from, so the import
/// can say what it simplified.
/// </summary>
public enum DiscordChannelType
{
    /// <summary>An ordinary text or voice channel.</summary>
    Conversation,

    /// <summary>A text channel other servers can follow.</summary>
    Announcements,

    /// <summary>A text channel that holds only threads, as posts.</summary>
    Forum,

    /// <summary>A forum laid out for images and video.</summary>
    Media,

    /// <summary>A voice channel with speakers and an audience.</summary>
    Stage,
}

public sealed record ChannelSnapshot
{
    public required string Id { get; init; }
    public required string Name { get; init; }
    public required ChannelKind Kind { get; init; }
    public required DiscordChannelType DiscordType { get; init; }
    public string? Topic { get; init; }
    public int SlowModeSeconds { get; init; }

    /// <summary>For a voice channel, how many may be in it at once; zero for no limit.</summary>
    public int UserLimit { get; init; }

    /// <summary>
    /// Discord's position, which is not unique: its client sorts a section's
    /// text channels by position and then id, and its voice channels after them.
    /// </summary>
    public int Position { get; init; }

    /// <summary>The section this channel sits under, or null when it has none.</summary>
    public string? SectionId { get; init; }

    public IReadOnlyList<PermissionOverwriteSnapshot> Overwrites { get; init; } = [];
}

public enum OverwriteTarget
{
    Role,
    Member,
}

/// <summary>
/// One allow and deny pair on a channel or section, for one role or one
/// member. The two sets are Discord's permission bits as decimal text.
/// </summary>
public sealed record PermissionOverwriteSnapshot(OverwriteTarget Target, string TargetId, string Allow, string Deny);

public sealed record RoleSnapshot
{
    public required string Id { get; init; }
    public required string Name { get; init; }

    /// <summary>The colour as 0xRRGGBB, or null for a role without one.</summary>
    public int? Color { get; init; }

    /// <summary>Discord's position: higher is more senior, and @everyone is 0.</summary>
    public int Position { get; init; }

    /// <summary>Whether members with it are listed apart.</summary>
    public bool Hoisted { get; init; }

    public bool Mentionable { get; init; }

    /// <summary>A role a bot or integration owns, such as a bot's own role or the boosters'.</summary>
    public bool Managed { get; init; }

    /// <summary>Discord's permission bits as decimal text.</summary>
    public required string Permissions { get; init; }

    /// <summary>
    /// The role every member has. Discord gives it the server's id, and a
    /// template the placeholder 0.
    /// </summary>
    public bool IsEveryone { get; init; }
}

public sealed record EmojiSnapshot(string Id, string Name, bool Animated, bool Available, string ImageUrl);

public sealed record StickerSnapshot(string Id, string Name, string? Description, StickerFormat Format, string ImageUrl);

public sealed record MemberSnapshot
{
    public required string Id { get; init; }
    public required string Username { get; init; }

    /// <summary>The name the person shows everywhere on Discord, or null when they kept their username.</summary>
    public string? DisplayName { get; init; }

    /// <summary>The name they use on this server only.</summary>
    public string? Nickname { get; init; }

    public string? AvatarUrl { get; init; }

    /// <summary>The avatar they set for this server only.</summary>
    public string? ServerAvatarUrl { get; init; }

    /// <summary>Their roles, without @everyone, which every member has.</summary>
    public IReadOnlyList<string> RoleIds { get; init; } = [];

    public bool IsBot { get; init; }
    public DateTimeOffset? JoinedAt { get; init; }
}

public sealed record ThreadSnapshot
{
    public required string Id { get; init; }
    public required string ParentChannelId { get; init; }
    public required string Name { get; init; }

    /// <summary>
    /// The message the thread grew from, which has the thread's own id. It is
    /// in the parent channel for a thread under a text or announcement
    /// channel, the first message of the thread for a forum or media post,
    /// and possibly deleted since. A private thread has none.
    /// </summary>
    public string? StarterMessageId { get; init; }

    public bool Archived { get; init; }
    public bool Locked { get; init; }

    /// <summary>Visible only to those invited and to moderators.</summary>
    public bool Private { get; init; }

    /// <summary>When it was made; Discord only knows this for threads made after 2022-01-09.</summary>
    public DateTimeOffset? CreatedAt { get; init; }
}

public sealed record MessageSnapshot
{
    public required string Id { get; init; }

    /// <summary>The channel or thread the message is in.</summary>
    public required string ChannelId { get; init; }

    public required AuthorSnapshot Author { get; init; }

    /// <summary>
    /// As Discord stores it, tokens and all; <see cref="DiscordContent.Rewrite"/>
    /// makes Stellarr text of it. Empty for every message but the bot's own
    /// when the app lacks the Message Content intent.
    /// </summary>
    public required string Content { get; init; }

    public required DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset? EditedAt { get; init; }

    /// <summary>The message this one answers, if it is a reply.</summary>
    public string? ReplyToMessageId { get; init; }

    public bool Pinned { get; init; }

    /// <summary>Discord's message type: 0 is ordinary, 19 a reply, most others notices.</summary>
    public int Type { get; init; }

    /// <summary>A notice Discord wrote, such as a member joining or a pin, rather than something somebody said.</summary>
    public bool IsSystemNotice { get; init; }

    public IReadOnlyList<AttachmentSnapshot> Attachments { get; init; } = [];
    public IReadOnlyList<EmbedSummary> Embeds { get; init; } = [];
    public IReadOnlyList<StickerReference> Stickers { get; init; } = [];
    public IReadOnlyList<ReactionSnapshot> Reactions { get; init; } = [];

    /// <summary>The thread started from this message, which has the message's id.</summary>
    public string? StartedThreadId { get; init; }

    /// <summary>The copy of another message this one forwards; its content is empty then.</summary>
    public ForwardedSnapshot? Forwarded { get; init; }
}

/// <summary>
/// Who a message shows as written by. For a webhook message, that is the
/// name and avatar the webhook gave this one message, and <see cref="Id"/> is
/// the webhook's.
/// </summary>
public sealed record AuthorSnapshot
{
    public required string Id { get; init; }
    public required string Username { get; init; }
    public string? DisplayName { get; init; }
    public string? AvatarUrl { get; init; }
    public bool IsBot { get; init; }

    /// <summary>The webhook that posted the message, or null for a person or bot.</summary>
    public string? WebhookId { get; init; }
}

public sealed record AttachmentSnapshot
{
    public required string Id { get; init; }
    public required string FileName { get; init; }
    public long Size { get; init; }
    public string? ContentType { get; init; }

    /// <summary>A signed address that expires: download it while the import runs, or read the message again.</summary>
    public required string Url { get; init; }

    public int? Width { get; init; }
    public int? Height { get; init; }
    public double? DurationSeconds { get; init; }
}

/// <summary>An embed reduced to what a link preview shows.</summary>
public sealed record EmbedSummary(string? Type, string? Url, string? Title, string? Description, string? ImageUrl);

public sealed record StickerReference(string Id, string Name, StickerFormat Format, string ImageUrl);

/// <summary>
/// One emoji's reactions on a message. Who reacted is a separate request per
/// emoji (and per kind, for super reactions), so only the counts come with
/// the message.
/// </summary>
public sealed record ReactionSnapshot(ReactionEmoji Emoji, int Count, int BurstCount);

/// <summary>A reaction's emoji: Unicode when <see cref="Id"/> is null, a custom emoji otherwise.</summary>
public sealed record ReactionEmoji(string? Id, string Name, bool Animated);

public sealed record ForwardedSnapshot(string Content, IReadOnlyList<AttachmentSnapshot> Attachments);

/// <summary>A pinned message and when it was pinned, which the message itself does not say.</summary>
public sealed record PinSnapshot(MessageSnapshot Message, DateTimeOffset PinnedAt);

/// <summary>Somebody who reacted.</summary>
public sealed record ReactionUser(string Id, string Username, string? DisplayName, bool IsBot);
