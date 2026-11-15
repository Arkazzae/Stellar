using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// What keeps the channel index current after a message is stored: the
/// triggers on the tables a message is made of.
/// </summary>
public sealed class MessageIndexSchemaTests
{
    [Fact]
    public async Task AnEmbedAddedLaterIsSearchableAndLeavesWithItsRemoval()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var client = await SignInAsync(factory);
        var token = NewToken();
        var cardToken = NewToken();
        var messageId = await SendAsync(client, $"zwykły tekst {token}");
        Assert.DoesNotContain(messageId, await SearchAsync(client, cardToken));
        Assert.DoesNotContain(messageId, await ListAsync(client, "links"));

        // Cards are built after the message went out, by a worker of their own.
        var embedId = Guid.NewGuid();
        await WithContextAsync(factory, async db =>
        {
            db.MessageEmbeds.Add(new MessageEmbed
            {
                Id = embedId,
                MessageId = Guid.Parse(messageId),
                Url = $"https://cards.example/{cardToken}",
                Kind = EmbedKind.Link,
                Provider = "cards",
                Domain = "cards.example",
                Title = $"Karta {cardToken}",
                Position = 0,
                CreatedAt = DateTime.UtcNow,
            });
            await db.SaveChangesAsync();
        });
        Assert.Contains(messageId, await SearchAsync(client, cardToken));
        Assert.Contains(messageId, await ListAsync(client, "links"));

        await WithContextAsync(factory, async db =>
            await db.MessageEmbeds.Where(x => x.Id == embedId).ExecuteDeleteAsync());
        Assert.DoesNotContain(messageId, await SearchAsync(client, cardToken));
        Assert.DoesNotContain(messageId, await ListAsync(client, "links"));
        Assert.Contains(messageId, await SearchAsync(client, token));
    }

    [Fact]
    public async Task RenamingTheAuthorMakesTheirMessagesFindableByTheNewName()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var client = await SignInAsync(factory);
        var messageId = await SendAsync(client, $"podpisana {NewToken()}");
        var newName = $"Zorka{NewToken()}";
        Assert.DoesNotContain(messageId, await SearchAsync(client, newName));

        await WithContextAsync(factory, async db =>
            await db.Users.Where(x => x.Username == "maja")
                .ExecuteUpdateAsync(x => x.SetProperty(user => user.DisplayName, newName)));

        Assert.Contains(messageId, await SearchAsync(client, newName));
    }

    private static async Task<HttpClient> SignInAsync(StellarrWebApplicationFactory factory)
    {
        var client = factory.CreateClient(new WebApplicationFactoryClientOptions
        {
            AllowAutoRedirect = false,
            HandleCookies = true,
        });
        (await client.PostAsJsonAsync("/api/auth/login", new { username = "maja", password = "Aurora!2026" }))
            .EnsureSuccessStatusCode();
        return client;
    }

    private static async Task WithContextAsync(StellarrWebApplicationFactory factory, Func<AppDbContext, Task> action)
    {
        await using var scope = factory.Services.CreateAsyncScope();
        await action(scope.ServiceProvider.GetRequiredService<AppDbContext>());
    }

    private static string NewToken() => Guid.NewGuid().ToString("N")[..10];

    private static async Task<string> SendAsync(HttpClient client, string content)
    {
        var response = await client.PostAsJsonAsync(
            $"/api/channels/{TestData.GeneralChannelId}/messages",
            new { content, attachmentIds = Array.Empty<Guid>() });
        response.EnsureSuccessStatusCode();
        return (await response.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetString()!;
    }

    private static async Task<string?[]> SearchAsync(HttpClient client, string query)
    {
        var found = await client.GetFromJsonAsync<JsonElement>(
            $"/api/channels/{TestData.GeneralChannelId}/messages/search?query={Uri.EscapeDataString(query)}");
        return found.EnumerateArray().Select(x => x.GetProperty("id").GetString()).ToArray();
    }

    private static async Task<string?[]> ListAsync(HttpClient client, string kind)
    {
        var page = await client.GetFromJsonAsync<JsonElement>(
            $"/api/channels/{TestData.GeneralChannelId}/messages/index?kind={kind}&limit=100");
        return page.GetProperty("messages").EnumerateArray().Select(x => x.GetProperty("id").GetString()).ToArray();
    }
}
