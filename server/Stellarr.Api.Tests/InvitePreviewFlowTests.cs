using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Mvc.Testing;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

public sealed class InvitePreviewFlowTests
{
    private static readonly Guid MajaId = Guid.Parse("30000000-0000-0000-0000-000000000001");

    [Fact]
    public async Task AnInviteShowsAnybodyWhereItLeadsAndWhoSentItAndNothingMore()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await SignInAsync(factory);
        var toChannel = await CreateInviteAsync(maja, new { channelId = TestData.GeneralChannelId });
        var serverWide = await AddInviteAsync(factory, maxUses: null, uses: 0, expiresAt: null);
        using var anonymous = factory.CreateClient();

        using var response = await anonymous.GetAsync($"/api/invites/{toChannel.Code}");

        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        Assert.Equal("no-store", response.Headers.CacheControl?.ToString());
        var preview = await response.Content.ReadFromJsonAsync<JsonElement>();
        Assert.Equal(["channel", "expiresAt", "inviter", "server"], Names(preview));
        var server = preview.GetProperty("server");
        Assert.Equal(["description", "iconUrl", "id", "name"], Names(server));
        Assert.Equal(TestData.ServerId.ToString(), server.GetProperty("id").GetString());
        Assert.Equal("Orbit", server.GetProperty("name").GetString());
        Assert.Equal("Integration test server", server.GetProperty("description").GetString());
        Assert.Equal(JsonValueKind.Null, server.GetProperty("iconUrl").ValueKind);
        var channel = preview.GetProperty("channel");
        Assert.Equal(["id", "name"], Names(channel));
        Assert.Equal(TestData.GeneralChannelId.ToString(), channel.GetProperty("id").GetString());
        Assert.Equal("ogólny", channel.GetProperty("name").GetString());
        var inviter = preview.GetProperty("inviter");
        Assert.Equal(["avatarColor", "avatarUrl", "displayName"], Names(inviter));
        Assert.Equal("Maja", inviter.GetProperty("displayName").GetString());
        Assert.Equal("#777777", inviter.GetProperty("avatarColor").GetString());
        Assert.Equal(JsonValueKind.Null, inviter.GetProperty("avatarUrl").ValueKind);
        Assert.EndsWith("Z", preview.GetProperty("expiresAt").GetString());
        Assert.Equal(toChannel.ExpiresAt, preview.GetProperty("expiresAt").GetDateTime());

        var signedIn = await maja.GetFromJsonAsync<JsonElement>($"/api/invites/{toChannel.Code}");
        Assert.Equal(preview.GetRawText(), signedIn.GetRawText());

        var wide = await anonymous.GetFromJsonAsync<JsonElement>($"/api/invites/{serverWide}");
        Assert.Equal(JsonValueKind.Null, wide.GetProperty("channel").ValueKind);
        Assert.Equal(JsonValueKind.Null, wide.GetProperty("expiresAt").ValueKind);
    }

    [Fact]
    public async Task EveryInviteThatCannotBeUsedGetsTheSameNotFound()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await SignInAsync(factory);
        var revoked = await CreateInviteAsync(maja, new { });
        Assert.Equal(HttpStatusCode.NoContent, (await maja.DeleteAsync($"/api/admin/invites/{revoked.Id}")).StatusCode);
        var expired = await AddInviteAsync(factory, maxUses: null, uses: 0, expiresAt: DateTime.UtcNow.AddMinutes(-1));
        var usedUp = await AddInviteAsync(factory, maxUses: 3, uses: 3, expiresAt: null);
        var created = await maja.PostAsJsonAsync("/api/servers", new { name = "Na chwilę" });
        var deletedServerId = (await created.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetString()!;
        using var onDeleted = await SignInAsync(factory, deletedServerId);
        var toDeletedServer = await CreateInviteAsync(onDeleted, new { });
        Assert.Equal(HttpStatusCode.NoContent, (await onDeleted.DeleteAsync("/api/admin/server")).StatusCode);
        using var anonymous = factory.CreateClient();

        var answers = new List<string>();
        foreach (var code in new[] { "0123456789abcdef", revoked.Code, expired, usedUp, toDeletedServer.Code })
        {
            using var response = await anonymous.GetAsync($"/api/invites/{code}");
            var problem = await response.Content.ReadFromJsonAsync<JsonElement>();
            // The trace id differs from one request to the next whatever the invite.
            var fields = problem.EnumerateObject()
                .Where(x => x.Name != "traceId")
                .Select(x => $"{x.Name}={x.Value.GetRawText()}");
            var heading = new[]
            {
                $"{(int)response.StatusCode}",
                $"{response.Content.Headers.ContentType?.MediaType}",
                $"{response.Headers.CacheControl}",
            };
            answers.Add(string.Join("; ", heading.Concat(fields)));
        }

        var answer = Assert.Single(answers.Distinct());
        Assert.StartsWith("404; application/problem+json; no-store; ", answer);
    }

    private static string[] Names(JsonElement element) =>
        element.EnumerateObject().Select(x => x.Name).Order(StringComparer.Ordinal).ToArray();

    private static async Task<HttpClient> SignInAsync(StellarrWebApplicationFactory factory, string? serverId = null)
    {
        var client = factory.CreateClient(new WebApplicationFactoryClientOptions
        {
            AllowAutoRedirect = false,
            HandleCookies = true,
        });
        (await client.PostAsJsonAsync("/api/auth/login", new { username = "maja", password = "Aurora!2026" }))
            .EnsureSuccessStatusCode();
        if (serverId is not null)
        {
            client.DefaultRequestHeaders.Add("X-Stellarr-Server", serverId);
        }

        return client;
    }

    private static async Task<(Guid Id, string Code, DateTime ExpiresAt)> CreateInviteAsync(HttpClient client, object request)
    {
        var response = await client.PostAsJsonAsync("/api/admin/invites", request);
        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        var invite = await response.Content.ReadFromJsonAsync<JsonElement>();
        var url = invite.GetProperty("url").GetString()!;
        return (
            Guid.Parse(invite.GetProperty("id").GetString()!),
            url[(url.LastIndexOf('/') + 1)..],
            invite.GetProperty("expiresAt").GetDateTime());
    }

    /// <summary>Stores an invite by Maja to the seeded server and returns its code.</summary>
    private static async Task<string> AddInviteAsync(
        StellarrWebApplicationFactory factory,
        int? maxUses,
        int uses,
        DateTime? expiresAt)
    {
        await using var scope = factory.OpenScope(out var db);
        var code = Convert.ToHexString(Guid.NewGuid().ToByteArray()[..8]).ToLowerInvariant();
        db.ServerInvites.Add(new ServerInvite
        {
            Id = Guid.NewGuid(),
            ServerId = TestData.ServerId,
            CreatedById = MajaId,
            Code = code,
            Uses = uses,
            MaxUses = maxUses,
            ExpiresAt = expiresAt,
            CreatedAt = DateTime.UtcNow,
        });
        await db.SaveChangesAsync();
        return code;
    }
}
