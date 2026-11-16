using System.Net;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Http;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using Stellarr.Api.Logging;

namespace Stellarr.Api.Tests;

/// <summary>
/// What a request leaves in the log, as the Cockpit buffer holds it: the same
/// lines reach the console.
/// </summary>
public sealed class RequestLoggingTests
{
    private const string FailingPath = "/test/fails";

    [Fact]
    public async Task AnOrdinaryRequestLeavesOneLineWithItsMethodPathAndStatus()
    {
        using var factory = new StellarrWebApplicationFactory(loadTestData: false);
        using var client = factory.CreateClient();

        Assert.Equal(HttpStatusCode.OK, (await client.GetAsync("/api/auth/registration?probe=secret")).StatusCode);

        var lines = await LinesOfRequestAsync(factory, "/api/auth/registration");
        var line = Assert.Single(lines);
        Assert.Equal(LogLevel.Information, line.Level);
        Assert.StartsWith("GET /api/auth/registration 200 in ", line.Message, StringComparison.Ordinal);
        Assert.DoesNotContain("secret", line.Message, StringComparison.Ordinal);
    }

    [Fact]
    public async Task AFailingRequestLeavesItsErrorBesideItsLine()
    {
        using var factory = new StellarrWebApplicationFactory(
            loadTestData: false,
            configureServices: services => services.AddTransient<IStartupFilter, FailingRoute>());
        using var client = factory.CreateClient();

        Assert.Equal(HttpStatusCode.InternalServerError, (await client.GetAsync(FailingPath)).StatusCode);

        var lines = await LinesOfRequestAsync(factory, FailingPath);
        Assert.Contains(lines, line => line.Level == LogLevel.Error
            && line.Message.Contains("thrown on purpose", StringComparison.Ordinal));
        Assert.Contains(lines, line => line.Level == LogLevel.Information
            && line.Message.StartsWith($"GET {FailingPath} 500 in ", StringComparison.Ordinal));
    }

    [Fact]
    public async Task AProbeLeavesNothingWhileTheRequestLineIsOn()
    {
        using var factory = new StellarrWebApplicationFactory(loadTestData: false);
        using var client = factory.CreateClient();

        Assert.Equal(HttpStatusCode.OK, (await client.GetAsync("/health/ready")).StatusCode);
        // A request after the probe, whose line marks that the probe's would be in.
        _ = await client.GetAsync("/api/auth/registration");
        await LinesOfRequestAsync(factory, "/api/auth/registration");

        var buffer = factory.Services.GetRequiredService<CockpitLogBuffer>();
        Assert.DoesNotContain(buffer.All(), line => line.Message.Contains("/health/", StringComparison.Ordinal));
    }

    [Fact]
    public async Task AnOperatorCanBringBackTheFrameworksLinesFromTheConfiguration()
    {
        using var factory = new StellarrWebApplicationFactory(
            loadTestData: false,
            configuration: new Dictionary<string, string?> { ["Logging:LogLevel:Microsoft.AspNetCore"] = "Information" });
        using var client = factory.CreateClient();

        _ = await client.GetAsync("/api/auth/registration");

        var lines = await LinesOfRequestAsync(factory, "/api/auth/registration");
        Assert.True(lines.Count > 1, $"{lines.Count} line for the request");
        Assert.Contains(lines, line => line.Message.StartsWith("GET /api/auth/registration 200 in ", StringComparison.Ordinal));
    }

    /// <summary>
    /// Every line of the request whose own line names <paramref name="path"/>,
    /// by its trace. The request line is written once the response is out, so
    /// it is waited for.
    /// </summary>
    private static async Task<IReadOnlyList<CockpitLogRecord>> LinesOfRequestAsync(
        StellarrWebApplicationFactory factory,
        string path)
    {
        var buffer = factory.Services.GetRequiredService<CockpitLogBuffer>();
        for (var attempt = 0; attempt < 100; attempt++)
        {
            var all = buffer.All();
            var own = all.FirstOrDefault(line => line.Message.StartsWith($"GET {path} ", StringComparison.Ordinal));
            if (own is not null)
            {
                return all.Where(line => line.TraceId == own.TraceId).ToList();
            }

            await Task.Delay(20);
        }

        throw new InvalidOperationException($"No line for {path} reached the log.");
    }

    /// <summary>A route that throws, behind the API's own middleware and exception handler.</summary>
    private sealed class FailingRoute : IStartupFilter
    {
        public Action<IApplicationBuilder> Configure(Action<IApplicationBuilder> next) => app =>
        {
            next(app);
            app.Map(FailingPath, branch => branch.Run(_ => throw new InvalidOperationException("thrown on purpose")));
        };
    }
}
