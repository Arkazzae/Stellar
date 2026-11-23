using System.Net;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Integrations.Discord;

namespace Stellarr.Api.Tests;

/// <summary>
/// The Discord reader's requests: where they go, how they wait out rate
/// limits on a fake clock, and what each failure becomes.
/// </summary>
public sealed class DiscordClientTests
{
    private const string Token = "sample-bot-token.not-a-real-one";
    private const string Me = """{"id":"198622483471925248","username":"importer","bot":true}""";

    [Fact]
    public async Task TheTestClockFiresAMinuteLongWaitAtOnce()
    {
        var clock = new DiscordTestClock();
        var before = clock.GetUtcNow();

        await Task.Delay(TimeSpan.FromSeconds(60), clock).WaitAsync(TimeSpan.FromSeconds(5));

        Assert.Equal([TimeSpan.FromSeconds(60)], clock.Waits);
        Assert.Equal(before + TimeSpan.FromSeconds(60), clock.GetUtcNow());
    }

    [Fact]
    public void TheApplicationHandsOutAReader()
    {
        using var factory = new StellarrWebApplicationFactory(loadTestData: false);

        Assert.NotNull(factory.Services.GetRequiredService<DiscordClient>());
    }

    [Fact]
    public async Task ATemplateIsReadWithoutATokenFromAnAddressBuiltFromItsCode()
    {
        var api = new DiscordFakeApi(DiscordFakeApi.Json(DiscordSnapshotTests.TemplateFixture()));
        Assert.True(DiscordTemplateLink.TryParse("https://discord.new/hgM48av5Q69A", out var code, out _));

        var snapshot = await Client(api).ReadTemplateAsync(code, CancellationToken.None);

        var request = Assert.Single(api.Requests);
        Assert.Equal("https://discord.com/api/v10/guilds/templates/hgM48av5Q69A", request.Uri.ToString());
        Assert.Null(request.Authorization);
        Assert.StartsWith("DiscordBot (", request.UserAgent, StringComparison.Ordinal);
        Assert.Equal("Friends & Family", snapshot.Name);
        Assert.Equal(5, snapshot.Channels.Count);
    }

    [Fact]
    public async Task AnUnknownTemplateSaysSo()
    {
        var api = new DiscordFakeApi(DiscordFakeApi.Error(HttpStatusCode.NotFound, 10057, "Unknown Guild Template"));
        Assert.True(DiscordTemplateLink.TryParse("abcdef", out var code, out _));

        var failure = await Assert.ThrowsAsync<DiscordApiException>(
            () => Client(api).ReadTemplateAsync(code, CancellationToken.None));

        Assert.Equal(DiscordFailureKind.UnknownResource, failure.Kind);
        Assert.Equal(10057, failure.DiscordCode);
        Assert.Equal(404, failure.StatusCode);
        Assert.Equal("GET guilds/templates/abcdef", failure.Route);
    }

    [Fact]
    public async Task ABotRequestCarriesTheTokenInItsHeaderOnly()
    {
        var api = new DiscordFakeApi(DiscordFakeApi.Json(Me));

        var bot = await Client(api).ForBot(new DiscordBotToken(Token)).GetCurrentUserAsync(CancellationToken.None);

        var request = Assert.Single(api.Requests);
        Assert.Equal("https://discord.com/api/v10/users/@me", request.Uri.ToString());
        Assert.Equal("Bot " + Token, request.Authorization);
        Assert.DoesNotContain(Token, request.Uri.ToString(), StringComparison.Ordinal);
        Assert.Equal(new DiscordBotIdentity("198622483471925248", "importer", true), bot);
    }

    [Theory]
    [InlineData("Bot " + Token)]
    [InlineData("  " + Token + "\n")]
    public void APastedTokenIsForgivenItsPrefixAndSpaces(string pasted) =>
        Assert.Equal(Token, new DiscordBotToken(pasted).Value);

    [Theory]
    [InlineData("")]
    [InlineData("Bot ")]
    [InlineData("two words")]
    public void ATokenIsOneWord(string pasted) =>
        Assert.Throws<ArgumentException>(() => new DiscordBotToken(pasted));

    [Theory]
    [InlineData(1L << 15 | 1L << 19, true, true)]
    [InlineData(1L << 14 | 1L << 18, true, true)]
    [InlineData(1L << 15, true, false)]
    [InlineData(1L << 19, false, true)]
    [InlineData(1L << 12 | 1L << 13, false, false)]
    public async Task TheAppsFlagsSayWhichPrivilegedIntentsAreOn(long flags, bool members, bool content)
    {
        var api = new DiscordFakeApi(DiscordFakeApi.Json($$"""{"id":"1","name":"Importer","flags":{{flags}}}"""));

        var application = await Bot(api).GetCurrentApplicationAsync(CancellationToken.None);

        Assert.Equal("https://discord.com/api/v10/applications/@me", Assert.Single(api.Requests).Uri.ToString());
        Assert.Equal((members, content), (application.ServerMembersIntent, application.MessageContentIntent));
    }

    /* ------------------------------------------------------------ rate limits */

    [Fact]
    public async Task AnExhaustedBucketIsWaitedOutBeforeTheNextRequest()
    {
        var clock = new DiscordTestClock();
        var api = new DiscordFakeApi(
            DiscordFakeApi.Json(Me, HttpStatusCode.OK,
                ("X-RateLimit-Limit", "5"), ("X-RateLimit-Remaining", "0"),
                ("X-RateLimit-Reset", "1470173023"), ("X-RateLimit-Reset-After", "2.5"),
                ("X-RateLimit-Bucket", "abcd1234")),
            DiscordFakeApi.Json(Me, HttpStatusCode.OK, ("X-RateLimit-Remaining", "4"), ("X-RateLimit-Bucket", "abcd1234")),
            DiscordFakeApi.Json(Me));
        var bot = Bot(api, clock);

        await bot.GetCurrentUserAsync(CancellationToken.None);
        Assert.Empty(clock.Waits);
        await bot.GetCurrentUserAsync(CancellationToken.None);
        await bot.GetCurrentUserAsync(CancellationToken.None);

        // Reset-After, a duration, is used and not the epoch in Reset.
        Assert.Equal([TimeSpan.FromSeconds(2.5)], clock.Waits);
        Assert.Equal(3, api.Requests.Count);
    }

    [Fact]
    public async Task A429IsWaitedOutAndTheRequestSentAgain()
    {
        var clock = new DiscordTestClock();
        var api = new DiscordFakeApi(DiscordFakeApi.TooMany(1.5), DiscordFakeApi.Json(Me));

        var bot = await Bot(api, clock).GetCurrentUserAsync(CancellationToken.None);

        Assert.Equal("importer", bot.Username);
        Assert.Equal([TimeSpan.FromSeconds(1.5)], clock.Waits);
        Assert.Equal(2, api.Requests.Count);
    }

    [Fact]
    public async Task A429WithoutJsonFallsBackToRetryAfter()
    {
        var clock = new DiscordTestClock();
        var limited = new HttpResponseMessage(HttpStatusCode.TooManyRequests)
        {
            Content = new StringContent("<html>Slow down</html>", System.Text.Encoding.UTF8, "text/html"),
        };
        limited.Headers.TryAddWithoutValidation("Retry-After", "4");
        var api = new DiscordFakeApi(limited, DiscordFakeApi.Json(Me));

        await Bot(api, clock).GetCurrentUserAsync(CancellationToken.None);

        Assert.Equal([TimeSpan.FromSeconds(4)], clock.Waits);
    }

    [Fact]
    public async Task AGlobalLimitHoldsBackEveryRoute()
    {
        var clock = new DiscordTestClock();
        var api = new DiscordFakeApi(request => request.Uri.AbsolutePath.EndsWith("/users/@me", StringComparison.Ordinal)
            ? DiscordFakeApi.TooMany(3, global: true, ("X-RateLimit-Global", "true"), ("X-RateLimit-Scope", "global"))
            : DiscordFakeApi.Json("""{"id":"1","name":"Importer","flags":0}"""));
        var bot = Bot(api, clock, new DiscordRequestPolicy { MaxRateLimitRetries = 0 });

        var failure = await Assert.ThrowsAsync<DiscordApiException>(() => bot.GetCurrentUserAsync(CancellationToken.None));
        Assert.Equal(DiscordFailureKind.RateLimited, failure.Kind);
        Assert.Empty(clock.Waits);

        // Another route, never limited itself, still waits for the global limit.
        await bot.GetCurrentApplicationAsync(CancellationToken.None);
        Assert.Equal([TimeSpan.FromSeconds(3)], clock.Waits);
    }

    [Fact]
    public async Task AWaitLongerThanTheBoundFailsAtOnceAndSaysHowLong()
    {
        var clock = new DiscordTestClock();
        // The documentation's example of a shared resource's limit.
        var api = new DiscordFakeApi(
            DiscordFakeApi.TooMany(1336.57, headers: [("Retry-After", "1337"), ("X-RateLimit-Scope", "shared")]));

        var failure = await Assert.ThrowsAsync<DiscordApiException>(
            () => Bot(api, clock).GetCurrentUserAsync(CancellationToken.None));

        Assert.Equal(DiscordFailureKind.RateLimited, failure.Kind);
        Assert.Equal(TimeSpan.FromSeconds(1336.57), failure.RetryAfter);
        Assert.Empty(clock.Waits);
        Assert.Single(api.Requests);
    }

    [Fact]
    public async Task RepeatedLimitsGiveUpAfterTheBound()
    {
        var clock = new DiscordTestClock();
        var api = new DiscordFakeApi(_ => DiscordFakeApi.TooMany(2));

        var failure = await Assert.ThrowsAsync<DiscordApiException>(
            () => Bot(api, clock, new DiscordRequestPolicy { MaxRateLimitRetries = 3 }).GetCurrentUserAsync(CancellationToken.None));

        Assert.Equal(DiscordFailureKind.RateLimited, failure.Kind);
        Assert.Equal(4, api.Requests.Count);
        Assert.Equal(Enumerable.Repeat(TimeSpan.FromSeconds(2), 3), clock.Waits);
    }

    [Fact]
    public async Task CancellingDuringAWaitStopsTheRequest()
    {
        var clock = new DiscordTestClock { Hold = true };
        var api = new DiscordFakeApi(DiscordFakeApi.TooMany(30), DiscordFakeApi.Json(Me));
        using var cancellation = new CancellationTokenSource();

        var pending = Bot(api, clock).GetCurrentUserAsync(cancellation.Token);
        while (clock.Waits.Count == 0 && !pending.IsCompleted)
        {
            await Task.Yield();
        }

        await cancellation.CancelAsync();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => pending.WaitAsync(TimeSpan.FromSeconds(5)));
        Assert.Single(api.Requests);
    }

    /* ---------------------------------------------------------------- failures */

    public static TheoryData<HttpStatusCode, int?, DiscordFailureKind> Refusals() => new()
    {
        { HttpStatusCode.Unauthorized, 0, DiscordFailureKind.InvalidToken },
        { HttpStatusCode.Unauthorized, 50014, DiscordFailureKind.InvalidToken },
        { HttpStatusCode.Forbidden, 50001, DiscordFailureKind.MissingAccess },
        { HttpStatusCode.Forbidden, 50013, DiscordFailureKind.MissingAccess },
        { HttpStatusCode.NotFound, 10003, DiscordFailureKind.UnknownResource },
        { HttpStatusCode.NotFound, 10004, DiscordFailureKind.UnknownResource },
        { HttpStatusCode.BadRequest, 50035, DiscordFailureKind.UnexpectedResponse },
        { HttpStatusCode.MethodNotAllowed, null, DiscordFailureKind.UnexpectedResponse },
        { HttpStatusCode.Found, null, DiscordFailureKind.UnexpectedResponse },
    };

    [Theory]
    [MemberData(nameof(Refusals))]
    public async Task EachRefusalBecomesAKindTheImportCanActOn(HttpStatusCode status, int? code, DiscordFailureKind expected)
    {
        var answer = code is { } number
            ? DiscordFakeApi.Error(status, number, "No")
            : new HttpResponseMessage(status);
        var api = new DiscordFakeApi(answer);

        var failure = await Assert.ThrowsAsync<DiscordApiException>(
            () => Bot(api).GetCurrentUserAsync(CancellationToken.None));

        Assert.Equal(expected, failure.Kind);
        Assert.Equal((int)status, failure.StatusCode);
        Assert.Equal(code, failure.DiscordCode);
        Assert.Single(api.Requests);
    }

    [Fact]
    public async Task ServerErrorsAreTriedTwiceMoreThenReported()
    {
        var clock = new DiscordTestClock();
        var api = new DiscordFakeApi(_ => new HttpResponseMessage(HttpStatusCode.BadGateway));

        var failure = await Assert.ThrowsAsync<DiscordApiException>(
            () => Bot(api, clock).GetCurrentUserAsync(CancellationToken.None));

        Assert.Equal(DiscordFailureKind.Unreachable, failure.Kind);
        Assert.Equal(502, failure.StatusCode);
        Assert.Equal(3, api.Requests.Count);
        Assert.Equal([TimeSpan.FromSeconds(1), TimeSpan.FromSeconds(2)], clock.Waits);
    }

    [Fact]
    public async Task AServerErrorThatPassesIsNotAFailure()
    {
        var api = new DiscordFakeApi(new HttpResponseMessage(HttpStatusCode.ServiceUnavailable), DiscordFakeApi.Json(Me));

        Assert.Equal("importer", (await Bot(api).GetCurrentUserAsync(CancellationToken.None)).Username);
    }

    [Fact]
    public async Task TransportFailuresAndTimeoutsAreUnreachable()
    {
        var refused = new DiscordFakeApi(_ => throw new HttpRequestException(HttpRequestError.ConnectionError, "Connection refused"));
        var slow = new DiscordFakeApi(_ => throw new TaskCanceledException("The request was canceled due to the configured HttpClient.Timeout"));

        var connection = await Assert.ThrowsAsync<DiscordApiException>(() => Bot(refused).GetCurrentUserAsync(CancellationToken.None));
        var timeout = await Assert.ThrowsAsync<DiscordApiException>(() => Bot(slow).GetCurrentUserAsync(CancellationToken.None));

        Assert.Equal(DiscordFailureKind.Unreachable, connection.Kind);
        Assert.IsType<HttpRequestException>(connection.InnerException);
        Assert.Equal(DiscordFailureKind.Unreachable, timeout.Kind);
    }

    [Fact]
    public async Task AnAnswerThatStallsHalfwayTimesOut()
    {
        var api = new DiscordFakeApi(_ => new HttpResponseMessage(HttpStatusCode.OK) { Content = new StalledContent() });
        var client = new DiscordClient(
            new HttpClient(api) { Timeout = TimeSpan.FromMilliseconds(200) }, new DiscordTestClock(), new DiscordCapturingLogger());

        var failure = await Assert.ThrowsAsync<DiscordApiException>(
            () => client.ForBot(new DiscordBotToken(Token)).GetCurrentUserAsync(CancellationToken.None).WaitAsync(TimeSpan.FromSeconds(10)));

        Assert.Equal(DiscordFailureKind.Unreachable, failure.Kind);
    }

    [Theory]
    [InlineData("""{"id": [1, 2], "username": {"a": 1}}""", "application/json", false)]
    [InlineData("not json", "application/json", true)]
    [InlineData("<html></html>", "text/html", true)]
    [InlineData("null", "application/json", true)]
    public async Task AnAnswerOfAnotherShapeIsUnexpected(string body, string contentType, bool fails)
    {
        var api = new DiscordFakeApi(new HttpResponseMessage(HttpStatusCode.OK)
        {
            Content = new StringContent(body, System.Text.Encoding.UTF8, contentType),
        });

        var read = Bot(api).GetCurrentUserAsync(CancellationToken.None);

        if (fails)
        {
            Assert.Equal(DiscordFailureKind.UnexpectedResponse, (await Assert.ThrowsAsync<DiscordApiException>(() => read)).Kind);
        }
        else
        {
            // Odd values in fields read as absent rather than failing the answer.
            Assert.Equal(new DiscordBotIdentity("", "", false), await read);
        }
    }

    /* ------------------------------------------------------------------ token */

    [Fact]
    public async Task TheTokenAppearsInNoMessageNoLogAndNoPrintedForm()
    {
        var logger = new DiscordCapturingLogger();
        var clock = new DiscordTestClock();
        var api = new DiscordFakeApi(
            DiscordFakeApi.TooMany(1),
            DiscordFakeApi.Error(HttpStatusCode.Unauthorized, 0, $"401: Unauthorized, Bot {Token} is not valid"));
        var token = new DiscordBotToken(Token);
        var client = new DiscordClient(new HttpClient(api), clock, logger);

        var failure = await Assert.ThrowsAsync<DiscordApiException>(
            () => client.ForBot(token).GetCurrentUserAsync(CancellationToken.None));

        Assert.Equal(DiscordFailureKind.InvalidToken, failure.Kind);
        Assert.Contains("[token]", failure.Message, StringComparison.Ordinal);
        Assert.DoesNotContain(Token, failure.ToString(), StringComparison.Ordinal);
        Assert.NotEmpty(logger.Lines);
        Assert.All(logger.Lines, line => Assert.DoesNotContain(Token, line, StringComparison.Ordinal));
        Assert.DoesNotContain(Token, token.ToString(), StringComparison.Ordinal);
        Assert.DoesNotContain(Token, client.ForBot(token).ToString() ?? "", StringComparison.Ordinal);
    }

    [Fact]
    public async Task TheTokenIsNotInATransportFailure()
    {
        var api = new DiscordFakeApi(_ => throw new HttpRequestException("Name or service not known (discord.com:443)"));

        var failure = await Assert.ThrowsAsync<DiscordApiException>(
            () => Bot(api).GetCurrentUserAsync(CancellationToken.None));

        Assert.DoesNotContain(Token, failure.ToString(), StringComparison.Ordinal);
    }

    /// <summary>A body that sends its first bytes and then nothing, until it is given up on.</summary>
    private sealed class StalledContent : HttpContent
    {
        protected override Task SerializeToStreamAsync(Stream stream, System.Net.TransportContext? context) =>
            SerializeToStreamAsync(stream, context, CancellationToken.None);

        protected override async Task SerializeToStreamAsync(
            Stream stream, System.Net.TransportContext? context, CancellationToken cancellationToken)
        {
            await stream.WriteAsync("""{"id":"""u8.ToArray(), cancellationToken);
            await Task.Delay(Timeout.Infinite, cancellationToken);
        }

        protected override bool TryComputeLength(out long length)
        {
            length = 0;
            return false;
        }
    }

    internal static DiscordClient Client(DiscordFakeApi api, DiscordTestClock? clock = null, DiscordRequestPolicy? policy = null) =>
        new(new HttpClient(api), clock ?? new DiscordTestClock(), new DiscordCapturingLogger())
        {
            Policy = policy ?? DiscordRequestPolicy.Default,
        };

    internal static DiscordBotReader Bot(DiscordFakeApi api, DiscordTestClock? clock = null, DiscordRequestPolicy? policy = null) =>
        Client(api, clock, policy).ForBot(new DiscordBotToken(Token));
}
