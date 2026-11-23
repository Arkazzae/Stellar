using System.Globalization;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Integrations.Discord;

/// <summary>
/// Turns Discord's JSON shapes into the snapshot. A record without the id it
/// is known by is left out rather than guessed at; every other missing field
/// takes the value Discord documents as its default.
/// </summary>
internal static class DiscordSnapshotMapper
{
    /// <summary>The id a template gives the role every member has.</summary>
    internal const string TemplateEveryoneId = "0";

    /// <summary>Discord's epoch, the first moment of 2015, in Unix milliseconds.</summary>
    private const long DiscordEpochMilliseconds = 1_420_070_400_000;

    private const long HasThreadFlag = 1 << 5;
    private const int ReplyType = 19;
    private const int ForwardReference = 1;

    /// <summary>Message types that are somebody speaking: ordinary, reply, slash command, context menu command.</summary>
    private static readonly HashSet<int> SpokenTypes = [0, ReplyType, 20, 23];

    public static ServerSnapshot FromTemplate(WireTemplate template)
    {
        var guild = template.SerializedSourceGuild ?? new WireGuild();
        var sourceId = template.SourceGuildId ?? "";
        var (sections, channels) = Structure(guild.Channels);
        return new ServerSnapshot
        {
            Source = SnapshotSource.Template,
            SourceId = sourceId,
            Name = guild.Name ?? template.Name ?? "",
            Description = guild.Description,
            IconUrl = sourceId.Length > 0 ? DiscordCdn.ServerIcon(sourceId, guild.IconHash ?? guild.Icon) : null,
            Sections = sections,
            Channels = channels,
            Roles = Roles(guild.Roles, TemplateEveryoneId),
        };
    }

    public static IReadOnlyList<RoleSnapshot> Roles(IEnumerable<WireRole?>? roles, string everyoneId) =>
        (roles ?? [])
            .Select((role, index) => role?.Id is { } id
                ? new RoleSnapshot
                {
                    Id = id,
                    Name = role.Name ?? "",
                    Color = (role.Colors?.PrimaryColor ?? role.Color) is > 0 and var color ? color : null,
                    // A template lists its roles in order and gives no positions.
                    Position = role.Position ?? index,
                    Hoisted = role.Hoist ?? false,
                    Mentionable = role.Mentionable ?? false,
                    Managed = role.Managed ?? false,
                    Permissions = role.Permissions ?? "0",
                    IsEveryone = id == everyoneId,
                }
                : null)
            .OfType<RoleSnapshot>()
            .ToList();

    /// <summary>Splits a server's channel list into its categories and the channels Stellarr can have.</summary>
    /// <remarks>
    /// Discord's directory channels (a hub's list of servers) and any type
    /// newer than this reader are left out; threads never come in this list.
    /// </remarks>
    public static (IReadOnlyList<SectionSnapshot> Sections, IReadOnlyList<ChannelSnapshot> Channels) Structure(
        IEnumerable<WireChannel?>? channels)
    {
        var all = (channels ?? []).OfType<WireChannel>().Where(channel => channel.Id is not null).ToList();
        var sections = all
            .Where(channel => channel.Type == 4)
            .Select(channel => new SectionSnapshot(
                channel.Id!, channel.Name ?? "", channel.Position ?? 0, Overwrites(channel.PermissionOverwrites)))
            .ToList();
        var sectionIds = sections.Select(section => section.Id).ToHashSet(StringComparer.Ordinal);

        var mapped = new List<ChannelSnapshot>();
        foreach (var channel in all)
        {
            (ChannelKind Kind, DiscordChannelType Type)? shape = channel.Type switch
            {
                0 => (ChannelKind.Text, DiscordChannelType.Conversation),
                2 => (ChannelKind.Voice, DiscordChannelType.Conversation),
                5 => (ChannelKind.Text, DiscordChannelType.Announcements),
                13 => (ChannelKind.Voice, DiscordChannelType.Stage),
                15 => (ChannelKind.Text, DiscordChannelType.Forum),
                16 => (ChannelKind.Text, DiscordChannelType.Media),
                _ => null,
            };
            if (shape is not { } known)
            {
                continue;
            }

            mapped.Add(new ChannelSnapshot
            {
                Id = channel.Id!,
                Name = channel.Name ?? "",
                Kind = known.Kind,
                DiscordType = known.Type,
                Topic = channel.Topic,
                SlowModeSeconds = channel.RateLimitPerUser ?? 0,
                UserLimit = channel.UserLimit ?? 0,
                Position = channel.Position ?? 0,
                SectionId = channel.ParentId is { } parent && sectionIds.Contains(parent) ? parent : null,
                Overwrites = Overwrites(channel.PermissionOverwrites),
            });
        }

        return (sections, mapped);
    }

    private static List<PermissionOverwriteSnapshot> Overwrites(IEnumerable<WireOverwrite?>? overwrites) =>
        (overwrites ?? [])
            .Select(overwrite => overwrite is { Id: { } id, Type: 0 or 1 }
                ? new PermissionOverwriteSnapshot(
                    overwrite.Type == 0 ? OverwriteTarget.Role : OverwriteTarget.Member,
                    id,
                    overwrite.Allow ?? "0",
                    overwrite.Deny ?? "0")
                : null)
            .OfType<PermissionOverwriteSnapshot>()
            .ToList();

    public static MemberSnapshot? Member(WireMember member, string serverId)
    {
        if (member.User?.Id is not { } id)
        {
            return null;
        }

        return new MemberSnapshot
        {
            Id = id,
            Username = member.User.Username ?? "",
            DisplayName = member.User.GlobalName,
            Nickname = member.Nick,
            AvatarUrl = DiscordCdn.Avatar(id, member.User.Avatar),
            ServerAvatarUrl = DiscordCdn.MemberAvatar(serverId, id, member.Avatar),
            RoleIds = (member.Roles ?? []).Where(role => role is not null && role != serverId).ToList(),
            IsBot = member.User.Bot ?? false,
            JoinedAt = Time(member.JoinedAt),
        };
    }

    public static EmojiSnapshot? Emoji(WireEmoji emoji)
    {
        if (emoji.Id is not { } id)
        {
            return null;
        }

        var animated = emoji.Animated ?? false;
        return new EmojiSnapshot(id, emoji.Name ?? "", animated, emoji.Available ?? true, DiscordCdn.Emoji(id, animated));
    }

    public static StickerSnapshot? Sticker(WireSticker sticker)
    {
        if (sticker.Id is not { } id)
        {
            return null;
        }

        var format = Format(sticker.FormatType);
        return new StickerSnapshot(id, sticker.Name ?? "", sticker.Description, format, DiscordCdn.Sticker(id, format));
    }

    public static ThreadSnapshot? Thread(WireChannel thread)
    {
        if (thread.Id is not { } id || thread.ParentId is not { } parent || thread.Type is not (10 or 11 or 12))
        {
            return null;
        }

        var isPrivate = thread.Type == 12;
        return new ThreadSnapshot
        {
            Id = id,
            ParentChannelId = parent,
            Name = thread.Name ?? "",
            // Discord gives a public thread the id of the message it was started from.
            StarterMessageId = isPrivate ? null : id,
            Archived = thread.ThreadMetadata?.Archived ?? false,
            Locked = thread.ThreadMetadata?.Locked ?? false,
            Private = isPrivate,
            CreatedAt = Time(thread.ThreadMetadata?.CreateTimestamp),
        };
    }

    public static MessageSnapshot? Message(WireMessage message)
    {
        if (message.Id is not { } id || message.ChannelId is not { } channelId)
        {
            return null;
        }

        var type = message.Type ?? 0;
        var reference = message.MessageReference;
        var forwarded = reference?.Type == ForwardReference &&
            message.MessageSnapshots?.FirstOrDefault()?.Message is { } copy
                ? new ForwardedSnapshot(copy.Content ?? "", Attachments(copy.Attachments))
                : null;

        return new MessageSnapshot
        {
            Id = id,
            ChannelId = channelId,
            Author = Author(message.Author, message.WebhookId),
            Content = message.Content ?? "",
            CreatedAt = Time(message.Timestamp) ?? SnowflakeTime(id) ?? DateTimeOffset.UnixEpoch,
            EditedAt = Time(message.EditedTimestamp),
            ReplyToMessageId = type == ReplyType && (reference?.Type ?? 0) == 0 ? reference?.MessageId : null,
            Pinned = message.Pinned ?? false,
            Type = type,
            IsSystemNotice = !SpokenTypes.Contains(type),
            Attachments = Attachments(message.Attachments),
            Embeds = (message.Embeds ?? [])
                .OfType<WireEmbed>()
                .Select(embed => new EmbedSummary(
                    embed.Type, embed.Url, embed.Title, embed.Description, embed.Image?.Url ?? embed.Thumbnail?.Url))
                .ToList(),
            Stickers = (message.StickerItems ?? [])
                .Select(sticker => sticker?.Id is { } stickerId
                    ? new StickerReference(
                        stickerId,
                        sticker.Name ?? "",
                        Format(sticker.FormatType),
                        DiscordCdn.Sticker(stickerId, Format(sticker.FormatType)))
                    : null)
                .OfType<StickerReference>()
                .ToList(),
            Reactions = (message.Reactions ?? [])
                .Select(reaction => reaction?.Emoji is { } emoji
                    ? new ReactionSnapshot(
                        new ReactionEmoji(emoji.Id, emoji.Name ?? "", emoji.Animated ?? false),
                        reaction.Count ?? 0,
                        reaction.CountDetails?.Burst ?? 0)
                    : null)
                .OfType<ReactionSnapshot>()
                .ToList(),
            StartedThreadId = message.Thread?.Id ?? (((message.Flags ?? 0) & HasThreadFlag) != 0 ? id : null),
            Forwarded = forwarded,
        };
    }

    public static ReactionUser? ReactionUser(WireUser user) =>
        user.Id is { } id ? new ReactionUser(id, user.Username ?? "", user.GlobalName, user.Bot ?? false) : null;

    private static AuthorSnapshot Author(WireUser? author, string? webhookId)
    {
        var id = author?.Id ?? webhookId ?? "";
        return new AuthorSnapshot
        {
            Id = id,
            Username = author?.Username ?? "",
            DisplayName = author?.GlobalName,
            AvatarUrl = id.Length > 0 ? DiscordCdn.Avatar(id, author?.Avatar) : null,
            IsBot = author?.Bot ?? false,
            WebhookId = webhookId,
        };
    }

    private static List<AttachmentSnapshot> Attachments(IEnumerable<WireAttachment?>? attachments) =>
        (attachments ?? [])
            .Select(attachment => attachment is { Id: { } id, Url: { } url }
                ? new AttachmentSnapshot
                {
                    Id = id,
                    FileName = attachment.Filename ?? id,
                    Size = attachment.Size ?? 0,
                    ContentType = attachment.ContentType,
                    Url = url,
                    Width = attachment.Width,
                    Height = attachment.Height,
                    DurationSeconds = attachment.DurationSecs,
                }
                : null)
            .OfType<AttachmentSnapshot>()
            .ToList();

    private static StickerFormat Format(int? formatType) =>
        formatType is { } value && Enum.IsDefined(typeof(StickerFormat), value) ? (StickerFormat)value : StickerFormat.Png;

    internal static DateTimeOffset? Time(string? text) =>
        DateTimeOffset.TryParse(text, CultureInfo.InvariantCulture, DateTimeStyles.AssumeUniversal, out var time)
            ? time
            : null;

    /// <summary>When a snowflake was made: its top 42 bits are milliseconds since Discord's epoch.</summary>
    internal static DateTimeOffset? SnowflakeTime(string id) =>
        ulong.TryParse(id, NumberStyles.None, CultureInfo.InvariantCulture, out var snowflake)
            ? DateTimeOffset.FromUnixTimeMilliseconds((long)(snowflake >> 22) + DiscordEpochMilliseconds)
            : null;
}
