using System.Globalization;
using System.Net;
using System.Text.Json;
using Stellarr.Api.Realtime;

namespace Stellarr.Api.Tests;

public sealed partial class VoiceRoomTests
{
    [Fact]
    public async Task SubscriptionsForwardTheCompleteMidAllowlistIncludingEmptyAndUnknownMids()
    {
        var media = new FakeMediaServer();
        await using var app = new VoiceApp(media);
        await app.MediaReadyAsync();
        var maja = await app.ConnectAsync("maja");
        await maja.JoinAsync(Salon);
        var sessionId = await maja.OpenMediaAsync(Salon);
        var path = $"/v1/sessions/{sessionId}/subscriptions";

        await maja.MediaSignalAsync(Salon, sessionId, "subscriptions", """{"mids":[]}""");
        string[] mids = ["0", "screen-1", "!~", "abcdefghijklmnop", "A", "a"];
        await maja.MediaSignalAsync(Salon, sessionId, "subscriptions", JsonSerializer.Serialize(new { mids }));

        var requests = media.Requests.Where(request => request.Path == path).ToArray();
        Assert.Equal(2, requests.Length);
        Assert.All(requests, request => Assert.Equal("POST", request.Method));
        Assert.All(requests, request => Assert.Equal($"Bearer {FakeMediaServer.Secret}", request.Authorization));
        Assert.Empty(requests[0].Json.GetProperty("mids").EnumerateArray());
        Assert.Equal(mids, requests[1].Json.GetProperty("mids").EnumerateArray().Select(mid => mid.GetString()!));
    }

    [Fact]
    public async Task TheOrderOfTheListReachesTheMediaServerAsSentSinceItIsThePriority()
    {
        var media = new FakeMediaServer();
        await using var app = new VoiceApp(media);
        await app.MediaReadyAsync();
        var maja = await app.ConnectAsync("maja");
        await maja.JoinAsync(Salon);
        var sessionId = await maja.OpenMediaAsync(Salon);
        var path = $"/v1/sessions/{sessionId}/subscriptions";

        // The same sections, most important first, then in another order: a
        // new order alone is a new list.
        string[] voicesFirst = ["a1", "a2", "share", "share-sound", "cam-1", "cam-2"];
        string[] camerasFirst = ["cam-2", "cam-1", "a2", "a1", "share-sound", "share"];
        await maja.MediaSignalAsync(Salon, sessionId, "subscriptions", JsonSerializer.Serialize(new { mids = voicesFirst }));
        await maja.MediaSignalAsync(Salon, sessionId, "subscriptions", JsonSerializer.Serialize(new { mids = camerasFirst }));

        var requests = media.Requests.Where(request => request.Path == path).ToArray();
        Assert.Equal(2, requests.Length);
        Assert.Equal(voicesFirst, requests[0].Json.GetProperty("mids").EnumerateArray().Select(mid => mid.GetString()!));
        Assert.Equal(camerasFirst, requests[1].Json.GetProperty("mids").EnumerateArray().Select(mid => mid.GetString()!));
    }

    [Fact]
    public async Task MalformedSubscriptionAllowlistsNeverReachTheMediaServer()
    {
        var media = new FakeMediaServer();
        await using var app = new VoiceApp(media);
        await app.MediaReadyAsync();
        var maja = await app.ConnectAsync("maja");
        await maja.JoinAsync(Salon);
        var sessionId = await maja.OpenMediaAsync(Salon);

        foreach (var payload in new[]
        {
            "{not json",
            "null",
            "[]",
            "{}",
            """{"mids":null}""",
            """{"mids":"0"}""",
            """{"mids":{}}""",
            """{"mids":[null]}""",
            """{"mids":[0]}""",
            """{"mids":[true]}""",
            """{"mids":[{}]}""",
            """{"mids":[[]]}""",
            """{"mids":[""]}""",
            """{"mids":["abcdefghijklmnopq"]}""",
            """{"mids":["a b"]}""",
            """{"mids":["a\tb"]}""",
            """{"mids":["a\nb"]}""",
            """{"mids":["\u0000"]}""",
            """{"mids":["\u007f"]}""",
            """{"mids":["é"]}""",
            """{"mids":["0","0"]}""",
            """{"mids":["0","\u0030"]}""",
            """{"mids":[],"mids":["0"]}""",
            JsonSerializer.Serialize(new { mids = SubscriptionMids(4097) }),
        })
        {
            await RefusedAsync("Invalid media signal.", maja.MediaSignalAsync(Salon, sessionId, "subscriptions", payload));
        }

        Assert.Equal(0, media.Count("POST", $"/v1/sessions/{sessionId}/subscriptions"));
    }

    [Fact]
    public async Task SubscriptionCountAndEnvelopeLimitsAreInclusive()
    {
        var media = new FakeMediaServer();
        await using var app = new VoiceApp(media);
        await app.MediaReadyAsync();
        var maja = await app.ConnectAsync("maja");
        await maja.JoinAsync(Salon);
        var sessionId = await maja.OpenMediaAsync(Salon);
        var path = $"/v1/sessions/{sessionId}/subscriptions";
        var mids = SubscriptionMids(4096);

        await maja.MediaSignalAsync(Salon, sessionId, "subscriptions", JsonSerializer.Serialize(new { mids }));
        var largestList = Assert.Single(media.Requests, request => request.Path == path);
        Assert.Equal(mids, largestList.Json.GetProperty("mids").EnumerateArray().Select(mid => mid.GetString()!));

        const string empty = """{"mids":[]}""";
        var atLimit = empty.PadRight(RtcProtocol.MaxSignalBytes);
        await RefusedAsync(
            "Invalid media signal.",
            maja.MediaSignalAsync(Salon, sessionId, "subscriptions", atLimit + " "));
        Assert.Equal(1, media.Count("POST", path));
        await maja.MediaSignalAsync(Salon, sessionId, "subscriptions", atLimit);
        Assert.Equal(2, media.Count("POST", path));
    }

    [Fact]
    public async Task SubscriptionsRequireTheCallersCurrentSessionAndSeat()
    {
        var media = new FakeMediaServer();
        await using var app = new VoiceApp(media);
        await app.MediaReadyAsync();
        var maja = await app.ConnectAsync("maja");
        var majaElsewhere = await app.ConnectAsync("maja");
        var ada = await app.ConnectAsync("ada");
        await maja.JoinAsync(Salon);
        await ada.JoinAsync(Salon);
        var own = await maja.OpenMediaAsync(Salon);
        var others = await ada.OpenMediaAsync(Salon);
        const string payload = """{"mids":["0"]}""";
        const string refused = "Invalid media signal.";

        await RefusedAsync(refused, maja.MediaSignalAsync(Salon, own, "subscriptions", payload, version: 2));
        await RefusedAsync(refused, maja.MediaSignalAsync(NightShift, own, "subscriptions", payload));
        await RefusedAsync(refused, maja.MediaSignalAsync(Salon, others, "subscriptions", payload));
        await RefusedAsync(refused, majaElsewhere.MediaSignalAsync(Salon, own, "subscriptions", payload));
        await RefusedAsync(refused, maja.MediaSignalAsync(Salon, null, "subscriptions", payload));
        await RefusedAsync(refused, maja.MediaSignalAsync(Salon, "session-unknown", "subscriptions", payload));
        var replacement = await maja.OpenMediaAsync(Salon);
        await RefusedAsync(refused, maja.MediaSignalAsync(Salon, own, "subscriptions", payload));
        await maja.LeaveAsync();
        await RefusedAsync(refused, maja.MediaSignalAsync(Salon, replacement, "subscriptions", payload));

        Assert.DoesNotContain(media.Requests, request => request.Path.EndsWith("/subscriptions", StringComparison.Ordinal));
    }

    [Fact]
    public async Task SubscriptionFailuresUseTheExistingMediaSignalOutcomes()
    {
        var media = new FakeMediaServer();
        await using var app = new VoiceApp(media);
        await app.MediaReadyAsync();
        var maja = await app.ConnectAsync("maja");
        await maja.JoinAsync(Salon);
        var sessionId = await maja.OpenMediaAsync(Salon);
        const string payload = """{"mids":[]}""";

        media.AnswerSignals(sessionId, HttpStatusCode.UnprocessableEntity);
        await RefusedAsync("Invalid media signal.", maja.MediaSignalAsync(Salon, sessionId, "subscriptions", payload));
        media.AnswerSignals(sessionId, HttpStatusCode.InternalServerError);
        await RefusedAsync("Media server is unavailable.", maja.MediaSignalAsync(Salon, sessionId, "subscriptions", payload));
        Assert.Equal(sessionId, app.Presence.MediaSessionOf(maja.Connection.ConnectionId!, TestData.VoiceChannelId)?.SessionId);

        media.AnswerSignals(sessionId, HttpStatusCode.NotFound);
        media.DropSession(sessionId);
        await maja.MediaSignalAsync(Salon, sessionId, "subscriptions", payload);
        var closed = await maja["MediaSessionClosed"].NextAsync();
        Assert.Equal(sessionId, closed.GetProperty("sessionId").GetString());
        Assert.Equal("media-restarted", closed.GetProperty("reason").GetString());
        Assert.Null(app.Presence.MediaSessionOf(maja.Connection.ConnectionId!, TestData.VoiceChannelId));
    }

    private static string[] SubscriptionMids(int count) =>
        [.. Enumerable.Range(0, count).Select(index => index.ToString("D16", CultureInfo.InvariantCulture))];
}
