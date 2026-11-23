using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Stellarr.Api.Integrations.Discord;

/// <summary>
/// How Discord's JSON is read: snake_case names, unknown fields ignored,
/// absent fields left null, and every string field willing to take a number.
/// </summary>
/// <remarks>
/// Discord sends snowflakes and permission sets as strings, but a template's
/// placeholder ids are integers, and the documentation's own example gives
/// permissions as a number. Rather than know which field does what, any
/// string reads a number as its digits. Timestamps stay strings too: the
/// archived-thread and pin pages are asked for with the timestamp Discord gave,
/// exactly as it gave it, and one odd date must not fail a whole page.
/// </remarks>
internal static class DiscordJson
{
    public static readonly JsonSerializerOptions Options = new(JsonSerializerDefaults.Web)
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        NumberHandling = JsonNumberHandling.AllowReadingFromString,
        Converters = { new LenientStringConverter() },
    };

    private sealed class LenientStringConverter : JsonConverter<string>
    {
        public override bool HandleNull => false;

        public override string? Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
        {
            switch (reader.TokenType)
            {
                case JsonTokenType.String:
                    return reader.GetString();
                case JsonTokenType.Number:
                    return reader.HasValueSequence
                        ? Encoding.UTF8.GetString(reader.ValueSequence)
                        : Encoding.UTF8.GetString(reader.ValueSpan);
                case JsonTokenType.True:
                    return "true";
                case JsonTokenType.False:
                    return "false";
                default:
                    // An object or array where text was expected reads as absent.
                    reader.Skip();
                    return null;
            }
        }

        public override void Write(Utf8JsonWriter writer, string value, JsonSerializerOptions options) =>
            writer.WriteStringValue(value);
    }
}

// The shapes below hold only the fields the import reads; every one is
// optional, because Discord documents many as such and adds new ones freely.

internal sealed record WireUser
{
    public string? Id { get; init; }
    public string? Username { get; init; }
    public string? GlobalName { get; init; }
    public string? Avatar { get; init; }
    public bool? Bot { get; init; }
}

internal sealed record WireApplication
{
    public string? Id { get; init; }
    public string? Name { get; init; }
    public long? Flags { get; init; }
}

internal sealed record WireGuild
{
    public string? Id { get; init; }
    public string? Name { get; init; }
    public string? Description { get; init; }
    public string? Icon { get; init; }

    /// <summary>The icon in a template's copy of the server, which has no <c>icon</c>.</summary>
    public string? IconHash { get; init; }

    public string? OwnerId { get; init; }
    public int? ApproximateMemberCount { get; init; }
    public int? ApproximatePresenceCount { get; init; }
    public List<WireRole>? Roles { get; init; }
    public List<WireChannel>? Channels { get; init; }
}

internal sealed record WireTemplate
{
    public string? Code { get; init; }
    public string? Name { get; init; }
    public string? SourceGuildId { get; init; }
    public WireGuild? SerializedSourceGuild { get; init; }
}

internal sealed record WireRole
{
    public string? Id { get; init; }
    public string? Name { get; init; }
    public int? Color { get; init; }
    public WireRoleColors? Colors { get; init; }
    public bool? Hoist { get; init; }
    public int? Position { get; init; }
    public string? Permissions { get; init; }
    public bool? Managed { get; init; }
    public bool? Mentionable { get; init; }
}

internal sealed record WireRoleColors
{
    public int? PrimaryColor { get; init; }
}

internal sealed record WireChannel
{
    public string? Id { get; init; }
    public int? Type { get; init; }
    public int? Position { get; init; }
    public List<WireOverwrite>? PermissionOverwrites { get; init; }
    public string? Name { get; init; }
    public string? Topic { get; init; }
    public int? RateLimitPerUser { get; init; }
    public int? UserLimit { get; init; }
    public string? ParentId { get; init; }
    public WireThreadMetadata? ThreadMetadata { get; init; }
}

internal sealed record WireOverwrite
{
    public string? Id { get; init; }
    public int? Type { get; init; }
    public string? Allow { get; init; }
    public string? Deny { get; init; }
}

internal sealed record WireThreadMetadata
{
    public bool? Archived { get; init; }
    public bool? Locked { get; init; }
    public string? ArchiveTimestamp { get; init; }
    public string? CreateTimestamp { get; init; }
}

internal sealed record WireThreadList
{
    public List<WireChannel>? Threads { get; init; }
    public bool? HasMore { get; init; }
}

internal sealed record WireMember
{
    public WireUser? User { get; init; }
    public string? Nick { get; init; }
    public string? Avatar { get; init; }
    public List<string>? Roles { get; init; }
    public string? JoinedAt { get; init; }
}

internal sealed record WireEmoji
{
    public string? Id { get; init; }
    public string? Name { get; init; }
    public bool? Animated { get; init; }
    public bool? Available { get; init; }
}

internal sealed record WireSticker
{
    public string? Id { get; init; }
    public string? Name { get; init; }
    public string? Description { get; init; }
    public int? FormatType { get; init; }
}

internal sealed record WireMessage
{
    public string? Id { get; init; }
    public string? ChannelId { get; init; }
    public WireUser? Author { get; init; }
    public string? Content { get; init; }
    public string? Timestamp { get; init; }
    public string? EditedTimestamp { get; init; }
    public bool? Pinned { get; init; }
    public string? WebhookId { get; init; }
    public int? Type { get; init; }
    public long? Flags { get; init; }
    public WireMessageReference? MessageReference { get; init; }
    public List<WireMessageSnapshot>? MessageSnapshots { get; init; }
    public List<WireAttachment>? Attachments { get; init; }
    public List<WireEmbed>? Embeds { get; init; }
    public List<WireReaction>? Reactions { get; init; }
    public List<WireSticker>? StickerItems { get; init; }
    public WireChannel? Thread { get; init; }
}

internal sealed record WireMessageReference
{
    public int? Type { get; init; }
    public string? MessageId { get; init; }
    public string? ChannelId { get; init; }
}

internal sealed record WireMessageSnapshot
{
    public WireMessage? Message { get; init; }
}

internal sealed record WireAttachment
{
    public string? Id { get; init; }
    public string? Filename { get; init; }
    public string? ContentType { get; init; }
    public long? Size { get; init; }
    public string? Url { get; init; }
    public int? Width { get; init; }
    public int? Height { get; init; }
    public double? DurationSecs { get; init; }
}

internal sealed record WireEmbed
{
    public string? Type { get; init; }
    public string? Title { get; init; }
    public string? Description { get; init; }
    public string? Url { get; init; }
    public WireEmbedMedia? Image { get; init; }
    public WireEmbedMedia? Thumbnail { get; init; }
}

internal sealed record WireEmbedMedia
{
    public string? Url { get; init; }
}

internal sealed record WireReaction
{
    public int? Count { get; init; }
    public WireReactionCounts? CountDetails { get; init; }
    public WireEmoji? Emoji { get; init; }
}

internal sealed record WireReactionCounts
{
    public int? Burst { get; init; }
    public int? Normal { get; init; }
}

internal sealed record WirePin
{
    public string? PinnedAt { get; init; }
    public WireMessage? Message { get; init; }
}

internal sealed record WirePinList
{
    public List<WirePin>? Items { get; init; }
    public bool? HasMore { get; init; }
}

/// <summary>Discord's error body, which a 429 extends with how long to wait.</summary>
internal sealed record WireError
{
    public string? Message { get; init; }
    public int? Code { get; init; }
    public double? RetryAfter { get; init; }
    public bool? Global { get; init; }
}
