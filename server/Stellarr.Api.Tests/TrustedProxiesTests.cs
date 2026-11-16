using System.Net;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Http;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Operations;

namespace Stellarr.Api.Tests;

/// <summary>
/// The list of trusted proxies, and the address, scheme and host the API takes
/// from a request once the X-Forwarded-* headers are read the way Program.cs
/// reads them. Each request arrives as it would from the container's nginx: on
/// loopback, with the address of whoever connected to nginx appended to
/// X-Forwarded-For.
/// </summary>
public sealed class TrustedProxiesTests
{
    private const string Nginx = "127.0.0.1";
    private const string Proxy = "172.18.0.1";
    private const string Client = "198.51.100.7";
    private const string Forged = "203.0.113.66";

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("   ")]
    [InlineData(" ,\n, \t")]
    public void BlankListsNoProxy(string? value) => Assert.Empty(TrustedProxies.Parse(value).Networks);

    [Theory]
    [InlineData("172.18.0.1", "172.18.0.1/32")]
    [InlineData("172.18.0.0/16", "172.18.0.0/16")]
    [InlineData("10.0.0.0/8", "10.0.0.0/8")]
    [InlineData("172.018.000.001", "172.18.0.1/32")]
    [InlineData("2001:db8::7", "2001:db8::7/128")]
    [InlineData("2001:DB8:1::/48", "2001:db8:1::/48")]
    [InlineData("fd00::/8", "fd00::/8")]
    public void ReadsAnAddressOrANetwork(string value, string network) =>
        Assert.Equal(network, Assert.Single(TrustedProxies.Parse(value).Networks).ToString());

    [Fact]
    public void ReadsSeveralEntriesSeparatedByCommasAndWhitespace()
    {
        var proxies = TrustedProxies.Parse(" 172.18.0.1 ,10.0.0.0/8,\n\t2001:db8::/32  fd00::1, ");

        Assert.Equal(
            ["172.18.0.1/32", "10.0.0.0/8", "2001:db8::/32", "fd00::1/128"],
            proxies.Networks.Select(network => network.ToString()));
    }

    [Theory]
    [InlineData("proxy.example")]
    [InlineData("localhost")]
    [InlineData("10")]
    [InlineData("10.1")]
    [InlineData("10.0.0")]
    [InlineData("10.0.0.256")]
    [InlineData("10.0.0.1.5")]
    [InlineData("0x0a.0.0.1")]
    [InlineData("10.0.0.1:8080")]
    [InlineData("[2001:db8::1]")]
    [InlineData("fe80::1%eth0")]
    [InlineData("2001:db8::g")]
    [InlineData("10.0.0.0/")]
    [InlineData("10.0.0.0/33")]
    [InlineData("10.0.0.0/-8")]
    [InlineData("10.0.0.0/8/8")]
    [InlineData("2001:db8::/129")]
    [InlineData("10.0.0.1/24")]
    [InlineData("2001:db8::1/64")]
    [InlineData("0.0.0.0/0")]
    [InlineData("::/0")]
    public void AnEntryTheApiCannotUseIsNamedRatherThanDropped(string entry)
    {
        var error = Assert.Throws<InvalidOperationException>(
            () => TrustedProxies.Parse($"172.18.0.1, {entry}, 10.0.0.0/8"));

        Assert.StartsWith(
            $"{TrustedProxies.SettingName} ({TrustedProxies.VariableName}) lists \"{entry}\", which ",
            error.Message,
            StringComparison.Ordinal);
    }

    [Theory]
    [InlineData("10.0.0.1/24", "the network is 10.0.0.0/24")]
    [InlineData("2001:db8::1/64", "the network is 2001:db8::/64")]
    [InlineData("0.0.0.0/0", "anyone could choose the address")]
    [InlineData("10.0.0.0/33", "a number from 1 to 32")]
    [InlineData("2001:db8::/129", "a number from 1 to 128")]
    public void TheMessageSaysWhatIsWrongWithTheEntry(string entry, string explanation)
    {
        var error = Assert.Throws<InvalidOperationException>(() => TrustedProxies.Parse(entry));

        Assert.Contains(explanation, error.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void LoadReadsTheOperationsSetting()
    {
        var configuration = new ConfigurationBuilder()
            .AddInMemoryCollection(new Dictionary<string, string?> { [TrustedProxies.SettingName] = "172.18.0.1" })
            .Build();

        Assert.Equal("172.18.0.1/32", Assert.Single(TrustedProxies.Load(configuration).Networks).ToString());
    }

    [Theory]
    // A client that reaches the container directly.
    [InlineData(Client, Client)]
    // Whatever it writes into the header itself.
    [InlineData($"{Forged}, {Client}", Client)]
    // A client behind a proxy: the proxy is all the container sees.
    [InlineData($"{Client}, {Proxy}", Proxy)]
    [InlineData($"{Forged}, {Client}, {Proxy}", Proxy)]
    public async Task WithNothingListedTheAddressIsWhoeverConnectedToTheContainer(string forwardedFor, string address)
    {
        var context = await ReceiveAsync("", Nginx, forwardedFor);

        Assert.Equal(IPAddress.Parse(address), context.Connection.RemoteIpAddress);
    }

    [Theory]
    [InlineData($"{Client}, {Proxy}")]
    // What the client wrote stays to the left of what the proxy appended.
    [InlineData($"{Forged}, {Client}, {Proxy}")]
    [InlineData($"{Forged}, 10.0.0.9, {Client}, {Proxy}")]
    // Even a client that writes the proxy's own address in front of its own.
    [InlineData($"{Forged}, {Proxy}, {Client}, {Proxy}")]
    public async Task AListedProxyNamesTheClientAndNothingTheClientWrote(string forwardedFor)
    {
        var context = await ReceiveAsync(Proxy, Nginx, forwardedFor);

        Assert.Equal(IPAddress.Parse(Client), context.Connection.RemoteIpAddress);
    }

    [Theory]
    // A client that reaches the container without the proxy is not believed,
    [InlineData($"{Forged}, {Client}")]
    // even when it claims to have come through the proxy.
    [InlineData($"{Forged}, {Proxy}, {Client}")]
    // A proxy that is not listed is the address, as with nothing listed.
    [InlineData($"{Forged}, 192.0.2.44")]
    public async Task ASenderThatIsNotAListedProxyChangesNothing(string forwardedFor)
    {
        var context = await ReceiveAsync(Proxy, Nginx, forwardedFor);

        Assert.Equal(IPAddress.Parse(forwardedFor.Split(", ")[^1]), context.Connection.RemoteIpAddress);
    }

    [Fact]
    public async Task OnlyTheContainersNginxIsBelievedWithoutBeingListed()
    {
        var context = await ReceiveAsync(Proxy, "192.0.2.44", $"{Forged}, {Client}");

        Assert.Equal(IPAddress.Parse("192.0.2.44"), context.Connection.RemoteIpAddress);
    }

    [Theory]
    [InlineData(Nginx)]
    [InlineData("::1")]
    [InlineData("::ffff:127.0.0.1")]
    public async Task NginxIsBelievedOnEitherLoopback(string peer)
    {
        var context = await ReceiveAsync(Proxy, peer, $"{Client}, {Proxy}");

        Assert.Equal(IPAddress.Parse(Client), context.Connection.RemoteIpAddress);
    }

    [Theory]
    [InlineData("127.0.0.1")]
    [InlineData("::1")]
    public async Task LoopbackPastNginxIsNotBelieved(string loopback)
    {
        // Something on the proxy's own machine connected to it over loopback
        // and wrote the first entry itself.
        var context = await ReceiveAsync(Proxy, Nginx, $"{Forged}, {loopback}, {Proxy}");

        Assert.Equal(IPAddress.Parse(loopback), context.Connection.RemoteIpAddress);
    }

    [Fact]
    public async Task LoopbackPastNginxIsBelievedOnceListed()
    {
        var context = await ReceiveAsync($"{Proxy}, 127.0.0.1", Nginx, $"{Client}, 127.0.0.1, {Proxy}");

        Assert.Equal(IPAddress.Parse(Client), context.Connection.RemoteIpAddress);
    }

    [Theory]
    // Two proxies in a row, both listed: the client.
    [InlineData("172.18.0.1, 10.0.0.0/24", Client)]
    // Only the outer one listed: the inner proxy, which nobody vouches for.
    [InlineData("172.18.0.1", "10.0.0.5")]
    public async Task EachProxyInAChainHasToBeListed(string proxies, string address)
    {
        var context = await ReceiveAsync(proxies, Nginx, $"{Forged}, {Client}, 10.0.0.5, {Proxy}");

        Assert.Equal(IPAddress.Parse(address), context.Connection.RemoteIpAddress);
    }

    [Theory]
    [InlineData("2001:db8:1::/64", "2001:db8:ff::7, 2001:db8:1::10", "2001:db8:ff::7")]
    [InlineData("2001:db8:1::/64", "2001:db8:1::99, 2001:db8:ff::7, 2001:db8:1::10", "2001:db8:ff::7")]
    [InlineData("2001:db8:1::10", "[2001:db8:ff::7]:4711, 2001:db8:1::10", "2001:db8:ff::7")]
    [InlineData("172.18.0.0/16", "2001:db8:ff::7, 172.18.0.5", "2001:db8:ff::7")]
    [InlineData("fd00::5", "198.51.100.7:4711, fd00::5", Client)]
    [InlineData("2001:db8:1::/64", "2001:db8:ff::7, 172.18.0.5", "172.18.0.5")]
    public async Task IPv4AndIPv6MixFreely(string proxies, string forwardedFor, string address)
    {
        var context = await ReceiveAsync(proxies, Nginx, forwardedFor);

        Assert.Equal(IPAddress.Parse(address), context.Connection.RemoteIpAddress);
    }

    [Theory]
    [InlineData("")]
    [InlineData(Proxy)]
    public async Task TheSchemeAndHostAreNginxsWhateverIsListed(string proxies)
    {
        var context = await ReceiveAsync(
            proxies,
            Nginx,
            $"{Forged}, {Client}, {Proxy}",
            forwardedProto: "https",
            forwardedHost: "forged.example, chat.example.test");

        Assert.Equal("https", context.Request.Scheme);
        Assert.Equal("chat.example.test", context.Request.Host.Value);
    }

    /// <summary>Runs one request through the forwarded-header middleware as Program.cs adds it.</summary>
    private static async Task<HttpContext> ReceiveAsync(
        string proxies,
        string peer,
        string forwardedFor,
        string? forwardedProto = null,
        string? forwardedHost = null)
    {
        await using var services = new ServiceCollection().AddLogging().BuildServiceProvider();
        var app = new ApplicationBuilder(services);
        app.UseForwardedHeaders(TrustedProxies.Parse(proxies));
        var pipeline = app.Build();

        var context = new DefaultHttpContext { RequestServices = services };
        context.Connection.RemoteIpAddress = IPAddress.Parse(peer);
        context.Request.Scheme = "http";
        context.Request.Host = new HostString("stellarr");
        context.Request.Headers["X-Forwarded-For"] = forwardedFor;
        if (forwardedProto is not null)
        {
            context.Request.Headers["X-Forwarded-Proto"] = forwardedProto;
        }
        if (forwardedHost is not null)
        {
            context.Request.Headers["X-Forwarded-Host"] = forwardedHost;
        }

        await pipeline(context);
        return context;
    }
}
