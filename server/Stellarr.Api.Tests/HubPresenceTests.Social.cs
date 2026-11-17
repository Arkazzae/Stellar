using System.Net.Http.Json;
using Microsoft.AspNetCore.SignalR;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Domain;
using Stellarr.Api.Realtime;

namespace Stellarr.Api.Tests;

public sealed partial class HubPresenceTests
{
    [Theory]
    [InlineData("friend")]
    [InlineData("conversation")]
    [InlineData("both")]
    public async Task SocialPeersReceivePresenceWithoutASharedServer(string relationship)
    {
        using var isolated = new StellarrWebApplicationFactory();
        var harness = new HubPresenceTests(isolated);
        await AddSocialPeersAsync(isolated, relationship);
        await using var observer = await harness.BuildAsync("ada");
        await using var actor = await harness.BuildAsync("jo");
        var seen = Listen(observer, "PresenceChanged", Concerns("jo"));
        await StartAsync(observer, "ada");
        await StartAsync(actor, "jo");
        Assert.Equal("online", (await seen.Next()).GetProperty("presence").GetString());

        await actor.InvokeAsync("SetPresence", "idle");
        Assert.Equal("idle", (await seen.Next()).GetProperty("presence").GetString());
        using var account = isolated.CreateClient();
        (await account.PostAsJsonAsync("/api/auth/login", new { username = "jo", password = Accounts["jo"].Password }))
            .EnsureSuccessStatusCode();
        (await account.PatchAsJsonAsync("/api/me/preferences", new { showPresence = false })).EnsureSuccessStatusCode();
        Assert.Equal("offline", (await seen.Next()).GetProperty("presence").GetString());
        (await account.PatchAsJsonAsync("/api/me/preferences", new { showPresence = true })).EnsureSuccessStatusCode();
        Assert.Equal("idle", (await seen.Next()).GetProperty("presence").GetString());

        await actor.StopAsync();
        Assert.Equal("offline", (await seen.Next()).GetProperty("presence").GetString());
        await PresenceBarrierAsync(isolated, observer);
        Assert.Equal(0, seen.Pending);
    }

    [Theory]
    [InlineData(true)]
    [InlineData(false)]
    public async Task BlockedSocialPeersDoNotReceivePrivatePresence(bool actorBlocks)
    {
        using var isolated = new StellarrWebApplicationFactory();
        var harness = new HubPresenceTests(isolated);
        await AddSocialPeersAsync(isolated, "both");
        await using (var scope = isolated.OpenScope(out var db))
        {
            db.UserBlocks.Add(new UserBlock
            {
                BlockerId = Accounts[actorBlocks ? "jo" : "ada"].Id,
                BlockedId = Accounts[actorBlocks ? "ada" : "jo"].Id,
                CreatedAt = DateTime.UtcNow,
            });
            await db.SaveChangesAsync();
        }
        await using var observer = await harness.BuildAsync("ada");
        await using var actor = await harness.BuildAsync("jo");
        var seen = Listen(observer, "PresenceChanged", Concerns("jo"));
        await StartAsync(observer, "ada");
        await StartAsync(actor, "jo");
        await actor.InvokeAsync("SetPresence", "dnd");
        await PresenceBarrierAsync(isolated, observer);
        Assert.Equal(0, seen.Pending);
    }

    private static async Task AddSocialPeersAsync(StellarrWebApplicationFactory isolated, string relationship)
    {
        await using var scope = isolated.OpenScope(out var db);
        var actorId = Accounts["jo"].Id;
        var observerId = Accounts["ada"].Id;
        // A server group must not accidentally deliver the tested events.
        await db.ServerMembers.Where(x => x.UserId == actorId).ExecuteDeleteAsync();
        if (relationship is "friend" or "both")
            db.Friendships.Add(new Friendship
            {
                Id = Guid.NewGuid(), UserAId = observerId, UserBId = actorId, CreatedAt = DateTime.UtcNow,
            });
        if (relationship is "conversation" or "both")
            db.DirectConversations.Add(new DirectConversation
            {
                Id = Guid.NewGuid(), UserAId = observerId, UserBId = actorId, CreatedAt = DateTime.UtcNow,
            });
        await db.SaveChangesAsync();
    }

    private static async Task PresenceBarrierAsync(StellarrWebApplicationFactory isolated, HubConnection observer)
    {
        var barrier = Listen(observer, "PresenceProbe");
        await isolated.Services.GetRequiredService<IHubContext<AppHub>>().Clients
            .Group(AppHub.UserGroup(Accounts["ada"].Id)).SendAsync("PresenceProbe", "settled");
        await barrier.Next();
    }
}
