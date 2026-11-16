using System.Net;
using System.Net.Http.Json;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Http.Connections;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.AspNetCore.SignalR;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Auth;
using Stellarr.Api.Domain;
using Stellarr.Api.Operations;

namespace Stellarr.Api.Tests;

/// <summary>
/// The address the whole API records and limits by, with requests arriving as
/// they do in the container: from nginx on loopback, behind a proxy at
/// <see cref="Proxy"/>.
/// </summary>
public sealed class TrustedProxyFlowTests
{
    private const string Password = "Passw0rd!2026";
    private const string Proxy = "172.18.0.1";
    private const string Client = "198.51.100.7";
    private const string Neighbour = "198.51.100.8";
    private const string Forged = "203.0.113.66";

    // The client wrote an address of its own in front of the one its proxy saw.
    private const string ForgedChain = $"{Forged}, {Client}, {Proxy}";

    [Theory]
    [InlineData(Proxy, Client)]
    [InlineData("", Proxy)]
    public async Task SignInRecordsTheAddressTheListedProxyReports(string proxies, string recorded)
    {
        using var factory = Factory(proxies);
        var account = await CreateAccountAsync(factory);
        using var client = NewClient(factory, ForgedChain);

        await SignInAsync(client, account.Username);

        await using var scope = factory.OpenScope(out var db);
        var session = await db.UserSessions.AsNoTracking().SingleAsync(x => x.UserId == account.Id);
        Assert.Equal(recorded, session.IpAddress);
    }

    [Theory]
    [InlineData(HttpTransportType.LongPolling)]
    [InlineData(HttpTransportType.WebSockets)]
    public async Task HubConnectionsSeeTheSameAddressAsTheSession(HttpTransportType transport)
    {
        var seen = new ConnectedAddresses();
        using var factory = Factory(Proxy, services => services.Configure<HubOptions>(options => options.AddFilter(seen)));
        var account = await CreateAccountAsync(factory);
        using var client = NewClient(factory, ForgedChain);
        var cookie = await SignInAsync(client, account.Username);

        await using var connection = Connection(factory, cookie, transport, ForgedChain);
        await connection.StartAsync();

        Assert.Equal(IPAddress.Parse(Client), await seen.First.Task.WaitAsync(TimeSpan.FromSeconds(10)));
        await using var scope = factory.OpenScope(out var db);
        Assert.Equal(Client, (await db.UserSessions.AsNoTracking().SingleAsync(x => x.UserId == account.Id)).IpAddress);
    }

    [Fact]
    public async Task EveryoneBehindAListedProxyHasASignInBudgetOfTheirOwn()
    {
        using var factory = Factory(Proxy);
        using var limited = Limited(factory);
        using var first = NewClient(limited, $"{Client}, {Proxy}");
        using var neighbour = NewClient(limited, $"{Neighbour}, {Proxy}");
        // The first person again, naming some other address as its own.
        using var disguised = NewClient(limited, ForgedChain);

        Assert.Equal(
            [HttpStatusCode.Unauthorized, HttpStatusCode.Unauthorized, HttpStatusCode.TooManyRequests],
            await FailToSignInAsync(first, times: 3));
        Assert.Equal([HttpStatusCode.TooManyRequests], await FailToSignInAsync(disguised, times: 1));
        Assert.Equal([HttpStatusCode.Unauthorized], await FailToSignInAsync(neighbour, times: 1));
    }

    [Fact]
    public async Task WithNothingListedEveryoneBehindTheProxySharesItsBudget()
    {
        using var factory = Factory("");
        using var limited = Limited(factory);
        using var first = NewClient(limited, $"{Client}, {Proxy}");
        using var neighbour = NewClient(limited, $"{Neighbour}, {Proxy}");

        await FailToSignInAsync(first, times: 2);

        Assert.Equal([HttpStatusCode.TooManyRequests], await FailToSignInAsync(neighbour, times: 1));
    }

    [Fact]
    public void AnEntryTheApiCannotUseStopsItAtStart()
    {
        using var factory = Factory("172.18.0.1, proxy.example");

        var failure = Assert.ThrowsAny<Exception>(() => factory.CreateClient());

        Assert.Contains(
            $"{TrustedProxies.SettingName} ({TrustedProxies.VariableName}) lists \"proxy.example\"",
            failure.ToString(),
            StringComparison.Ordinal);
    }

    private static StellarrWebApplicationFactory Factory(string proxies, Action<IServiceCollection>? configureServices = null) =>
        new(
            loadTestData: true,
            configuration: new Dictionary<string, string?> { [TrustedProxies.SettingName] = proxies },
            configureServices: services =>
            {
                services.AddSingleton<IStartupFilter>(new PeerFromHeader());
                configureServices?.Invoke(services);
            });

    /// <summary>
    /// The sign-in limit is lifted in the Testing environment, so this host
    /// runs as development, where it comes from configuration.
    /// </summary>
    private static WebApplicationFactory<Program> Limited(StellarrWebApplicationFactory factory) =>
        factory.WithWebHostBuilder(builder =>
        {
            builder.UseEnvironment("Development");
            builder.UseSetting("RateLimiting:Login:PermitLimit", "2");
        });

    /// <summary>A client whose requests reach the API from nginx, carrying <paramref name="forwardedFor"/>.</summary>
    private static HttpClient NewClient(WebApplicationFactory<Program> factory, string forwardedFor)
    {
        var client = factory.CreateClient(new WebApplicationFactoryClientOptions { HandleCookies = false });
        client.DefaultRequestHeaders.Add(PeerFromHeader.Header, "127.0.0.1");
        client.DefaultRequestHeaders.Add("X-Forwarded-For", forwardedFor);
        return client;
    }

    private static HubConnection Connection(
        StellarrWebApplicationFactory factory,
        string cookie,
        HttpTransportType transport,
        string forwardedFor) => new HubConnectionBuilder()
            .WithUrl("https://localhost/hubs/app", options =>
            {
                options.Headers["Cookie"] = cookie;
                options.Headers[PeerFromHeader.Header] = "127.0.0.1";
                options.Headers["X-Forwarded-For"] = forwardedFor;
                options.Transports = transport;
                options.HttpMessageHandlerFactory = _ => factory.Server.CreateHandler();
                // The WebSocket handshake is not made through the handler above and carries no headers of its own.
                options.WebSocketFactory = async (context, cancellationToken) =>
                {
                    var socketClient = factory.Server.CreateWebSocketClient();
                    socketClient.ConfigureRequest = request =>
                    {
                        request.Headers.Cookie = cookie;
                        request.Headers[PeerFromHeader.Header] = "127.0.0.1";
                        request.Headers["X-Forwarded-For"] = forwardedFor;
                    };
                    return await socketClient.ConnectAsync(context.Uri, cancellationToken);
                };
            })
            .Build();

    private static async Task<User> CreateAccountAsync(StellarrWebApplicationFactory factory)
    {
        await using var scope = factory.OpenScope(out var db);
        var user = (await DataModelSupport.AddUsersAsync(db, 1))[0];
        user.PasswordHash = scope.ServiceProvider.GetRequiredService<PasswordService>().Hash(Password);
        await db.SaveChangesAsync();
        return user;
    }

    /// <summary>Signs in and returns the cookies the server set, as a Cookie header.</summary>
    private static async Task<string> SignInAsync(HttpClient client, string username)
    {
        using var response = await client.PostAsJsonAsync("/api/auth/login", new { username, password = Password });
        response.EnsureSuccessStatusCode();
        return string.Join("; ", response.Headers.GetValues("Set-Cookie").Select(x => x.Split(';', 2)[0]));
    }

    private static async Task<HttpStatusCode[]> FailToSignInAsync(HttpClient client, int times)
    {
        var statuses = new HttpStatusCode[times];
        for (var attempt = 0; attempt < times; attempt++)
        {
            using var response = await client.PostAsJsonAsync(
                "/api/auth/login",
                new { username = "nobody-here", password = "not-the-password" });
            statuses[attempt] = response.StatusCode;
        }

        return statuses;
    }

    /// <summary>
    /// Stands in for the TCP connection the API sees: the address in
    /// <see cref="Header"/> becomes the request's remote address before any of
    /// the API's own middleware runs, as the connection from nginx would.
    /// </summary>
    private sealed class PeerFromHeader : IStartupFilter
    {
        internal const string Header = "X-Test-Peer";

        public Action<IApplicationBuilder> Configure(Action<IApplicationBuilder> next) => app =>
        {
            app.Use((context, following) =>
            {
                if (IPAddress.TryParse(context.Request.Headers[Header].ToString(), out var peer))
                {
                    context.Connection.RemoteIpAddress = peer;
                }

                return following(context);
            });
            next(app);
        };
    }

    /// <summary>The address the hub's own context holds when a connection opens.</summary>
    private sealed class ConnectedAddresses : IHubFilter
    {
        internal TaskCompletionSource<IPAddress?> First { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);

        public Task OnConnectedAsync(HubLifetimeContext context, Func<HubLifetimeContext, Task> next)
        {
            First.TrySetResult(context.Context.GetHttpContext()?.Connection.RemoteIpAddress);
            return next(context);
        }
    }
}
