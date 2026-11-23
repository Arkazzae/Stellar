using Stellarr.Api.Domain;

namespace Stellarr.Api.Integrations.Discord;

/// <summary>What one structure write made, and what it found already there.</summary>
/// <param name="Simplified">
/// The channels that Stellarr only has as text or voice, by Discord id, with
/// what they were: forums, media channels, announcement and stage channels.
/// </param>
/// <param name="HiddenFromEveryone">
/// The channels whose Discord overwrites hid them from everyone. Overwrites
/// are not imported, so these are open to the whole Stellarr server until a
/// later stage closes them.
/// </param>
public sealed record DiscordStructureResult(
    int SectionsMade,
    int ChannelsMade,
    int ThreadsRecorded,
    int AlreadyImported,
    IReadOnlyDictionary<string, DiscordChannelType> Simplified,
    IReadOnlyList<string> HiddenFromEveryone);

/// <summary>What one page of a channel's messages came to.</summary>
/// <param name="AlreadySeen">Messages an earlier page already took, which this one left alone.</param>
/// <param name="AttachmentsPending">Written with their text; their attachments wait for the attachment stage.</param>
/// <param name="AwaitingAttachments">Nothing but attachments: recorded, not written, for the attachment stage.</param>
/// <param name="Skipped">Left out, by reason.</param>
/// <param name="NoticesByType">The notices among <paramref name="Skipped"/>, by Discord's message type.</param>
public sealed record DiscordPageResult(
    int Written,
    int AlreadySeen,
    int AttachmentsPending,
    int AwaitingAttachments,
    int PlaceholdersMade,
    IReadOnlyDictionary<DiscordSkipReason, int> Skipped,
    IReadOnlyDictionary<int, int> NoticesByType);

/// <summary>What linking one channel or thread did.</summary>
/// <param name="Waiting">
/// A thread started from a message of a parent channel whose history is not
/// read yet: it is linked when the parent is completed.
/// </param>
public sealed record DiscordLinkResult(int RepliesLinked, int RepliesLeftPlain, int ThreadsLinked, bool Waiting);

/// <summary>Everything an import has met on one server so far, counted from the bookkeeping.</summary>
public sealed record DiscordImportTally(
    int Written,
    int AttachmentsPending,
    int AwaitingAttachments,
    IReadOnlyDictionary<DiscordSkipReason, int> Skipped,
    IReadOnlyDictionary<int, int> NoticesByType,
    int Placeholders);
