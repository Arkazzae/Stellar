using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.FileProviders;
using Microsoft.Extensions.Hosting;
using Stellarr.Api.Realtime;

namespace Stellarr.Api.Tests;

public sealed class DevelopmentPresenceSeedTests
{
    [Fact]
    public void ASeededDocumentRestoresPresenceInDevelopment()
    {
        using var folder = new TemporaryFolder();
        var userId = Guid.NewGuid();
        folder.Write(
            DevelopmentPresenceSeed.FileName,
            $$"""
            {
              "version": 1,
              "users": [
                { "id": "{{userId}}", "presence": "idle" },
                { "id": "not-a-guid", "presence": "dnd" },
                { "id": "{{Guid.NewGuid()}}", "presence": "offline" }
              ]
            }
            """);

        var tracker = ApplyTo(folder.Path, Environments.Development);

        Assert.Equal("idle", tracker.PresenceOf(userId));
    }

    [Fact]
    public void ProductionIgnoresASeededDocument()
    {
        using var folder = new TemporaryFolder();
        var userId = Guid.NewGuid();
        folder.Write(
            DevelopmentPresenceSeed.FileName,
            $$"""{"users":[{"id":"{{userId}}","presence":"idle"}]}""");

        var tracker = ApplyTo(folder.Path, Environments.Production);

        Assert.Equal("offline", tracker.PresenceOf(userId));
    }

    [Fact]
    public void AMalformedDocumentLeavesEveryoneOffline()
    {
        using var folder = new TemporaryFolder();
        folder.Write(DevelopmentPresenceSeed.FileName, "{ this is not json");

        var tracker = ApplyTo(folder.Path, Environments.Development);

        Assert.Equal("offline", tracker.PresenceOf(Guid.NewGuid()));
    }

    private static PresenceTracker ApplyTo(string dataRoot, string environmentName)
    {
        var tracker = new PresenceTracker();
        var configuration = new ConfigurationBuilder()
            .AddInMemoryCollection(new Dictionary<string, string?> { ["DataRoot"] = dataRoot })
            .Build();
        using var services = new ServiceCollection()
            .AddSingleton<IHostEnvironment>(new TestEnvironment(environmentName))
            .AddSingleton<IConfiguration>(configuration)
            .AddLogging()
            .AddSingleton(tracker)
            .BuildServiceProvider();

        DevelopmentPresenceSeed.Apply(services);
        return tracker;
    }

    private sealed class TestEnvironment(string environmentName) : IHostEnvironment
    {
        public string EnvironmentName { get; set; } = environmentName;
        public string ApplicationName { get; set; } = "Stellarr.Api.Tests";
        public string ContentRootPath { get; set; } = "";
        public IFileProvider ContentRootFileProvider { get; set; } = new NullFileProvider();
    }

    private sealed class TemporaryFolder : IDisposable
    {
        public TemporaryFolder()
        {
            Path = Directory.CreateTempSubdirectory("stellarr-presence-").FullName;
        }

        public string Path { get; }

        public void Write(string name, string content) =>
            File.WriteAllText(System.IO.Path.Combine(Path, name), content);

        public void Dispose()
        {
            try
            {
                Directory.Delete(Path, recursive: true);
            }
            catch (DirectoryNotFoundException)
            {
            }
        }
    }
}
