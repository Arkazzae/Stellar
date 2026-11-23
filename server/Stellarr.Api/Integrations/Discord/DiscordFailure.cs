namespace Stellarr.Api.Integrations.Discord;

/// <summary>What went wrong talking to Discord, in terms an import can act on.</summary>
public enum DiscordFailureKind
{
    /// <summary>
    /// The token is wrong, revoked or not a bot's (HTTP 401). Stop using it:
    /// Discord shuts out an address after too many refused requests.
    /// </summary>
    InvalidToken,

    /// <summary>
    /// The bot may not see this server or channel, or lacks a permission the
    /// read needs (HTTP 403). <see cref="DiscordApiException.DiscordCode"/> says
    /// which: 50001 for no access, 50013 for a missing permission.
    /// </summary>
    MissingAccess,

    /// <summary>No such server, channel, message, template or emoji (HTTP 404).</summary>
    UnknownResource,

    /// <summary>
    /// The app lacks a privileged intent the read needs, which its owner turns
    /// on in the Developer Portal, on the Bot page.
    /// </summary>
    MissingIntent,

    /// <summary>
    /// Discord asked to wait longer than the reader may wait, or kept refusing
    /// after the reader waited; <see cref="DiscordApiException.RetryAfter"/>
    /// says how long Discord asked for.
    /// </summary>
    RateLimited,

    /// <summary>Discord could not be reached, did not answer in time, or failed itself (HTTP 5xx).</summary>
    Unreachable,

    /// <summary>Discord answered with a status or a body the reader does not understand.</summary>
    UnexpectedResponse,
}

/// <summary>
/// A request to Discord that did not give what it was asked for.
/// </summary>
/// <remarks>
/// The message names the route with its ids, the status and Discord's own
/// message, and never the token: requests carry it in a header that no
/// message or log line here prints, and Discord's text is checked for it.
/// </remarks>
public sealed class DiscordApiException(
    DiscordFailureKind kind,
    string message,
    string route,
    int? statusCode = null,
    int? discordCode = null,
    TimeSpan? retryAfter = null,
    Exception? innerException = null) : Exception(message, innerException)
{
    public DiscordFailureKind Kind { get; } = kind;

    /// <summary>The method and path that failed, such as <c>GET guilds/1/members</c>.</summary>
    public string Route { get; } = route;

    public int? StatusCode { get; } = statusCode;

    /// <summary>Discord's JSON error code, such as 50001 or 10003, when it gave one.</summary>
    public int? DiscordCode { get; } = discordCode;

    /// <summary>For <see cref="DiscordFailureKind.RateLimited"/>, how long Discord asked to wait.</summary>
    public TimeSpan? RetryAfter { get; } = retryAfter;
}
