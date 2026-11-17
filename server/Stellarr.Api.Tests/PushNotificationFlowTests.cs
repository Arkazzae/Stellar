using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Data;
using Stellarr.Api.Notifications;

namespace Stellarr.Api.Tests;

public sealed class PushNotificationFlowTests
{
    [Fact]
    public async Task DeliveryCapabilitiesAreAccountGlobalAndReportConfiguredPush()
    {
        var keys = PushTestKeys.Vapid();
        using var factory = CreateFactory(keys.PublicKey, keys.PrivateKey);
        using var anonymous = factory.CreateClient(new WebApplicationFactoryClientOptions
        {
            AllowAutoRedirect = false,
        });
        Assert.Equal(HttpStatusCode.Unauthorized, (await anonymous.GetAsync("/api/me/notification-delivery")).StatusCode);

        using var maja = await AuthenticatedClientAsync(factory, "maja", "Aurora!2026");
        // Not a member of the active server any more, but the notification
        // settings are still the account's own, not the server's.
        await using (var scope = factory.Services.CreateAsyncScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
            var member = await db.ServerMembers.SingleAsync(
                x => x.UserId == Guid.Parse("30000000-0000-0000-0000-000000000001"));
            db.ServerMembers.Remove(member);
            await db.SaveChangesAsync();
        }

        var capabilities = await maja.GetFromJsonAsync<JsonElement>("/api/me/notification-delivery");

        Assert.True(capabilities.GetProperty("emailAvailable").GetBoolean());
        Assert.False(capabilities.GetProperty("emailVerified").GetBoolean());
        Assert.True(capabilities.GetProperty("pushAvailable").GetBoolean());
        Assert.Equal(keys.PublicKey, capabilities.GetProperty("applicationServerKey").GetString());
        Assert.DoesNotContain("private", capabilities.GetRawText(), StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public async Task InvalidPushConfigurationMakesPushUnavailableWithoutFailingTheHost()
    {
        using var factory = CreateFactory("not-a-key", "not-a-key");
        using var client = await AuthenticatedClientAsync(factory, "ada", "Quasar!2026");

        var capabilities = await client.GetFromJsonAsync<JsonElement>("/api/me/notification-delivery");

        Assert.False(capabilities.GetProperty("pushAvailable").GetBoolean());
        Assert.Equal(JsonValueKind.Null, capabilities.GetProperty("applicationServerKey").ValueKind);
    }

    [Fact]
    public async Task MismatchedVapidKeysMakePushUnavailable()
    {
        var publicPair = PushTestKeys.Vapid();
        var privatePair = PushTestKeys.Vapid();
        using var factory = CreateFactory(publicPair.PublicKey, privatePair.PrivateKey);
        using var client = await AuthenticatedClientAsync(factory, "ada", "Quasar!2026");

        var capabilities = await client.GetFromJsonAsync<JsonElement>("/api/me/notification-delivery");

        Assert.False(capabilities.GetProperty("pushAvailable").GetBoolean());
        Assert.Equal(JsonValueKind.Null, capabilities.GetProperty("applicationServerKey").ValueKind);
    }

    [Fact]
    public async Task SubscriptionUpsertIsIdempotentForTheCurrentSession()
    {
        var keys = PushTestKeys.Vapid();
        using var factory = CreateFactory(keys.PublicKey, keys.PrivateKey);
        using var client = await AuthenticatedClientAsync(factory, "maja", "Aurora!2026");
        var subscription = PushTestKeys.Subscription();
        var endpoint = "https://push.example.test/send/first";

        var first = await SaveAsync(client, endpoint, subscription.P256dh, subscription.Auth);
        Assert.Equal(HttpStatusCode.OK, first.StatusCode);
        var firstId = (await ReadJsonAsync(first)).GetProperty("id").GetString();

        var second = await SaveAsync(client, endpoint, subscription.P256dh, subscription.Auth);
        Assert.Equal(HttpStatusCode.OK, second.StatusCode);
        Assert.Equal(firstId, (await ReadJsonAsync(second)).GetProperty("id").GetString());

        await using var scope = factory.Services.CreateAsyncScope();
        var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
        var rows = await db.PushSubscriptions.AsNoTracking()
            .Where(x => x.Endpoint == endpoint)
            .ToArrayAsync();
        Assert.Single(rows);
        Assert.Equal(firstId, rows[0].Id.ToString());
    }

    [Fact]
    public async Task EndpointIsNeverReassignedToAnotherAccountOrSignIn()
    {
        var keys = PushTestKeys.Vapid();
        using var factory = CreateFactory(keys.PublicKey, keys.PrivateKey);
        using var maja = await AuthenticatedClientAsync(factory, "maja", "Aurora!2026");
        var subscription = PushTestKeys.Subscription();
        var endpoint = "https://push.example.test/send/owned";

        var first = await SaveAsync(maja, endpoint, subscription.P256dh, subscription.Auth);
        Assert.Equal(HttpStatusCode.OK, first.StatusCode);
        var originalId = (await ReadJsonAsync(first)).GetProperty("id").GetString();

        using var ada = await AuthenticatedClientAsync(factory, "ada", "Quasar!2026");
        var otherAccount = await SaveAsync(ada, endpoint, subscription.P256dh, subscription.Auth);
        Assert.Equal(HttpStatusCode.Conflict, otherAccount.StatusCode);

        using var majaSecondSignIn = await AuthenticatedClientAsync(factory, "maja", "Aurora!2026");
        var otherSession = await SaveAsync(
            majaSecondSignIn,
            endpoint,
            subscription.P256dh,
            subscription.Auth);
        Assert.Equal(HttpStatusCode.Conflict, otherSession.StatusCode);

        await using (var scope = factory.Services.CreateAsyncScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
            var row = await db.PushSubscriptions.AsNoTracking().SingleAsync(x => x.Endpoint == endpoint);
            Assert.Equal(originalId, row.Id.ToString());
            Assert.Equal(Guid.Parse("30000000-0000-0000-0000-000000000001"), row.UserId);
        }

        var again = await SaveAsync(maja, endpoint, subscription.P256dh, subscription.Auth);
        Assert.Equal(HttpStatusCode.OK, again.StatusCode);
        Assert.Equal(originalId, (await ReadJsonAsync(again)).GetProperty("id").GetString());
    }

    [Fact]
    public async Task EndedSessionsSubscriptionCanMoveToANewSignInOfTheSameAccount()
    {
        var keys = PushTestKeys.Vapid();
        using var factory = CreateFactory(keys.PublicKey, keys.PrivateKey);
        using var first = await AuthenticatedClientAsync(factory, "maja", "Aurora!2026");
        var subscription = PushTestKeys.Subscription();
        var endpoint = "https://push.example.test/send/rebind";

        var saved = await SaveAsync(first, endpoint, subscription.P256dh, subscription.Auth);
        Assert.Equal(HttpStatusCode.OK, saved.StatusCode);
        var originalId = (await ReadJsonAsync(saved)).GetProperty("id").GetString();

        Guid oldSessionId;
        await using (var scope = factory.Services.CreateAsyncScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
            var row = await db.PushSubscriptions.SingleAsync(x => x.Endpoint == endpoint);
            oldSessionId = row.SessionId;
            var session = await db.UserSessions.SingleAsync(x => x.Id == oldSessionId);
            session.RevokedAt = DateTime.UtcNow;
            await db.SaveChangesAsync();
        }

        using var second = await AuthenticatedClientAsync(factory, "maja", "Aurora!2026");
        var rebound = await SaveAsync(second, endpoint, subscription.P256dh, subscription.Auth);
        Assert.Equal(HttpStatusCode.OK, rebound.StatusCode);
        Assert.Equal(originalId, (await ReadJsonAsync(rebound)).GetProperty("id").GetString());

        await using (var scope = factory.Services.CreateAsyncScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
            var row = await db.PushSubscriptions.SingleAsync(x => x.Endpoint == endpoint);
            Assert.Equal(originalId, row.Id.ToString());
            Assert.NotEqual(oldSessionId, row.SessionId);
            Assert.True(await db.UserSessions.AnyAsync(x => x.Id == row.SessionId && x.RevokedAt == null));
        }
    }

    [Fact]
    public async Task AnAccountHasAtMostTenSubscriptions()
    {
        var keys = PushTestKeys.Vapid();
        using var factory = CreateFactory(keys.PublicKey, keys.PrivateKey);
        using var client = await AuthenticatedClientAsync(factory, "maja", "Aurora!2026");
        var subscription = PushTestKeys.Subscription();

        for (var index = 0; index < 10; index++)
        {
            var accepted = await SaveAsync(
                client,
                $"https://push.example.test/send/{index}",
                subscription.P256dh,
                subscription.Auth);
            Assert.Equal(HttpStatusCode.OK, accepted.StatusCode);
        }

        var rejected = await SaveAsync(
            client,
            "https://push.example.test/send/one-too-many",
            subscription.P256dh,
            subscription.Auth);
        Assert.Equal(HttpStatusCode.Conflict, rejected.StatusCode);

        await using var scope = factory.Services.CreateAsyncScope();
        var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
        Assert.Equal(
            10,
            await db.PushSubscriptions.CountAsync(
                x => x.UserId == Guid.Parse("30000000-0000-0000-0000-000000000001")));
    }

    [Theory]
    [InlineData("http://push.example.test/send/plain")]
    [InlineData("https://user:secret@push.example.test/send/userinfo")]
    [InlineData("https://push.example.test/send/fragment#part")]
    [InlineData("https://push.example.test:8443/send/port")]
    [InlineData("https://localhost/send/local")]
    [InlineData("https://127.0.0.1/send/loopback")]
    [InlineData("https://[::1]/send/loopback-v6")]
    [InlineData("https://10.0.0.1/send/private")]
    [InlineData("https://192.168.1.1/send/private-v4")]
    [InlineData("https://169.254.1.1/send/link-local")]
    public async Task MalformedOrLocalEndpointsAreRejected(string endpoint)
    {
        var keys = PushTestKeys.Vapid();
        using var factory = CreateFactory(keys.PublicKey, keys.PrivateKey);
        using var client = await AuthenticatedClientAsync(factory, "maja", "Aurora!2026");
        var subscription = PushTestKeys.Subscription();

        var response = await SaveAsync(client, endpoint, subscription.P256dh, subscription.Auth);

        Assert.Equal(HttpStatusCode.BadRequest, response.StatusCode);
    }

    [Fact]
    public async Task MalformedKeysAreRejected()
    {
        var keys = PushTestKeys.Vapid();
        using var factory = CreateFactory(keys.PublicKey, keys.PrivateKey);
        using var client = await AuthenticatedClientAsync(factory, "maja", "Aurora!2026");
        var valid = PushTestKeys.Subscription();
        var endpoint = "https://push.example.test/send/keys";

        Assert.Equal(
            HttpStatusCode.BadRequest,
            (await SaveAsync(client, endpoint, "not-base64!", valid.Auth)).StatusCode);
        Assert.Equal(
            HttpStatusCode.BadRequest,
            (await SaveAsync(client, endpoint, PushTestKeys.Base64Url(new byte[64]), valid.Auth)).StatusCode);
        var offCurve = new byte[65];
        offCurve[0] = 0x04;
        Assert.Equal(
            HttpStatusCode.BadRequest,
            (await SaveAsync(client, endpoint, PushTestKeys.Base64Url(offCurve), valid.Auth)).StatusCode);
        Assert.Equal(
            HttpStatusCode.BadRequest,
            (await SaveAsync(client, endpoint, valid.P256dh, PushTestKeys.Base64Url(new byte[15]))).StatusCode);
        Assert.Equal(
            HttpStatusCode.BadRequest,
            (await SaveAsync(client, endpoint, valid.P256dh, valid.Auth + "+")).StatusCode);
    }

    [Fact]
    public async Task DeleteOnlyRemovesTheCurrentAccountsCurrentSession()
    {
        var keys = PushTestKeys.Vapid();
        using var factory = CreateFactory(keys.PublicKey, keys.PrivateKey);
        using var maja = await AuthenticatedClientAsync(factory, "maja", "Aurora!2026");
        using var majaOtherDevice = await AuthenticatedClientAsync(factory, "maja", "Aurora!2026");
        using var ada = await AuthenticatedClientAsync(factory, "ada", "Quasar!2026");
        var subscription = PushTestKeys.Subscription();
        var endpoint = "https://push.example.test/send/delete";

        Assert.Equal(
            HttpStatusCode.OK,
            (await SaveAsync(maja, endpoint, subscription.P256dh, subscription.Auth)).StatusCode);

        Assert.Equal(
            HttpStatusCode.NoContent,
            (await DeleteAsync(ada, endpoint)).StatusCode);
        Assert.Equal(
            HttpStatusCode.NoContent,
            (await DeleteAsync(majaOtherDevice, endpoint)).StatusCode);
        await using (var scope = factory.Services.CreateAsyncScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
            Assert.True(await db.PushSubscriptions.AnyAsync(x => x.Endpoint == endpoint));
        }

        Assert.Equal(
            HttpStatusCode.NoContent,
            (await DeleteAsync(maja, endpoint)).StatusCode);
        await using (var scope = factory.Services.CreateAsyncScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
            Assert.False(await db.PushSubscriptions.AnyAsync(x => x.Endpoint == endpoint));
        }
    }

    [Fact]
    public async Task EndedSessionCannotReadOrChangeDeliverySettings()
    {
        var keys = PushTestKeys.Vapid();
        using var factory = CreateFactory(keys.PublicKey, keys.PrivateKey);
        using var client = await AuthenticatedClientAsync(factory, "maja", "Aurora!2026");

        await using (var scope = factory.Services.CreateAsyncScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
            var sessions = await db.UserSessions
                .Where(x => x.UserId == Guid.Parse("30000000-0000-0000-0000-000000000001"))
                .ToArrayAsync();
            foreach (var session in sessions)
            {
                session.RevokedAt = DateTime.UtcNow;
            }

            await db.SaveChangesAsync();
        }

        Assert.Equal(HttpStatusCode.Unauthorized, (await client.GetAsync("/api/me/notification-delivery")).StatusCode);
        var subscription = PushTestKeys.Subscription();
        Assert.Equal(
            HttpStatusCode.Unauthorized,
            (await SaveAsync(client, "https://push.example.test/send/ended", subscription.P256dh, subscription.Auth))
                .StatusCode);
    }

    [Fact]
    public async Task MailCapabilityComesFromTheConfiguredTransport()
    {
        using var factory = new StellarrWebApplicationFactory(
            loadTestData: true,
            configuration: new Dictionary<string, string?>
            {
                ["Mail:Enabled"] = "false",
            });
        using var client = await AuthenticatedClientAsync(factory, "maja", "Aurora!2026");

        var capabilities = await client.GetFromJsonAsync<JsonElement>("/api/me/notification-delivery");

        Assert.False(capabilities.GetProperty("emailAvailable").GetBoolean());
        Assert.False(capabilities.GetProperty("pushAvailable").GetBoolean());
    }

    private static StellarrWebApplicationFactory CreateFactory(string publicKey, string privateKey) =>
        new(
            loadTestData: true,
            configuration: new Dictionary<string, string?>
            {
                ["Push:Enabled"] = "true",
                ["Push:Subject"] = "mailto:push@example.test",
                ["Push:PublicKey"] = publicKey,
                ["Push:PrivateKey"] = privateKey,
            });

    private static async Task<HttpClient> AuthenticatedClientAsync(
        StellarrWebApplicationFactory factory,
        string username,
        string password)
    {
        var client = factory.CreateClient(new WebApplicationFactoryClientOptions
        {
            AllowAutoRedirect = false,
            HandleCookies = true,
        });
        var login = await client.PostAsJsonAsync("/api/auth/login", new { username, password });
        login.EnsureSuccessStatusCode();
        return client;
    }

    private static Task<HttpResponseMessage> SaveAsync(
        HttpClient client,
        string endpoint,
        string p256dh,
        string auth) =>
        client.PutAsJsonAsync("/api/me/push-subscriptions", new
        {
            endpoint,
            keys = new { p256dh, auth },
        });

    private static Task<HttpResponseMessage> DeleteAsync(HttpClient client, string endpoint) =>
        client.SendAsync(new HttpRequestMessage(HttpMethod.Delete, "/api/me/push-subscriptions")
        {
            Content = JsonContent.Create(new { endpoint }),
        });

    private static async Task<JsonElement> ReadJsonAsync(HttpResponseMessage response)
    {
        using var document = JsonDocument.Parse(await response.Content.ReadAsStringAsync());
        return document.RootElement.Clone();
    }
}
