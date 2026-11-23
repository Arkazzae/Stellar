using System.Net;
using System.Text.Json;
using Stellarr.Api.Domain;
using Stellarr.Api.Integrations.Discord;

namespace Stellarr.Api.Tests;

/// <summary>
/// The bot's reads: which route each asks, how its pages are walked, and how
/// a whole server comes together.
/// </summary>
public sealed class DiscordBotReaderTests
{
    private const string Guild = "41771983423143936";
    private const string Channel = "290926798999357250";

    [Fact]
    public async Task MessagesAreReadNewestFirstAPageOfAHundredAtATime()
    {
        // 130 messages with ids 1001..1130; Discord answers newest first.
        var api = new DiscordFakeApi(request =>
        {
            var before = request.Query("before") is { } text ? long.Parse(text, System.Globalization.CultureInfo.InvariantCulture) : 1131;
            var limit = int.Parse(request.Query("limit")!, System.Globalization.CultureInfo.InvariantCulture);
            var ids = Enumerable.Range(0, limit).Select(step => before - 1 - step).Where(id => id >= 1001);
            return DiscordFakeApi.Json(Array(ids.Select(id => Message(id.ToString(System.Globalization.CultureInfo.InvariantCulture)))));
        });

        var messages = await Collect(DiscordClientTests.Bot(api).ReadMessagesAsync(Channel));

        Assert.Equal(130, messages.Count);
        Assert.Equal("1130", messages[0].Id);
        Assert.Equal("1001", messages[^1].Id);
        Assert.Equal(
            [$"/api/v10/channels/{Channel}/messages?limit=100", $"/api/v10/channels/{Channel}/messages?limit=100&before=1031"],
            api.Requests.Select(request => request.PathAndQuery));
    }

    [Fact]
    public async Task AMessageReadCanGoOnFromWhereItStopped()
    {
        var api = new DiscordFakeApi(DiscordFakeApi.Json(Array([Message("5"), Message("4")])));

        var messages = await Collect(DiscordClientTests.Bot(api).ReadMessagesAsync(Channel, before: "6"));

        Assert.Equal(["5", "4"], messages.Select(message => message.Id));
        Assert.Equal("6", Assert.Single(api.Requests).Query("before"));
    }

    [Fact]
    public async Task MembersArePagedByTheHighestIdAsANumber()
    {
        // Ids 1..1000 then 1001: as text, "999" would sort after "1000".
        var api = new DiscordFakeApi(
            DiscordFakeApi.Json(Array(Enumerable.Range(1, 1000).Select(id => Member(id.ToString(System.Globalization.CultureInfo.InvariantCulture))))),
            DiscordFakeApi.Json(Array([Member("1001")])));

        var members = await Collect(DiscordClientTests.Bot(api).ReadMembersAsync(Guild));

        Assert.Equal(1001, members.Count);
        Assert.Equal(
            [$"/api/v10/guilds/{Guild}/members?limit=1000", $"/api/v10/guilds/{Guild}/members?limit=1000&after=1000"],
            api.Requests.Select(request => request.PathAndQuery));
    }

    [Theory]
    // The app's flags decide, not the code: the docs name no code for a missing intent.
    [InlineData(50001, "0", DiscordFailureKind.MissingIntent)]
    [InlineData(50013, "0", DiscordFailureKind.MissingIntent)]
    [InlineData(50001, "32768", DiscordFailureKind.MissingAccess)]
    [InlineData(50001, "16384", DiscordFailureKind.MissingAccess)]
    [InlineData(50001, null, DiscordFailureKind.MissingAccess)]
    public async Task ARefusedMemberListIsAMissingIntentOnlyWhenTheAppSaysSo(int code, string? flags, DiscordFailureKind expected)
    {
        var api = new DiscordFakeApi(request => request.Uri.AbsolutePath switch
        {
            "/api/v10/applications/@me" when flags is not null => DiscordFakeApi.Json($$"""{"id":"1","name":"Importer","flags":{{flags}}}"""),
            "/api/v10/applications/@me" => DiscordFakeApi.Error(HttpStatusCode.Forbidden, 50001, "Missing Access"),
            _ => DiscordFakeApi.Error(HttpStatusCode.Forbidden, code, "Missing Access"),
        });

        var failure = await Assert.ThrowsAsync<DiscordApiException>(
            () => Collect(DiscordClientTests.Bot(api).ReadMembersAsync(Guild)));

        Assert.Equal(expected, failure.Kind);
        Assert.Equal(code, failure.DiscordCode);
        Assert.Equal($"GET guilds/{Guild}/members", failure.Route);
        Assert.Equal(["/api/v10/guilds/" + Guild + "/members", "/api/v10/applications/@me"], api.Requests.Select(request => request.Uri.AbsolutePath));
    }

    [Fact]
    public async Task ReactionUsersArePagedAndTheEmojiEscaped()
    {
        var api = new DiscordFakeApi(
            DiscordFakeApi.Json(Array(Enumerable.Range(1, 100).Select(id => User(id.ToString(System.Globalization.CultureInfo.InvariantCulture))))),
            DiscordFakeApi.Json(Array([User("101"), User("102")])),
            DiscordFakeApi.Json("[]"));
        var bot = DiscordClientTests.Bot(api);

        var fire = await Collect(bot.ReadReactionUsersAsync(Channel, "7", new ReactionEmoji(null, "🔥", false)));
        var custom = await Collect(bot.ReadReactionUsersAsync(Channel, "7", new ReactionEmoji("41771983429993937", "LUL", true), burst: true));

        Assert.Equal(102, fire.Count);
        Assert.Empty(custom);
        Assert.Equal(
            [
                $"/api/v10/channels/{Channel}/messages/7/reactions/%F0%9F%94%A5?type=0&limit=100",
                $"/api/v10/channels/{Channel}/messages/7/reactions/%F0%9F%94%A5?type=0&limit=100&after=100",
                $"/api/v10/channels/{Channel}/messages/7/reactions/LUL%3A41771983429993937?type=1&limit=100",
            ],
            api.Requests.Select(request => request.Uri.GetComponents(UriComponents.PathAndQuery, UriFormat.UriEscaped)));
    }

    [Fact]
    public async Task ArchivedThreadsArePagedByTheirArchiveTime()
    {
        var api = new DiscordFakeApi(
            DiscordFakeApi.Json($$"""
                { "has_more": true, "threads": [
                  {{Thread("21", "2024-05-02T10:00:00.000000+00:00")}},
                  {{Thread("20", "2024-05-01T10:00:00.000000+00:00")}} ] }
                """),
            DiscordFakeApi.Json($$"""{ "has_more": false, "threads": [ {{Thread("19", "2024-04-01T10:00:00.000000+00:00")}} ] }"""));

        var threads = await Collect(DiscordClientTests.Bot(api).ReadArchivedThreadsAsync(Channel, privateThreads: false));

        Assert.Equal(["21", "20", "19"], threads.Select(thread => thread.Id));
        Assert.All(threads, thread => Assert.True(thread.Archived));
        Assert.Equal($"/api/v10/channels/{Channel}/threads/archived/public", api.Requests[0].PathAndQuery);
        // The offset's + must arrive as a plus, not as a space.
        Assert.Equal(
            $"/api/v10/channels/{Channel}/threads/archived/public?before=2024-05-01T10%3A00%3A00.000000%2B00%3A00",
            api.Requests[1].PathAndQuery);
        Assert.Equal("2024-05-01T10:00:00.000000+00:00", api.Requests[1].Query("before"));
    }

    [Fact]
    public async Task PinsComeWithWhenTheyWerePinned()
    {
        var api = new DiscordFakeApi(
            DiscordFakeApi.Json($$"""
                { "has_more": true, "items": [
                  { "pinned_at": "2024-02-02T00:00:00+00:00", "message": {{Message("9")}} },
                  { "pinned_at": "2024-02-01T00:00:00+00:00", "message": {{Message("8")}} } ] }
                """),
            DiscordFakeApi.Json($$"""{ "has_more": false, "items": [ { "pinned_at": "2024-01-01T00:00:00+00:00", "message": {{Message("3")}} } ] }"""));

        var pins = await Collect(DiscordClientTests.Bot(api).ReadPinsAsync(Channel));

        Assert.Equal(["9", "8", "3"], pins.Select(pin => pin.Message.Id));
        Assert.Equal(new DateTimeOffset(2024, 2, 2, 0, 0, 0, TimeSpan.Zero), pins[0].PinnedAt);
        Assert.Equal($"/api/v10/channels/{Channel}/messages/pins?limit=50", api.Requests[0].PathAndQuery);
        Assert.Equal("2024-02-01T00:00:00+00:00", api.Requests[1].Query("before"));
    }

    [Fact]
    public async Task AFullPageThatDoesNotMoveTheCursorEndsTheRead()
    {
        // A server that answers every page with the same hundred messages.
        var api = new DiscordFakeApi(_ => DiscordFakeApi.Json(Array(Enumerable.Range(1, 100).Select(id => Message(id.ToString(System.Globalization.CultureInfo.InvariantCulture))))));

        var messages = await Collect(DiscordClientTests.Bot(api).ReadMessagesAsync(Channel));

        Assert.Equal(200, messages.Count);
        Assert.Equal(2, api.Requests.Count);
    }

    [Fact]
    public async Task TheServersTheBotIsInComeWithTheirSize()
    {
        var api = new DiscordFakeApi(DiscordFakeApi.Json($$"""
            [{ "id": "{{Guild}}", "name": "1337 Krew", "icon": "8342729096ea3675442027381ff50dfe", "approximate_member_count": 3268 }]
            """));

        var servers = await DiscordClientTests.Bot(api).ListServersAsync(CancellationToken.None);

        Assert.Equal(
            new DiscordServerSummary(Guild, "1337 Krew",
                $"https://cdn.discordapp.com/icons/{Guild}/8342729096ea3675442027381ff50dfe.png", 3268),
            Assert.Single(servers));
        Assert.Equal("/api/v10/users/@me/guilds?limit=200&with_counts=true", Assert.Single(api.Requests).PathAndQuery);
    }

    [Theory]
    [InlineData("../users/@me")]
    [InlineData("12a")]
    [InlineData("")]
    [InlineData("123456789012345678901")]
    public async Task AnIdThatIsNotASnowflakeIsRefusedBeforeAnyRequest(string id)
    {
        var api = new DiscordFakeApi(_ => throw new InvalidOperationException("No request expected."));
        var bot = DiscordClientTests.Bot(api);

        await Assert.ThrowsAsync<ArgumentException>(() => bot.GetServerAsync(id, CancellationToken.None));
        await Assert.ThrowsAsync<ArgumentException>(() => Collect(bot.ReadMessagesAsync(id)));
        await Assert.ThrowsAsync<ArgumentException>(() => Collect(bot.ReadMessagesAsync(Channel, before: id)));
        Assert.Empty(api.Requests);
    }

    [Fact]
    public async Task AWholeServerComesTogetherAndSaysWhatItCouldNotRead()
    {
        const string text = "100";
        const string forum = "101";
        const string voice = "102";
        var api = new DiscordFakeApi(request => request.Uri.AbsolutePath.Replace("/api/v10/", "", StringComparison.Ordinal) switch
        {
            $"guilds/{Guild}" => DiscordFakeApi.Json($$"""
                { "id": "{{Guild}}", "name": "Krew", "description": "Hi", "icon": "abc", "approximate_member_count": 2 }
                """),
            $"guilds/{Guild}/channels" => DiscordFakeApi.Json($$"""
                [ { "id": "99", "type": 4, "name": "Talk", "position": 0 },
                  { "id": "{{text}}", "type": 0, "name": "general", "parent_id": "99", "topic": "Hello", "rate_limit_per_user": 5,
                    "permission_overwrites": [{ "id": "{{Guild}}", "type": 0, "allow": "0", "deny": "2048" }] },
                  { "id": "{{forum}}", "type": 15, "name": "help", "parent_id": "99" },
                  { "id": "{{voice}}", "type": 2, "name": "Lounge", "user_limit": 5 } ]
                """),
            $"guilds/{Guild}/roles" => DiscordFakeApi.Json($$"""
                [ { "id": "{{Guild}}", "name": "@everyone", "position": 0, "permissions": "104324673" },
                  { "id": "5", "name": "Mods", "position": 1, "permissions": "8", "colors": { "primary_color": 3447003 }, "mentionable": true } ]
                """),
            $"guilds/{Guild}/emojis" => DiscordFakeApi.Json("""[{ "id": "41771983429993937", "name": "LUL", "animated": true }]"""),
            $"guilds/{Guild}/stickers" => DiscordFakeApi.Json("""[{ "id": "6", "name": "Wave", "format_type": 1 }]"""),
            $"guilds/{Guild}/members" => DiscordFakeApi.Json($"[{Member("7")}, {Member("8")}]"),
            $"guilds/{Guild}/threads/active" => DiscordFakeApi.Json($$"""
                { "threads": [ { "id": "30", "type": 11, "parent_id": "{{forum}}", "name": "Question" } ], "members": [] }
                """),
            $"channels/{text}/threads/archived/public" => DiscordFakeApi.Json($$"""
                { "has_more": false, "threads": [ {{Thread("31", "2024-01-01T00:00:00+00:00", text)}} ] }
                """),
            $"channels/{text}/threads/archived/private" => DiscordFakeApi.Error(HttpStatusCode.Forbidden, 50013, "Missing Permissions"),
            // The active thread shows up again among the archived ones; it is one thread.
            $"channels/{forum}/threads/archived/public" => DiscordFakeApi.Json($$"""
                { "has_more": false, "threads": [ { "id": "30", "type": 11, "parent_id": "{{forum}}", "name": "Question" } ] }
                """),
            var path => throw new InvalidOperationException($"Not expected: {path}"),
        });

        var snapshot = await DiscordClientTests.Bot(api).ReadServerAsync(Guild, CancellationToken.None);

        Assert.Equal(SnapshotSource.Bot, snapshot.Source);
        Assert.Equal((Guild, "Krew", "Hi"), (snapshot.SourceId, snapshot.Name, snapshot.Description));
        Assert.Equal($"https://cdn.discordapp.com/icons/{Guild}/abc.png", snapshot.IconUrl);
        Assert.Equal("Talk", Assert.Single(snapshot.Sections).Name);
        Assert.Equal(
            [(text, ChannelKind.Text, DiscordChannelType.Conversation), (forum, ChannelKind.Text, DiscordChannelType.Forum), (voice, ChannelKind.Voice, DiscordChannelType.Conversation)],
            snapshot.Channels.Select(channel => (channel.Id, channel.Kind, channel.DiscordType)));
        Assert.Equal(5, snapshot.Channels[0].SlowModeSeconds);
        Assert.Equal(new PermissionOverwriteSnapshot(OverwriteTarget.Role, Guild, "0", "2048"), Assert.Single(snapshot.Channels[0].Overwrites));
        Assert.Equal([true, false], snapshot.Roles.Select(role => role.IsEveryone));
        Assert.Equal(3447003, snapshot.Roles[1].Color);
        Assert.Equal("LUL", Assert.Single(snapshot.Emoji).Name);
        Assert.Equal("Wave", Assert.Single(snapshot.Stickers).Name);
        Assert.Equal(["7", "8"], snapshot.Members.Select(member => member.Id));
        Assert.Equal(["30", "31"], snapshot.Threads.Select(thread => thread.Id).Order());
        Assert.Equal(new SnapshotGap("private archived threads", text, DiscordFailureKind.MissingAccess, 50013), Assert.Single(snapshot.Gaps));
        // Voice channels have no threads, and a forum has no private ones.
        Assert.DoesNotContain(api.Requests, request => request.PathAndQuery.Contains($"channels/{voice}/", StringComparison.Ordinal));
        Assert.DoesNotContain(api.Requests, request => request.PathAndQuery.Contains($"channels/{forum}/threads/archived/private", StringComparison.Ordinal));
    }

    [Fact]
    public async Task AWholeServerWithoutTheMembersIntentStillReads()
    {
        var api = new DiscordFakeApi(request => request.Uri.AbsolutePath.Replace("/api/v10/", "", StringComparison.Ordinal) switch
        {
            $"guilds/{Guild}" => DiscordFakeApi.Json($$"""{ "id": "{{Guild}}", "name": "Krew" }"""),
            $"guilds/{Guild}/members" => DiscordFakeApi.Error(HttpStatusCode.Forbidden, 50001, "Missing Access"),
            "applications/@me" => DiscordFakeApi.Json("""{ "id": "1", "name": "Importer", "flags": 0 }"""),
            $"guilds/{Guild}/threads/active" => DiscordFakeApi.Json("""{ "threads": [] }"""),
            _ => DiscordFakeApi.Json("[]"),
        });

        var snapshot = await DiscordClientTests.Bot(api).ReadServerAsync(Guild, CancellationToken.None);

        Assert.Empty(snapshot.Members);
        Assert.Equal(new SnapshotGap("members", null, DiscordFailureKind.MissingIntent, 50001), Assert.Single(snapshot.Gaps));
    }

    [Fact]
    public async Task ABadTokenFailsTheWholeRead()
    {
        var api = new DiscordFakeApi(_ => DiscordFakeApi.Error(HttpStatusCode.Unauthorized, 0, "401: Unauthorized"));

        var failure = await Assert.ThrowsAsync<DiscordApiException>(
            () => DiscordClientTests.Bot(api).ReadServerAsync(Guild, CancellationToken.None));

        Assert.Equal(DiscordFailureKind.InvalidToken, failure.Kind);
        Assert.Single(api.Requests);
    }

    private static async Task<List<T>> Collect<T>(IAsyncEnumerable<T> items)
    {
        var list = new List<T>();
        await foreach (var item in items)
        {
            list.Add(item);
        }

        return list;
    }

    private static string Array(IEnumerable<string> items) => "[" + string.Join(",", items) + "]";

    private static string Message(string id) => JsonSerializer.Serialize(new
    {
        id,
        channel_id = Channel,
        type = 0,
        content = $"message {id}",
        timestamp = "2024-01-01T00:00:00+00:00",
        author = new { id = "1", username = "jo" },
    });

    private static string Member(string id) => JsonSerializer.Serialize(new
    {
        user = new { id, username = $"user{id}" },
        roles = System.Array.Empty<string>(),
        joined_at = "2024-01-01T00:00:00+00:00",
    });

    private static string User(string id) => JsonSerializer.Serialize(new { id, username = $"user{id}" });

    private static string Thread(string id, string archivedAt, string parent = Channel) => JsonSerializer.Serialize(new
    {
        id,
        type = 11,
        parent_id = parent,
        name = $"thread {id}",
        thread_metadata = new { archived = true, archive_timestamp = archivedAt },
    });
}
