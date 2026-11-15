using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Storage;

namespace Stellarr.Api.Tests;

public sealed class HealthEndpointTests(StellarrWebApplicationFactory factory)
    : IClassFixture<StellarrWebApplicationFactory>
{
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
