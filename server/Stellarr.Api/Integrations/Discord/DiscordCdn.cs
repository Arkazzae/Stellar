namespace Stellarr.Api.Integrations.Discord;

/// <summary>
/// Addresses of images on Discord's CDN, and the check every download from it
/// must pass.
/// </summary>
/// <remarks>
/// An import downloads what a Discord server points at: avatars, icons,
/// emoji, stickers and attachments. Those addresses arrive inside API answers,
/// and an attachment's address in particular is a signed URL Discord wrote,
/// so the downloader takes nothing on trust: only https on the two hosts the
/// documentation names is fetched at all.
/// </remarks>
public static class DiscordCdn
{
    /// <summary>The image CDN, also the host of signed attachment URLs.</summary>
    public const string CdnHost = "cdn.discordapp.com";

    /// <summary>The media proxy: attachments' <c>proxy_url</c>, and the only home of GIF stickers.</summary>
    public const string MediaHost = "media.discordapp.net";

    private const string CdnBase = "https://" + CdnHost + "/";

    /// <summary>
    /// Whether <paramref name="url"/> may be downloaded: absolute https, exactly
    /// one of the two CDN hosts, no login part and no port but the default.
    /// </summary>
    public static bool IsCdnUrl(string? url) =>
        Uri.TryCreate(url, UriKind.Absolute, out var parsed) && IsCdnUrl(parsed);

    /// <inheritdoc cref="IsCdnUrl(string?)"/>
    public static bool IsCdnUrl(Uri url) =>
        url.IsAbsoluteUri &&
        url.Scheme == Uri.UriSchemeHttps &&
        url.UserInfo.Length == 0 &&
        url.IsDefaultPort &&
        (url.IdnHost.Equals(CdnHost, StringComparison.OrdinalIgnoreCase) ||
            url.IdnHost.Equals(MediaHost, StringComparison.OrdinalIgnoreCase));

    /// <summary>A server's icon; null when it has none.</summary>
    public static string? ServerIcon(string serverId, string? hash) =>
        Image($"icons/{Segment(serverId)}", hash);

    /// <summary>A user's (or a webhook's) own avatar; null for Discord's default one.</summary>
    public static string? Avatar(string userId, string? hash) =>
        Image($"avatars/{Segment(userId)}", hash);

    /// <summary>The avatar a member set for one server only.</summary>
    public static string? MemberAvatar(string serverId, string userId, string? hash) =>
        Image($"guilds/{Segment(serverId)}/users/{Segment(userId)}/avatars", hash);

    /// <summary>
    /// A custom emoji, as WebP: Discord serves every emoji as WebP, while one
    /// uploaded as WebP or AVIF does not convert cleanly to the other formats.
    /// </summary>
    public static string Emoji(string emojiId, bool animated) =>
        $"{CdnBase}emojis/{Segment(emojiId)}.webp" + (animated ? "?animated=true" : "");

    /// <summary>
    /// A sticker in the one format Discord stores it in: PNG (also APNG),
    /// Lottie JSON, or GIF, which lives on the media host instead.
    /// </summary>
    public static string Sticker(string stickerId, StickerFormat format) => format switch
    {
        StickerFormat.Gif => $"https://{MediaHost}/stickers/{Segment(stickerId)}.gif",
        StickerFormat.Lottie => $"{CdnBase}stickers/{Segment(stickerId)}.json",
        _ => $"{CdnBase}stickers/{Segment(stickerId)}.png",
    };

    /// <summary>
    /// An image hash under <paramref name="path"/>. A hash that starts with
    /// <c>a_</c> is animated and is asked for as animated WebP, which Discord
    /// recommends and Stellarr accepts; a still one as PNG.
    /// </summary>
    private static string? Image(string path, string? hash)
    {
        if (string.IsNullOrEmpty(hash))
        {
            return null;
        }

        return hash.StartsWith("a_", StringComparison.Ordinal)
            ? $"{CdnBase}{path}/{Segment(hash)}.webp?animated=true"
            : $"{CdnBase}{path}/{Segment(hash)}.png";
    }

    private static string Segment(string value) => Uri.EscapeDataString(value);
}

/// <summary>The file format a sticker is stored in; Discord's <c>format_type</c>.</summary>
public enum StickerFormat
{
    Png = 1,
    Apng = 2,
    Lottie = 3,
    Gif = 4,
}
