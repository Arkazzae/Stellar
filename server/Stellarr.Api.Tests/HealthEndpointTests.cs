using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Http;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Microsoft.Extensions.Logging;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Logging;
using Stellarr.Api.Storage;

namespace Stellarr.Api.Tests;

public sealed partial class HealthEndpointTests(StellarrWebApplicationFactory factory)
    : IClassFixture<StellarrWebApplicationFactory>
{
    [LoggerMessage(EventId = 1, Level = LogLevel.Information, Message = "{Marker}")]
    private static partial void LogInformation(ILogger logger, string marker);

    [LoggerMessage(EventId = 2, Level = LogLevel.Warning, Message = "{Marker}")]
    private static partial void LogWarning(ILogger logger, string marker);

    [Fact]
    public async Task AProbeLeavesNoLogLinesWhileOtherRequestsKeepTheirs()
    {
        using var client = factory.CreateClient();
        var buffer = factory.Services.GetRequiredService<CockpitLogBuffer>();
        var newest = buffer.Snapshot(1);
        var start = newest.Count > 0 ? newest[0].Sequence : -1;
        var other = $"/api/no-such-route-{Guid.NewGuid():N}";

        Assert.Equal(HttpStatusCode.OK, (await client.GetAsync("/health/ready")).StatusCode);
        Assert.Equal(HttpStatusCode.OK, (await client.GetAsync("/health/live")).StatusCode);
        _ = await client.GetAsync(other);

        // A request is logged once its response is out; wait for the last
        // request's line, which comes after the probes'.
        IReadOnlyList<CockpitLogRecord> lines = [];
        for (var attempt = 0; attempt < 50; attempt++)
        {
            lines = buffer.Snapshot(CockpitLogBuffer.Capacity).Where(line => line.Sequence > start).ToList();
            if (lines.Any(line => line.Message.Contains(other, StringComparison.Ordinal)))
            {
                break;
            }

            await Task.Delay(20);
        }

        Assert.Contains(lines, line => line.Message.StartsWith($"GET {other} 404", StringComparison.Ordinal));
        Assert.DoesNotContain(lines, line => line.Message.Contains("/health/", StringComparison.Ordinal));
    }

    [Fact]
    public void DuringAProbeOnlyWarningsAndErrorsAreLogged()
    {
        var context = new DefaultHttpContext();
        context.Request.Path = "/health/ready";
        var accessor = new HttpContextAccessor { HttpContext = context };
        var buffer = new CockpitLogBuffer(accessor);
        using var inner = new LoggerFactory([buffer]);
        using var quiet = new HealthProbeLogging.QuietLoggerFactory(inner, accessor);
        var logger = quiet.CreateLogger("HealthEndpointTests");

        LogInformation(logger, "probe information");
        LogWarning(logger, "probe warning");
        context.Request.Path = "/health";
        LogInformation(logger, "another path");
        accessor.HttpContext = null;
        LogInformation(logger, "no request");

        Assert.Equal(
            ["no request", "another path", "probe warning"],
            buffer.Snapshot(10).Select(line => line.Message));
    }

    [Theory]
    [InlineData("/health/live", "live")]
    [InlineData("/health/ready", "ready")]
    public async Task ReportsItsStatus(string path, string expectedStatus)
    {
        using var client = factory.CreateClient();

        var response = await client.GetAsync(path);

        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        var body = await response.Content.ReadFromJsonAsync<JsonElement>();
        Assert.Equal(expectedStatus, body.GetProperty("status").GetString());
    }

    [Fact]
    public async Task StartsWithTheSeededFixtureData()
    {
        using var client = factory.CreateClient();
        _ = await client.GetAsync("/health/ready");

        await using var scope = factory.Services.CreateAsyncScope();
        var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();

        Assert.Equal(5, await db.Users.CountAsync());
        Assert.Equal(1, await db.Servers.CountAsync());
        Assert.Equal(3, await db.ServerRoles.CountAsync());
        Assert.Equal(5, await db.Channels.CountAsync());
        Assert.Equal(1, await db.Messages.CountAsync());
    }

    [Fact]
    public async Task WaitsForTheObjectStoreBeforeServing()
    {
        var storage = new StartingUpObjectStorage(failures: 2);
        using var starting = new StellarrWebApplicationFactory(
            loadTestData: false,
            configuration: new Dictionary<string, string?> { ["Storage:SkipInitialization"] = "false" },
            configureServices: services =>
            {
                services.RemoveAll<IObjectStorage>();
                services.AddSingleton<IObjectStorage>(storage);
            });
        using var client = starting.CreateClient();

        var response = await client.GetAsync("/health/ready");

        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        Assert.Equal(3, storage.Preparations);
    }

    /// <summary>An object store that cannot be prepared until it has finished starting.</summary>
    private sealed class StartingUpObjectStorage(int failures) : IObjectStorage
    {
        public int Preparations { get; private set; }

        public Task PrepareAsync(CancellationToken cancellationToken)
        {
            Preparations++;
            return Preparations <= failures
                ? Task.FromException(new HttpRequestException("connection refused"))
                : Task.CompletedTask;
        }

        public Task PutAsync(string key, Stream data, CancellationToken cancellationToken) =>
            throw new NotSupportedException();

        public Task<StoredObject> GetAsync(string key, ByteRange? range, CancellationToken cancellationToken) =>
            throw new NotSupportedException();

        public Task DeleteAsync(string key, CancellationToken cancellationToken) =>
            throw new NotSupportedException();

        public IAsyncEnumerable<StoredObjectInfo> ListAsync(CancellationToken cancellationToken) =>
            throw new NotSupportedException();

        public Task ProbeAsync(CancellationToken cancellationToken) => throw new NotSupportedException();
    }
}
