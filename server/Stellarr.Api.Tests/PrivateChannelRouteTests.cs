using System.Net;
using System.Net.Http.Json;
using System.Reflection;
using System.Text.RegularExpressions;
using Microsoft.AspNetCore.Http.Metadata;
using Microsoft.AspNetCore.Routing;
using Microsoft.AspNetCore.SignalR;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Realtime;

namespace Stellarr.Api.Tests;

/// <summary>
/// A channel somebody cannot view does not exist for them. Every route that
/// takes a channel, a message, an attachment or a section, and every hub
/// method that takes a channel, is asked about the private places by members
/// who do not view them, and must answer exactly as for an id that does not
/// exist: the same status, the same body, and nothing from inside.
/// </summary>
/// <remarks>
/// The list of routes comes from the application's own endpoints, so a route
/// added later that names one of these ids fails until it has a probe here.
/// </remarks>
public sealed partial class PrivateChannelRouteTests
{
    /// <summary>The route values that name a channel, a section, or something in a channel.</summary>
    private static readonly HashSet<string> Scoped =
        new(StringComparer.Ordinal) { "channelId", "messageId", "attachmentId", "sectionId", "targetId", "rootId" };

    /// <summary>The ids a probe fills in: the private ones, or ids nothing has.</summary>
    private sealed record Ids(
        Guid Channel, Guid Voice, Guid Section, Guid SectionChannel, Guid Root, Guid OwnMessage, Guid Attachment, Guid Thumbnail)
    {
        public static Ids Private(PrivateChannelScenario state, string actor) => new(
            PrivateChannelScenario.Hidden,
            PrivateChannelScenario.HiddenVoice,
            state.HiddenSection,
            state.SectionChannel,
            state.Root,
            actor == "jo" ? state.JosMessage : state.NilsMessage,
            state.Attachment,
            state.Thumbnail);

        public static Ids Unknown() => new(
            Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid());
    }

    /// <summary>One request, built for a set of ids.</summary>
    private sealed record Probe(string Name, Func<Ids, HttpRequestMessage> Build, bool OwnerSees = false);

    private static HttpRequestMessage Get(string path) => new(HttpMethod.Get, path);

    private static HttpRequestMessage Delete(string path) => new(HttpMethod.Delete, path);

    private static HttpRequestMessage Send(HttpMethod method, string path, object body) =>
        new(method, path) { Content = JsonContent.Create(body) };

    private static readonly Guid Bystander = PrivateChannelScenario.IlyaId;
    private static readonly string[] SendOnly = ["sendMessages"];
    private static readonly string[] NoPermissions = [];

    /// <summary>Every route that names one of the scoped ids, with the requests that probe it.</summary>
    private static readonly Dictionary<string, Probe[]> Routes = new(StringComparer.Ordinal)
    {
        ["GET /assets/{attachmentId:guid}"] =
        [
            new("attachment", ids => Get($"/assets/{ids.Attachment}"), OwnerSees: true),
            new("embed thumbnail", ids => Get($"/assets/{ids.Thumbnail}"), OwnerSees: true),
        ],
        ["GET /assets/{attachmentId:guid}/poster"] = [new("poster", ids => Get($"/assets/{ids.Attachment}/poster"))],
        ["GET /assets/{attachmentId:guid}/preview"] = [new("preview", ids => Get($"/assets/{ids.Attachment}/preview"))],
        ["GET /api/channels/{channelId:guid}/messages"] =
            [new("history", ids => Get($"/api/channels/{ids.Channel}/messages"), OwnerSees: true)],
        ["GET /api/channels/{channelId:guid}/messages/sync"] =
            [new("sync", ids => Get($"/api/channels/{ids.Channel}/messages/sync?after=0"), OwnerSees: true)],
        ["GET /api/channels/{channelId:guid}/messages/search"] =
            [new("search", ids => Get($"/api/channels/{ids.Channel}/messages/search?query=tajne"), OwnerSees: true)],
        ["GET /api/channels/{channelId:guid}/messages/index"] =
        [
            new("index", ids => Get($"/api/channels/{ids.Channel}/messages/index"), OwnerSees: true),
            new("index of media", ids => Get($"/api/channels/{ids.Channel}/messages/index?kind=media&query=tajne"), OwnerSees: true),
        ],
        ["GET /api/channels/{channelId:guid}/threads"] =
            [new("threads", ids => Get($"/api/channels/{ids.Channel}/threads"), OwnerSees: true)],
        ["GET /api/channels/{channelId:guid}/threads/{rootId:guid}"] =
        [
            new("thread", ids => Get($"/api/channels/{ids.Channel}/threads/{ids.Root}"), OwnerSees: true),
            new("thread asked of an open channel", ids => Get($"/api/channels/{PrivateChannelScenario.Open}/threads/{ids.Root}")),
        ],
        ["GET /api/channels/{channelId:guid}/pins"] =
            [new("pins", ids => Get($"/api/channels/{ids.Channel}/pins"), OwnerSees: true)],
        ["POST /api/channels/{channelId:guid}/messages"] =
        [
            new("post", ids => Send(HttpMethod.Post, $"/api/channels/{ids.Channel}/messages", new { content = "próba" })),
            new("post with a known client id", ids => Send(
                HttpMethod.Post, $"/api/channels/{ids.Channel}/messages", new { content = "tajne jo", clientMessageId = ids.OwnMessage })),
            new("reply", ids => Send(
                HttpMethod.Post, $"/api/channels/{ids.Channel}/messages", new { content = "próba", replyToMessageId = ids.Root })),
        ],
        ["PATCH /api/messages/{messageId:guid}"] =
            [new("edit own", ids => Send(HttpMethod.Patch, $"/api/messages/{ids.OwnMessage}", new { content = "zmiana" }))],
        ["DELETE /api/messages/{messageId:guid}"] =
        [
            new("delete own", ids => Delete($"/api/messages/{ids.OwnMessage}")),
            new("delete another's", ids => Delete($"/api/messages/{ids.Root}")),
        ],
        ["PUT /api/messages/{messageId:guid}/reactions/{emoji}"] =
            [new("react", ids => new HttpRequestMessage(HttpMethod.Put, $"/api/messages/{ids.Root}/reactions/{Uri.EscapeDataString("🎉")}"))],
        ["DELETE /api/messages/{messageId:guid}/reactions/{emoji}"] =
            [new("take a reaction back", ids => Delete($"/api/messages/{ids.Root}/reactions/{Uri.EscapeDataString("👍")}"))],
        ["PUT /api/messages/{messageId:guid}/pin"] =
            [new("pin", ids => new HttpRequestMessage(HttpMethod.Put, $"/api/messages/{ids.Root}/pin"))],
        ["DELETE /api/messages/{messageId:guid}/pin"] = [new("unpin", ids => Delete($"/api/messages/{ids.Root}/pin"))],
        ["PUT /api/channels/{channelId:guid}/read"] =
            [new("mark read", ids => new HttpRequestMessage(HttpMethod.Put, $"/api/channels/{ids.Channel}/read"))],
        ["PATCH /api/admin/channels/{channelId:guid}"] =
        [
            new("channel settings", ids => Send(HttpMethod.Patch, $"/api/admin/channels/{ids.Channel}", new { topic = "nowy" })),
            new("voice channel settings", ids => Send(HttpMethod.Patch, $"/api/admin/channels/{ids.Voice}", new { userLimit = 5 })),
            new("a channel of a private section", ids => Send(
                HttpMethod.Patch, $"/api/admin/channels/{ids.SectionChannel}", new { name = "inna" })),
            new("a rename to a free name", ids => Send(HttpMethod.Patch, $"/api/admin/channels/{ids.Channel}", new { name = "wolna" })),
        ],
        ["DELETE /api/admin/channels/{channelId:guid}"] =
        [
            new("delete a channel", ids => Delete($"/api/admin/channels/{ids.Channel}")),
            new("delete a voice channel", ids => Delete($"/api/admin/channels/{ids.Voice}")),
        ],
        ["DELETE /api/admin/channels/{channelId:guid}/members/{userId:guid}"] =
            [new("disconnect from voice", ids => Delete($"/api/admin/channels/{ids.Voice}/members/{Bystander}"))],
        ["PATCH /api/admin/channels/{channelId:guid}/members/{userId:guid}"] =
        [
            new("move out of a private room", ids => Send(
                HttpMethod.Patch, $"/api/admin/channels/{ids.Voice}/members/{Bystander}",
                new { targetChannelId = PrivateChannelScenario.OpenVoice })),
            new("move into a private room", ids => Send(
                HttpMethod.Patch, $"/api/admin/channels/{PrivateChannelScenario.OpenVoice}/members/{Bystander}",
                new { targetChannelId = ids.Voice })),
        ],
        ["PATCH /api/admin/sections/{sectionId:guid}"] =
            [new("rename a section", ids => Send(HttpMethod.Patch, $"/api/admin/sections/{ids.Section}", new { name = "Inna" }))],
        ["DELETE /api/admin/sections/{sectionId:guid}"] =
            [new("delete a section", ids => Delete($"/api/admin/sections/{ids.Section}"))],
    };

    /// <summary>
    /// Routes whose request body names a channel or a section. Nothing in the
    /// route says so, so they are listed by hand.
    /// </summary>
    private static readonly Dictionary<string, Probe[]> BodyRoutes = new(StringComparer.Ordinal)
    {
        ["POST /api/admin/invites"] =
            [new("an invite to the channel", ids => Send(HttpMethod.Post, "/api/admin/invites", new { channelId = ids.Channel }))],
        ["POST /api/admin/channels"] =
        [
            new("a channel in the section", ids => Send(
                HttpMethod.Post, "/api/admin/channels", new { name = "nowy", kind = "text", sectionId = ids.Section })),
        ],
        ["PATCH /api/admin/notifications"] =
            [new("mute the channel", ids => Send(HttpMethod.Patch, "/api/admin/notifications", new { mutedChannelIds = new[] { ids.Channel } }))],
    };

    static PrivateChannelRouteTests()
    {
        // The override routes are the same six for a channel and a section.
        foreach (var (collection, target) in new (string, Func<Ids, Guid>)[] { ("channels", x => x.Channel), ("sections", x => x.Section) })
        {
            var root = $"/api/admin/{collection}/{{targetId:guid}}/permissions";
            string At(Ids ids) => $"/api/admin/{collection}/{target(ids)}/permissions";
            Routes[$"GET {root}"] = [new($"{collection}: list role overrides", ids => Get(At(ids)), OwnerSees: true)];
            Routes[$"PUT {root}/{{roleId:guid}}"] =
                [new($"{collection}: set a role override", ids => Send(HttpMethod.Put, $"{At(ids)}/{PrivateChannelScenario.MemberRoleId}",
                    new { allow = NoPermissions, deny = SendOnly }))];
            Routes[$"DELETE {root}/{{roleId:guid}}"] =
                [new($"{collection}: clear a role override", ids => Delete($"{At(ids)}/{PrivateChannelScenario.MemberRoleId}"))];
            Routes[$"GET {root}/members"] = [new($"{collection}: list member overrides", ids => Get($"{At(ids)}/members"), OwnerSees: true)];
            Routes[$"PUT {root}/members/{{userId:guid}}"] =
                [new($"{collection}: set a member override", ids => Send(HttpMethod.Put, $"{At(ids)}/members/{Bystander}",
                    new { allow = NoPermissions, deny = SendOnly }))];
            Routes[$"DELETE {root}/members/{{userId:guid}}"] =
                [new($"{collection}: clear a member override", ids => Delete($"{At(ids)}/members/{Bystander}"))];
        }
    }

    [Fact]
    public void EveryRouteThatNamesAChannelOrWhatIsInOneIsProbed()
    {
        using var factory = new StellarrWebApplicationFactory();
        var scoped = factory.Services.GetRequiredService<EndpointDataSource>().Endpoints
            .OfType<RouteEndpoint>()
            .Where(endpoint => endpoint.RoutePattern.Parameters.Any(parameter => Scoped.Contains(parameter.Name)))
            .SelectMany(endpoint => (endpoint.Metadata.GetMetadata<IHttpMethodMetadata>()?.HttpMethods ?? ["ANY"])
                // A group's own path ends in a slash.
                .Select(method => $"{method} {endpoint.RoutePattern.RawText?.TrimEnd('/')}"))
            .ToHashSet(StringComparer.Ordinal);

        var unprobed = scoped.Except(Routes.Keys).Order(StringComparer.Ordinal).ToArray();
        var gone = Routes.Keys.Except(scoped).Order(StringComparer.Ordinal).ToArray();
        Assert.True(unprobed.Length == 0, "Not probed: " + string.Join(", ", unprobed));
        Assert.True(gone.Length == 0, "Probed but not mapped: " + string.Join(", ", gone));
    }

    [Theory]
    [InlineData("jo")]
    [InlineData("nils")]
    public async Task ANonViewerIsAnsweredAsIfNothingWereThere(string actor)
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        var client = actor == "jo" ? state.Jo : state.Nils;
        var failures = new List<string>();
        foreach (var (route, probes) in Routes.Concat(BodyRoutes))
        {
            foreach (var probe in probes)
            {
                var hidden = await AnswerAsync(client, probe.Build(Ids.Private(state, actor)));
                var unknown = await AnswerAsync(client, probe.Build(Ids.Unknown()));
                if (hidden.Status != unknown.Status || hidden.Body != unknown.Body)
                {
                    failures.Add($"{route} ({probe.Name}): {(int)hidden.Status} {Shorten(hidden.Body)}, " +
                        $"where an unknown id gets {(int)unknown.Status} {Shorten(unknown.Body)}");
                }
                else if (PrivateChannelScenario.Secrets.FirstOrDefault(secret => hidden.Body.Contains(secret, StringComparison.Ordinal)) is { } secret)
                {
                    failures.Add($"{route} ({probe.Name}): the answer says \"{secret}\"");
                }
            }
        }

        Assert.True(failures.Count == 0, string.Join(Environment.NewLine, failures.Prepend($"{failures.Count} answers give something away:")));
    }

    /// <summary>
    /// The probes reach real things: where the owner may read, the private
    /// ids get the owner something an unknown id does not.
    /// </summary>
    [Fact]
    public async Task TheProbesAskAboutThingsThatExist()
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        var failures = new List<string>();
        foreach (var (route, probes) in Routes)
        {
            foreach (var probe in probes.Where(x => x.OwnerSees))
            {
                var hidden = await AnswerAsync(state.Owner, probe.Build(Ids.Private(state, "nils")));
                var unknown = await AnswerAsync(state.Owner, probe.Build(Ids.Unknown()));
                if (hidden.Status == unknown.Status && hidden.Body == unknown.Body)
                {
                    failures.Add($"{route} ({probe.Name}): {(int)hidden.Status} for both");
                }
            }
        }

        Assert.True(failures.Count == 0, string.Join(Environment.NewLine, failures));
    }

    private static async Task<(HttpStatusCode Status, string Body)> AnswerAsync(HttpClient client, HttpRequestMessage request)
    {
        using (request)
        using (var response = await client.SendAsync(request))
        {
            var body = await response.Content.ReadAsStringAsync();
            return (response.StatusCode, TraceId().Replace(body, ""));
        }
    }

    private static string Shorten(string body) => body.Length <= 160 ? body : body[..160] + "…";

    [GeneratedRegex("\"traceId\":\"[^\"]*\",?")]
    private static partial Regex TraceId();

    /// <summary>What every hub method that takes a channel answers a non-viewer.</summary>
    private static readonly Dictionary<string, Func<HubConnection, Guid, Task>> HubProbes = new(StringComparer.Ordinal)
    {
        ["JoinChannel"] = (hub, channel) => hub.InvokeAsync("JoinChannel", channel.ToString()),
        ["SendTyping"] = (hub, channel) => hub.InvokeAsync("SendTyping", channel.ToString()),
        ["JoinVoice"] = (hub, channel) => hub.InvokeAsync("JoinVoice", channel.ToString()),
        ["OpenMediaSession"] = (hub, channel) => hub.InvokeAsync<object>("OpenMediaSession", channel.ToString()),
        ["PlaySoundboard"] = (hub, channel) => hub.InvokeAsync("PlaySoundboard", channel.ToString(), Guid.NewGuid().ToString()),
        ["SendVoiceSignal"] = (hub, channel) => hub.InvokeAsync("SendVoiceSignal", new
        {
            protocolVersion = RtcProtocol.Version, channelId = channel.ToString(), kind = "state", payload = "{}",
        }),
        ["SendMediaSignal"] = (hub, channel) => hub.InvokeAsync("SendMediaSignal", new
        {
            protocolVersion = RtcProtocol.Version, channelId = channel.ToString(), sessionId = "s", kind = "offer", payload = "{}",
        }),
    };

    /// <summary>Hub methods that take no channel, and why.</summary>
    private static readonly Dictionary<string, string> HubExempt = new(StringComparer.Ordinal)
    {
        ["SetPresence"] = "presence belongs to the account",
        ["LeaveVoice"] = "leaves whatever seat the connection has, and names none",
        ["DeclineDirectCall"] = "names a direct conversation, never a channel",
        ["GetRtcClock"] = "the server's clock",
    };

    [Fact]
    public void EveryHubMethodIsProbedOrNamesNoChannel()
    {
        var methods = typeof(AppHub).GetMethods(BindingFlags.Public | BindingFlags.Instance | BindingFlags.DeclaredOnly)
            .Where(method => !method.IsSpecialName && method.Name is not ("OnConnectedAsync" or "OnDisconnectedAsync"))
            .Select(method => method.Name)
            .ToHashSet(StringComparer.Ordinal);

        Assert.Empty(methods.Except(HubProbes.Keys).Except(HubExempt.Keys).Order(StringComparer.Ordinal));
        Assert.Empty(HubProbes.Keys.Concat(HubExempt.Keys).Except(methods).Order(StringComparer.Ordinal));
    }

    [Theory]
    [InlineData("jo")]
    [InlineData("nils")]
    public async Task ANonViewerIsAnsweredByTheHubAsIfNothingWereThere(string actor)
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        await using var log = await state.ListenAsync(actor);
        var failures = new List<string>();
        foreach (var (method, probe) in HubProbes)
        {
            foreach (var channel in new[] { PrivateChannelScenario.Hidden, PrivateChannelScenario.HiddenVoice, state.SectionChannel })
            {
                var hidden = await RefusalAsync(probe(log.Connection, channel));
                var unknown = await RefusalAsync(probe(log.Connection, Guid.NewGuid()));
                if (hidden != unknown)
                {
                    failures.Add($"{method} on {channel}: \"{hidden}\", where an unknown id gets \"{unknown}\"");
                }
            }
        }

        Assert.True(failures.Count == 0, string.Join(Environment.NewLine, failures));
    }

    private static async Task<string> RefusalAsync(Task call)
    {
        try
        {
            await call;
            return "(accepted)";
        }
        catch (HubException refused)
        {
            return refused.Message;
        }
    }
}
