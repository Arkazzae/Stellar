using System.Collections.Concurrent;
using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Http.Connections;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;

namespace Stellarr.Api.Tests;

/// <summary>
/// The wallpaper: a picture on the account like the avatar and the banner,
/// but one only its owner ever reads or hears about.
/// </summary>
public sealed class ProfileWallpaperTests
{
    private static readonly byte[] Png = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];

    [Fact]
    public async Task IsSetReplacedAndRemovedOnTheOwnAccount()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var client = factory.CreateClient();
        await LoginAsync(client, "maja", "Aurora!2026");

        var first = await UploadAsync(client);
        Assert.Equal(first, (await JsonAsync(await client.GetAsync("/api/me"))).GetProperty("wallpaperUrl").GetString());
        Assert.Equal(HttpStatusCode.OK, (await client.GetAsync(first)).StatusCode);

        var second = await UploadAsync(client);
        Assert.NotEqual(first, second);
        await using (var scope = factory.Services.CreateAsyncScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
            // The replaced picture is nobody's any more, so it goes at once.
            Assert.False(await db.ImageAttachments.AnyAsync(x => x.Id == AttachmentId(first)));
            Assert.True(await db.ImageAttachments.AnyAsync(x => x.Id == AttachmentId(second)));
        }

        var removed = await client.DeleteAsync("/api/me/images/wallpaper");
        Assert.Equal(HttpStatusCode.OK, removed.StatusCode);
        Assert.Equal(JsonValueKind.Null, (await JsonAsync(removed)).GetProperty("wallpaperUrl").ValueKind);
        await using (var scope = factory.Services.CreateAsyncScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
            Assert.False(await db.ImageAttachments.AnyAsync(x => x.Id == AttachmentId(second)));
        }
    }

    [Fact]
    public async Task IsRefusedOverItsSizeCapWithItsOwnMessage()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var client = factory.CreateClient();
        await LoginAsync(client, "maja", "Aurora!2026");
        var large = new byte[8 * 1024 * 1024 + 1];
        Png.CopyTo(large, 0);

        var response = await client.PostAsync("/api/me/images/wallpaper", Form(large));

        Assert.Equal(HttpStatusCode.RequestEntityTooLarge, response.StatusCode);
        Assert.Contains("8 MB", await response.Content.ReadAsStringAsync());
    }

    [Fact]
    public async Task NobodyElseCanReadItOrLearnItsAddress()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var owner = factory.CreateClient();
        await LoginAsync(owner, "maja", "Aurora!2026");
        var wallpaper = await UploadAsync(owner);
        // The same file as an avatar is everybody's to see; as a wallpaper it is not.
        var avatar = await UploadAsync(owner, "avatar");

        using var other = factory.CreateClient();
        await LoginAsync(other, "ada", "Quasar!2026");
        Assert.Contains(
            (await other.GetAsync(wallpaper)).StatusCode,
            new[] { HttpStatusCode.Forbidden, HttpStatusCode.NotFound });
        Assert.Equal(HttpStatusCode.OK, (await other.GetAsync(avatar)).StatusCode);

        var id = AttachmentId(wallpaper).ToString();
        var bootstrap = await other.GetStringAsync("/api/bootstrap");
        Assert.Contains(AttachmentId(avatar).ToString(), bootstrap);
        Assert.DoesNotContain(id, bootstrap);
        Assert.DoesNotContain(id, await other.GetStringAsync("/api/me"));
    }

    [Fact]
    public void OnlyTheOwnAccountShapeCarriesIt()
    {
        // Every shape the API sends is a record in Contracts: users, members,
        // invite previews, Cockpit's views, the realtime payloads. The account
        // is the only one that may name a wallpaper.
        var carriers = typeof(AccountDto).Assembly.GetTypes()
            .Where(type => type.Namespace == typeof(AccountDto).Namespace)
            .Where(type => type.GetProperties().Any(property => property.Name.Contains("Wallpaper", StringComparison.Ordinal)))
            .Select(type => type.Name);
        Assert.Equal(["AccountDto"], carriers);
    }

    [Fact]
    public async Task IsNotAnnouncedToAnybody()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var owner = factory.CreateClient();
        await LoginAsync(owner, "maja", "Aurora!2026");

        using var watcher = factory.CreateClient(new Microsoft.AspNetCore.Mvc.Testing.WebApplicationFactoryClientOptions
        {
            HandleCookies = false,
        });
        var login = await watcher.PostAsJsonAsync("/api/auth/login", new { username = "ada", password = "Quasar!2026" });
        login.EnsureSuccessStatusCode();
        var cookie = string.Join("; ", login.Headers.GetValues("Set-Cookie").Select(x => x.Split(';', 2)[0]));
        await using var connection = new HubConnectionBuilder()
            .WithUrl("https://localhost/hubs/app", options =>
            {
                options.Headers["Cookie"] = cookie;
                options.Transports = HttpTransportType.LongPolling;
                options.HttpMessageHandlerFactory = _ => factory.Server.CreateHandler();
            })
            .Build();
        var updates = new ConcurrentQueue<JsonElement>();
        var announced = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        connection.On<JsonElement>("UserUpdated", payload =>
        {
            updates.Enqueue(payload);
            announced.TrySetResult();
        });
        await connection.StartAsync();

        await UploadAsync(owner);
        Assert.Equal(HttpStatusCode.OK, (await owner.DeleteAsync("/api/me/images/wallpaper")).StatusCode);
        await Task.Delay(TimeSpan.FromMilliseconds(500));
        Assert.Empty(updates);

        // The avatar, by contrast, is news to everybody who sees the person.
        await UploadAsync(owner, "avatar");
        await announced.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.All(updates, update => Assert.False(update.TryGetProperty("wallpaperUrl", out _)));
        await connection.StopAsync();
    }

    private static Guid AttachmentId(string assetUrl) => Guid.Parse(assetUrl["/assets/".Length..]);

    private static async Task<string> UploadAsync(HttpClient client, string kind = "wallpaper")
    {
        var response = await client.PostAsync($"/api/me/images/{kind}", Form(Png));
        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        return (await JsonAsync(response)).GetProperty($"{kind}Url").GetString()!;
    }

    private static MultipartFormDataContent Form(byte[] bytes)
    {
        var image = new ByteArrayContent(bytes);
        image.Headers.ContentType = new MediaTypeHeaderValue("image/png");
        return new MultipartFormDataContent { { image, "file", "wallpaper.png" } };
    }

    private static async Task LoginAsync(HttpClient client, string username, string password) =>
        (await client.PostAsJsonAsync("/api/auth/login", new { username, password })).EnsureSuccessStatusCode();

    private static async Task<JsonElement> JsonAsync(HttpResponseMessage response) =>
        await response.Content.ReadFromJsonAsync<JsonElement>();
}
