using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.AspNetCore.Routing;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Auth;
using Stellarr.Api.Domain;
using Stellarr.Api.Endpoints;
using Stellarr.Api.Integrations.Discord;

namespace Stellarr.Api.Tests;

/// <summary>
/// A placeholder, the account that stands in for a Discord author, is
/// nobody: every route that finds a person or acts on one answers it as an
/// account that is not there, and only the routes that show what it wrote
/// show it, marked as what it is.
/// </summary>
public sealed class DiscordPlaceholderTests(StellarrWebApplicationFactory factory)
    : IClassFixture<StellarrWebApplicationFactory>
{
    private const string Password = "Ghost!2026-placeholder";
    private static readonly string[] ManageMessages = ["manageMessages"];

    private static readonly Dictionary<string, (Guid Id, string Password)> Accounts = new()
    {
        ["maja"] = (Guid.Parse("30000000-0000-0000-0000-000000000001"), "Aurora!2026"),
        ["ada"] = (Guid.Parse("30000000-0000-0000-0000-000000000002"), "Quasar!2026"),
        ["nils"] = (Guid.Parse("30000000-0000-0000-0000-000000000004"), "Zenith!2026"),
    };

    /// <summary>
    /// Every route with a person's id in its path, which the walk below must
    /// cover: one added later fails here until it is walked too.
    /// </summary>
    private static readonly string[] RoutesNamingAPerson =
    [
        "DELETE /api/admin/bans/{userId:guid}",
        "DELETE /api/admin/channels/{channelId:guid}/members/{userId:guid}",
        "PATCH /api/admin/channels/{channelId:guid}/members/{userId:guid}",
        "PUT /api/admin/channels/{targetId:guid}/permissions/members/{userId:guid}",
        "DELETE /api/admin/channels/{targetId:guid}/permissions/members/{userId:guid}",
        "POST /api/admin/invites/{inviteId:guid}/members/{userId:guid}",
        "DELETE /api/admin/members/{userId:guid}",
        "POST /api/admin/members/{userId:guid}/ban",
        "PATCH /api/admin/members/{userId:guid}/role",
        "PUT /api/admin/members/{userId:guid}/roles",
        "PUT /api/admin/sections/{targetId:guid}/permissions/members/{userId:guid}",
        "DELETE /api/admin/sections/{targetId:guid}/permissions/members/{userId:guid}",
        "POST /api/cockpit/users/{userId:guid}/identities/{identityId:guid}/remove",
        "PATCH /api/cockpit/users/{userId:guid}/role",
        "GET /api/cockpit/users/{userId:guid}/security",
        "POST /api/cockpit/users/{userId:guid}/security/recover",
        "PATCH /api/cockpit/users/{userId:guid}/status",
        "POST /api/social/blocks/{userId:guid}",
        "DELETE /api/social/blocks/{userId:guid}",
        "DELETE /api/social/friends/{userId:guid}",
    ];

    [Fact]
    public async Task APlaceholderIsAbsentOrRefusedEverywhereAPersonIsFoundOrActedOn()
    {
        var walked = new HashSet<string>(StringComparer.Ordinal);
        var (serverId, placeholder, textChannel, voiceChannel, section) = await ImportAsync();
        var id = placeholder.ToString();
        using var ada = await SignInAsync("ada");
        using var nils = await SignInAsync("nils");
        using var maja = await SignInAsync("maja");
        using var anonymous = Client();

        async Task<HttpStatusCode> Send(HttpClient client, HttpMethod method, string route, string path, object? body = null)
        {
            walked.Add($"{method} {route}");
            using var request = new HttpRequestMessage(method, path)
            {
                Content = body is null ? null : JsonContent.Create(body),
            };
            // Cockpit speaks for the instance; everything else for the imported server.
            if (!path.StartsWith("/api/cockpit/", StringComparison.Ordinal))
            {
                request.Headers.Add(ServerAccess.HeaderName, serverId.ToString());
            }

            using var response = await client.SendAsync(request);
            return response.StatusCode;
        }

        // Signing in, with the account's right password; every other way in
        // (a passkey, a provider, a desktop handoff, a CLI credential) needs
        // something only a signed-in account or the CLI's recovery can make,
        // and recovery refuses a disabled account.
        foreach (var username in new[] { $"discord:{DiscordAuthor}", $"DISCORD:{DiscordAuthor}" })
        {
            Assert.Equal(
                HttpStatusCode.Unauthorized,
                (await anonymous.PostAsJsonAsync("/api/auth/login", new { username, password = Password })).StatusCode);
        }

        // Nobody can take the shape of such a name, by signing up or in Cockpit.
        Assert.Equal(
            HttpStatusCode.BadRequest,
            (await anonymous.PostAsJsonAsync("/api/auth/register", new
            {
                username = "discord:42",
                displayName = "Taker",
                email = "taker@example.com",
                password = "Taker!2026-long-enough",
            })).StatusCode);
        Assert.Equal(
            HttpStatusCode.BadRequest,
            await Send(maja, HttpMethod.Post, "/api/cockpit/users", "/api/cockpit/users", new
            {
                username = "discord:43",
                displayName = "Taker",
                password = "Taker!2026-long-enough",
                roleId = await InstanceMemberRoleAsync(),
            }));

        // Direct messages, friends and blocks: as an account that does not exist.
        Assert.Equal(
            HttpStatusCode.NotFound,
            (await nils.PostAsJsonAsync("/api/social/conversations", new { userId = placeholder })).StatusCode);
        var request = await nils.PostAsJsonAsync("/api/social/requests", new { username = $"discord:{DiscordAuthor}" });
        Assert.Equal("unknown-user", (await request.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("result").GetString());
        Assert.Equal(HttpStatusCode.NotFound, await Send(nils, HttpMethod.Post, "/api/social/blocks/{userId:guid}", $"/api/social/blocks/{id}"));
        Assert.Equal(HttpStatusCode.NotFound, await Send(nils, HttpMethod.Delete, "/api/social/blocks/{userId:guid}", $"/api/social/blocks/{id}"));
        Assert.Equal(HttpStatusCode.NotFound, await Send(nils, HttpMethod.Delete, "/api/social/friends/{userId:guid}", $"/api/social/friends/{id}"));
        var social = await nils.GetFromJsonAsync<JsonElement>("/api/social");
        Assert.DoesNotContain(id, social.GetRawText(), StringComparison.Ordinal);

        // An invite sent to it, the moderation of a member, and their permissions.
        var invite = await ada.SendAsync(new HttpRequestMessage(HttpMethod.Post, "/api/admin/invites")
        {
            Content = JsonContent.Create(new { channelId = (Guid?)null }),
            Headers = { { ServerAccess.HeaderName, serverId.ToString() } },
        });
        invite.EnsureSuccessStatusCode();
        var inviteId = (await invite.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetString();
        Assert.Equal(HttpStatusCode.NotFound,
            await Send(ada, HttpMethod.Post, "/api/admin/invites/{inviteId:guid}/members/{userId:guid}", $"/api/admin/invites/{inviteId}/members/{id}"));
        var baseRole = await ServerMemberRoleAsync(serverId);
        Assert.Equal(HttpStatusCode.NotFound,
            await Send(ada, HttpMethod.Patch, "/api/admin/members/{userId:guid}/role", $"/api/admin/members/{id}/role", new { roleId = baseRole }));
        Assert.Equal(HttpStatusCode.NotFound,
            await Send(ada, HttpMethod.Put, "/api/admin/members/{userId:guid}/roles", $"/api/admin/members/{id}/roles", new { roleIds = Array.Empty<Guid>() }));
        Assert.Equal(HttpStatusCode.NotFound,
            await Send(ada, HttpMethod.Post, "/api/admin/members/{userId:guid}/ban", $"/api/admin/members/{id}/ban", new { reason = "imported" }));
        Assert.Equal(HttpStatusCode.NotFound,
            await Send(ada, HttpMethod.Delete, "/api/admin/members/{userId:guid}", $"/api/admin/members/{id}"));
        Assert.Equal(HttpStatusCode.NotFound,
            await Send(ada, HttpMethod.Delete, "/api/admin/bans/{userId:guid}", $"/api/admin/bans/{id}"));
        Assert.Equal(HttpStatusCode.NotFound,
            await Send(ada, HttpMethod.Delete, "/api/admin/channels/{channelId:guid}/members/{userId:guid}", $"/api/admin/channels/{voiceChannel}/members/{id}"));
        Assert.Equal(HttpStatusCode.NotFound,
            await Send(ada, HttpMethod.Patch, "/api/admin/channels/{channelId:guid}/members/{userId:guid}", $"/api/admin/channels/{voiceChannel}/members/{id}",
                new { targetChannelId = voiceChannel }));
        foreach (var (kind, target) in new[] { ("channels", textChannel), ("sections", section) })
        {
            var route = $"/api/admin/{kind}/{{targetId:guid}}/permissions/members/{{userId:guid}}";
            var path = $"/api/admin/{kind}/{target}/permissions/members/{id}";
            Assert.Equal(HttpStatusCode.NotFound,
                await Send(ada, HttpMethod.Put, route, path, new { allow = Array.Empty<string>(), deny = ManageMessages }));
            Assert.Equal(HttpStatusCode.NotFound, await Send(ada, HttpMethod.Delete, route, path));
        }

        // The successor must be a member: this one is refused as an invalid choice.
        Assert.Equal(HttpStatusCode.BadRequest,
            await Send(ada, HttpMethod.Post, "/api/admin/server/transfer", "/api/admin/server/transfer", new { userId = placeholder }));

        // Member lists: the placeholder is no member of anything.
        foreach (var (client, path) in new[]
                 {
                     (ada, "/api/bootstrap"),
                     (ada, "/api/admin/members"),
                     (ada, "/api/admin/bans"),
                     (ada, $"/api/admin/channels/{textChannel}/permissions/members"),
                     (ada, "/api/servers"),
                     (maja, "/api/cockpit/users"),
                     (maja, $"/api/cockpit/servers/{serverId}/members"),
                     (maja, "/api/cockpit/sessions"),
                 })
        {
            var listed = await GetAsync(client, serverId, path);
            var members = path == "/api/bootstrap" ? listed.GetProperty("members").GetRawText() : listed.GetRawText();
            Assert.DoesNotContain(id, members, StringComparison.Ordinal);
        }

        // Cockpit: not a user to manage, and not counted as one.
        foreach (var (method, route, body) in new (HttpMethod, string, object?)[]
                 {
                     (HttpMethod.Patch, "/api/cockpit/users/{userId:guid}/role", new { roleId = await InstanceMemberRoleAsync() }),
                     (HttpMethod.Patch, "/api/cockpit/users/{userId:guid}/status", new { disabled = false }),
                     (HttpMethod.Get, "/api/cockpit/users/{userId:guid}/security", null),
                     (HttpMethod.Post, "/api/cockpit/users/{userId:guid}/security/recover", new { reason = "lost" }),
                     (HttpMethod.Post, "/api/cockpit/users/{userId:guid}/identities/{identityId:guid}/remove", null),
                 })
        {
            var path = route.Replace("{userId:guid}", id, StringComparison.Ordinal)
                .Replace("{identityId:guid}", Guid.NewGuid().ToString(), StringComparison.Ordinal);
            Assert.Equal(HttpStatusCode.NotFound, await Send(maja, method, route, path, body));
        }

        Assert.Equal(HttpStatusCode.BadRequest,
            await Send(maja, HttpMethod.Post, "/api/cockpit/servers/{serverId:guid}/transfer", $"/api/cockpit/servers/{serverId}/transfer",
                new { userId = placeholder }));
        var overview = (await maja.GetFromJsonAsync<JsonElement>("/api/cockpit/bootstrap")).GetProperty("overview");
        await using (var scope = factory.OpenScope(out var db))
        {
            Assert.Equal(
                await db.ServerMembers.CountAsync(x => x.ServerId == TestData.ServerId),
                overview.GetProperty("userCount").GetInt32());
            Assert.True(await db.Users.AnyAsync(x => x.Id == placeholder && x.Origin == AccountOrigin.Discord));
        }

        // Mentions: a placeholder's name cannot be written as one, and nothing reaches it.
        var sent = await ada.SendAsync(new HttpRequestMessage(HttpMethod.Post, $"/api/channels/{textChannel}/messages")
        {
            Content = JsonContent.Create(new { content = $"@discord:{DiscordAuthor} and @Ghost, hello" }),
            Headers = { { ServerAccess.HeaderName, serverId.ToString() } },
        });
        sent.EnsureSuccessStatusCode();
        await using (var scope = factory.OpenScope(out var db))
        {
            Assert.False(await db.UserNotifications.AnyAsync(x => x.UserId == placeholder));
            Assert.False(await db.ServerMembers.AnyAsync(x => x.UserId == placeholder));
        }

        // What it wrote is shown, and says whose it is.
        var root = DiscordImportNames.MessageId(serverId, "1001");
        (await ada.SendAsync(new HttpRequestMessage(HttpMethod.Put, $"/api/messages/{DiscordImportNames.MessageId(serverId, "1002")}/pin")
        {
            Headers = { { ServerAccess.HeaderName, serverId.ToString() } },
        })).EnsureSuccessStatusCode();
        foreach (var (path, select) in new (string, Func<JsonElement, IEnumerable<JsonElement>>)[]
                 {
                     ($"/api/channels/{textChannel}/messages", x => x.GetProperty("messages").EnumerateArray()),
                     ($"/api/channels/{textChannel}/messages/sync?after=0", x => x.GetProperty("changes").EnumerateArray()
                         .Where(change => change.GetProperty("message").ValueKind == JsonValueKind.Object)
                         .Select(change => change.GetProperty("message"))),
                     ($"/api/channels/{textChannel}/messages/search?query=ghostly", x => x.EnumerateArray()),
                     ($"/api/channels/{textChannel}/messages/index?query=ghostly", x => x.GetProperty("messages").EnumerateArray()),
                     ($"/api/channels/{textChannel}/pins", x => x.GetProperty("messages").EnumerateArray()),
                     ($"/api/channels/{textChannel}/threads/{root}", x => x.GetProperty("messages").EnumerateArray()),
                 })
        {
            var authors = select(await GetAsync(ada, serverId, path))
                .Select(x => x.GetProperty("author"))
                .Where(x => x.GetProperty("id").GetString() == id)
                .ToList();
            Assert.NotEmpty(authors);
            Assert.All(authors, x => Assert.Equal("discord", x.GetProperty("origin").GetString()));
        }

        var threads = await GetAsync(ada, serverId, $"/api/channels/{textChannel}/threads");
        Assert.Equal(id, Assert.Single(threads.EnumerateArray()).GetProperty("root").GetProperty("authorId").GetString());

        // A person reads as a person.
        var me = await ada.GetFromJsonAsync<JsonElement>("/api/session");
        Assert.Equal(JsonValueKind.Null, me.GetProperty("user").GetProperty("origin").ValueKind);

        // Every route that names a person in its path was walked.
        var routes = factory.Services.GetRequiredService<EndpointDataSource>().Endpoints
            .OfType<RouteEndpoint>()
            .Where(x => x.RoutePattern.RawText?.Contains("{userId", StringComparison.Ordinal) == true)
            .SelectMany(x => (x.Metadata.GetMetadata<HttpMethodMetadata>()?.HttpMethods ?? []).Select(method => $"{method} {x.RoutePattern.RawText}"))
            .Order(StringComparer.Ordinal)
            .ToList();
        Assert.Equal(RoutesNamingAPerson.Order(StringComparer.Ordinal), routes);
        Assert.All(RoutesNamingAPerson, route => Assert.Contains(route, walked));
    }

    private const string DiscordAuthor = "1";

    /// <summary>
    /// Imports a channel with a thread, a reply and a pin into a server of
    /// ada's, of which nils is a member, and gives the placeholder a password
    /// so that refusing it proves more than a missing hash.
    /// </summary>
    private async Task<(Guid Server, Guid Placeholder, Guid Text, Guid Voice, Guid Section)> ImportAsync()
    {
        using var ada = await SignInAsync("ada");
        var created = await ada.PostAsJsonAsync("/api/servers", new
        {
            name = "Imported",
            description = (string?)null,
            textChannelName = "general",
            voiceChannelName = "voice",
        });
        created.EnsureSuccessStatusCode();
        var serverId = Guid.Parse((await created.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetString()!);

        var guild = new DiscordFakeGuild()
            .Category("900", "Talk", position: 0)
            .Channel("100", "chat", parent: "900")
            .Member(DiscordAuthor, "jo", nick: "Ghost");
        guild.Message("1001", "100", DiscordAuthor, "a ghostly question", minute: 1);
        guild.Message("1002", "100", DiscordAuthor, "a ghostly answer", minute: 2, replyTo: "1001");
        var bot = guild.Bot();
        var snapshot = await bot.ReadServerAsync(DiscordFakeGuild.Id, CancellationToken.None);

        await using var scope = factory.OpenScope(out var db);
        var writer = new DiscordImportWriter(db, new DiscordTestClock());
        var import = await writer.StartAsync(serverId, Accounts["ada"].Id, snapshot, CancellationToken.None);
        await writer.WriteStructureAsync(import, snapshot, CancellationToken.None);
        var page = new List<MessageSnapshot>();
        await foreach (var message in bot.ReadMessagesAsync("100"))
        {
            page.Add(message);
        }

        await writer.WriteMessagesAsync(import, snapshot, "100", page, null, CancellationToken.None);
        await writer.CompleteChannelAsync(import, "100", CancellationToken.None);

        var placeholder = await db.DiscordImportAccounts.Where(x => x.DiscordId == DiscordAuthor).Select(x => x.UserId).SingleAsync();
        var passwords = scope.ServiceProvider.GetRequiredService<PasswordService>();
        await db.Users.Where(x => x.Id == placeholder)
            .ExecuteUpdateAsync(setters => setters.SetProperty(x => x.PasswordHash, passwords.Hash(Password)));
        var role = await ServerAccess.FindMemberRoleAsync(db, serverId, CancellationToken.None);
        MemberRoles.Admit(db, new ServerMember
        {
            ServerId = serverId,
            UserId = Accounts["nils"].Id,
            RoleId = role!.Id,
            JoinedAt = DateTime.UtcNow,
        }, isBaseRole: true);
        await db.SaveChangesAsync();

        var text = await db.DiscordImportChannels.Where(x => x.ServerId == serverId && x.DiscordId == "100")
            .Select(x => x.ChannelId!.Value).SingleAsync();
        var voice = await db.Channels.Where(x => x.ServerId == serverId && x.Kind == ChannelKind.Voice)
            .Select(x => x.Id).FirstAsync();
        var section = await db.DiscordImportSections.Where(x => x.ServerId == serverId && x.DiscordId == "900")
            .Select(x => x.SectionId!.Value).SingleAsync();
        return (serverId, placeholder, text, voice, section);
    }

    private async Task<Guid> ServerMemberRoleAsync(Guid serverId)
    {
        await using var scope = factory.OpenScope(out var db);
        return (await ServerAccess.FindMemberRoleAsync(db, serverId, CancellationToken.None))!.Id;
    }

    private Task<Guid> InstanceMemberRoleAsync() => ServerMemberRoleAsync(TestData.ServerId);

    private HttpClient Client() => factory.CreateClient(new WebApplicationFactoryClientOptions
    {
        AllowAutoRedirect = false,
        HandleCookies = true,
    });

    private async Task<HttpClient> SignInAsync(string username)
    {
        var client = Client();
        (await client.PostAsJsonAsync("/api/auth/login", new { username, password = Accounts[username].Password }))
            .EnsureSuccessStatusCode();
        return client;
    }

    private static async Task<JsonElement> GetAsync(HttpClient client, Guid serverId, string path)
    {
        using var request = new HttpRequestMessage(HttpMethod.Get, path);
        if (!path.StartsWith("/api/cockpit/", StringComparison.Ordinal))
        {
            request.Headers.Add(ServerAccess.HeaderName, serverId.ToString());
        }

        using var response = await client.SendAsync(request);
        response.EnsureSuccessStatusCode();
        return await response.Content.ReadFromJsonAsync<JsonElement>();
    }
}
