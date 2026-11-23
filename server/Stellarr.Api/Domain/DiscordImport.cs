using Stellarr.Api.Integrations.Discord;

namespace Stellarr.Api.Domain;

// The bookkeeping of a Discord import: what was brought into which Stellarr
// server, keyed by Discord's ids, so that a second run brings nothing twice
// and later stages (attachments, permissions, an undo) can find what the
// import made. Discord's ids are kept as the text the reader gives them.

/// <summary>One import of a Discord server into a Stellarr server that already exists.</summary>
public sealed class DiscordImport
{
    public Guid Id { get; set; }

    /// <summary>
    /// The Stellarr server it writes into, or null once that server is
    /// deleted. The row outlives the server: the placeholder accounts it made
    /// belong to no server, and an undo has to be able to find them.
    /// </summary>
    public Guid? ServerId { get; set; }

    /// <summary>The Discord server it reads.</summary>
    public required string DiscordServerId { get; set; }

    /// <summary>The Discord server's name when the import started.</summary>
    public required string DiscordServerName { get; set; }

    /// <summary>Who started it, or null once that account is gone.</summary>
    public Guid? StartedById { get; set; }

    public DateTime StartedAt { get; set; }
}

/// <summary>
/// The placeholder account that stands in for one Discord author. There is
/// one per Discord id on the whole instance, because the account's username,
/// <c>discord:{id}</c>, is unique on it: a person who wrote in two imported
/// servers is one placeholder, made by whichever import met them first.
/// </summary>
public sealed class DiscordImportAccount
{
    /// <summary>The Discord user, or for a webhook's messages the webhook.</summary>
    public required string DiscordId { get; set; }

    public Guid UserId { get; set; }

    /// <summary>
    /// The import that made the account, or null once that import's record is
    /// gone. Accounts are not removed with a server, so this is how an undo
    /// finds the ones it made; one that messages of another import still name
    /// must stay.
    /// </summary>
    public Guid? ImportId { get; set; }

    /// <summary>
    /// A webhook, which may show a different name on every message; the
    /// account keeps the name of the first message the import met.
    /// </summary>
    public bool IsWebhook { get; set; }

    public bool IsBot { get; set; }

    /// <summary>Discord's username, which the account's own username replaces with the id.</summary>
    public required string DiscordUsername { get; set; }

    /// <summary>
    /// The avatar on Discord's CDN, for a later stage to copy. The account's own
    /// avatar only ever points at this server, so this is not it.
    /// </summary>
    public string? DiscordAvatarUrl { get; set; }

    public DateTime CreatedAt { get; set; }
}

/// <summary>A Discord category and the section it became.</summary>
public sealed class DiscordImportSection
{
    public Guid ServerId { get; set; }
    public required string DiscordId { get; set; }

    /// <summary>The import that made the section.</summary>
    public Guid ImportId { get; set; }

    /// <summary>The section, or null once somebody deleted it; an import does not bring it back.</summary>
    public Guid? SectionId { get; set; }
}

/// <summary>Whether a <see cref="DiscordImportChannel"/> is a channel or a thread in one.</summary>
public enum DiscordImportChannelKind
{
    Channel = 0,

    /// <summary>A thread or a forum post: its messages become a thread in the parent's channel.</summary>
    Thread = 1,
}

/// <summary>
/// A Discord channel or thread, where its messages went, and how far the
/// import has read it. Messages carry the id of the channel or thread they
/// were read from, so the cursor belongs to that, not to the Stellarr channel.
/// </summary>
public sealed class DiscordImportChannel
{
    public Guid ServerId { get; set; }
    public required string DiscordId { get; set; }

    /// <summary>The import that recorded it.</summary>
    public Guid ImportId { get; set; }

    public DiscordImportChannelKind Kind { get; set; }

    /// <summary>
    /// The Stellarr channel its messages go to: for a thread, its parent's.
    /// Null once somebody deleted that channel; its messages are then left out.
    /// </summary>
    public Guid? ChannelId { get; set; }

    /// <summary>
    /// What Discord had: a forum, a stage and the others became text or voice
    /// channels, which a later stage may want to know. A thread has its parent's.
    /// </summary>
    public DiscordChannelType DiscordType { get; set; }

    /// <summary>The thread's parent channel; null for a channel.</summary>
    public string? ParentDiscordId { get; set; }

    /// <summary>The thread's name, which heads it when no message of the parent started it.</summary>
    public string? Name { get; set; }

    /// <summary>The message of the parent channel a thread grew from, as Discord says.</summary>
    public string? StarterMessageId { get; set; }

    /// <summary>
    /// A private thread, or a channel whose own overwrite hides it from
    /// everyone. Overwrites are not imported here, so whoever writes the
    /// messages has to know that they would be shown to the whole server.
    /// </summary>
    public bool IsPrivate { get; set; }

    /// <summary>
    /// The oldest message the import has taken from this channel, written or
    /// left out: reading on from before it resumes the import. Null until the
    /// first page.
    /// </summary>
    public string? OldestMessageId { get; set; }

    /// <summary>When the whole history was read, which is when replies can be linked for good.</summary>
    public DateTime? CompletedAt { get; set; }

    /// <summary>
    /// For a thread, the Stellarr message its messages answer, once chosen.
    /// When it is the thread's own first message, that message also carries
    /// the thread's name.
    /// </summary>
    public Guid? RootMessageId { get; set; }

    /// <summary>When the replies were last linked.</summary>
    public DateTime? LinkedAt { get; set; }
}

/// <summary>What became of one Discord message.</summary>
public enum DiscordMessageOutcome
{
    /// <summary>Written as a Stellarr message.</summary>
    Written = 0,

    /// <summary>Left out, for the <see cref="DiscordSkipReason"/> recorded with it.</summary>
    Skipped = 1,

    /// <summary>
    /// Nothing but attachments, which this stage does not copy: the attachment
    /// stage writes it. Replies to it wait for that.
    /// </summary>
    AwaitingAttachments = 2,
}

/// <summary>Why a Discord message was left out.</summary>
public enum DiscordSkipReason
{
    None = 0,

    /// <summary>A notice Discord wrote (a join, a boost, a pin and the like); its type says which.</summary>
    Notice = 1,

    /// <summary>A copy of another message, which Stellarr has no shape for.</summary>
    Forward = 2,

    /// <summary>Only a sticker.</summary>
    StickerOnly = 3,

    /// <summary>Only embeds, as bots post; Stellarr's link previews are made from links, not stored.</summary>
    EmbedOnly = 4,

    /// <summary>
    /// Nothing at all, which is what every message reads as when the bot's app
    /// lacks the Message Content intent.
    /// </summary>
    Empty = 5,

    /// <summary>The chat of a voice channel, which Stellarr does not have.</summary>
    VoiceChannel = 6,

    /// <summary>Its Stellarr channel was deleted after the import made it.</summary>
    ChannelRemoved = 7,

    /// <summary>Discord named no author and no webhook.</summary>
    NoAuthor = 8,

    /// <summary>Longer, once rewritten, than a Stellarr message may be.</summary>
    TooLong = 9,

    /// <summary>
    /// The id the message would get is held by a message the import did not
    /// write: a client may choose the id of what it sends.
    /// </summary>
    IdTaken = 10,
}

/// <summary>One Discord message the import has seen, and what became of it.</summary>
public sealed class DiscordImportMessage
{
    public Guid ServerId { get; set; }
    public required string DiscordId { get; set; }

    /// <summary>The import that met it first.</summary>
    public Guid ImportId { get; set; }

    /// <summary>The channel or thread it was read from.</summary>
    public required string ChannelDiscordId { get; set; }

    public DiscordMessageOutcome Outcome { get; set; }
    public DiscordSkipReason Reason { get; set; }

    /// <summary>Discord's message type.</summary>
    public int DiscordType { get; set; }

    /// <summary>When it was sent on Discord, which orders a thread when its first message is chosen.</summary>
    public DateTime CreatedAt { get; set; }

    /// <summary>
    /// The Stellarr message, derived from the server and the Discord id. Null
    /// when nothing was written, or once somebody deleted the message, which a
    /// second run then leaves deleted.
    /// </summary>
    public Guid? MessageId { get; set; }

    /// <summary>The message it answers on Discord. Kept after linking, for the attachment stage to link again.</summary>
    public string? ReplyToDiscordId { get; set; }

    /// <summary>It had attachments that are not copied yet.</summary>
    public bool AttachmentsPending { get; set; }
}
