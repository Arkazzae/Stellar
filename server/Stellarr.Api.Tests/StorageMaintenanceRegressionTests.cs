using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Maintenance;
using Stellarr.Api.Storage;

namespace Stellarr.Api.Tests;

public sealed class StorageMaintenanceRegressionTests
{
    [Fact]
    public async Task DeletingOwnMessageDoesNotResetSlowMode()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var client = factory.CreateClient();
        (await client.PostAsJsonAsync("/api/auth/login", new { username = "jo", password = "Corona!2026" })).EnsureSuccessStatusCode();
        await using (var scope = factory.Services.CreateAsyncScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
            await db.Channels.Where(x => x.Id == TestData.MemesChannelId)
                .ExecuteUpdateAsync(setters => setters.SetProperty(x => x.SlowModeSeconds, 3600));
        }
        var path = $"/api/channels/{TestData.MemesChannelId}/messages";
        var first = await client.PostAsJsonAsync(path, new { content = "slow-mode-proof-first" });
        Assert.Equal(HttpStatusCode.Created, first.StatusCode);
        var id = (await first.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetString();
        // Upgrade a pre-watermark database, then run the startup step again
        // after deletion: neither migration nor restart may forget the send.
        await using (var scope = factory.Services.CreateAsyncScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
            await db.Database.ExecuteSqlRawAsync("DROP TRIGGER Messages_PostTime_AfterInsert; DROP TABLE ChannelPostTimes;");
            await DatabaseSchema.PrepareAsync(db, default);
        }
        Assert.Equal(HttpStatusCode.TooManyRequests,
            (await client.PostAsJsonAsync(path, new { content = "held" })).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent,
            (await client.DeleteAsync($"/api/messages/{id}")).StatusCode);
        await using (var scope = factory.Services.CreateAsyncScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
            await DatabaseSchema.PrepareAsync(db, default);
            Assert.True(await db.ChannelPostTimes.AnyAsync(x => x.ChannelId == TestData.MemesChannelId));
        }
        Assert.Equal(HttpStatusCode.TooManyRequests,
            (await client.PostAsJsonAsync(path, new { content = "bypasses-one-hour-wait" })).StatusCode);
    }

    [Theory]
    [InlineData("avatar")]
    [InlineData("banner")]
    [InlineData("icon")]
    public async Task ProtectedProfileImagesDoNotStarveTheOrphanBatch(string kind)
    {
        using var factory = new StellarrWebApplicationFactory(true,
            new Dictionary<string, string?> { ["Maintenance:BatchSize"] = "1" });
        using var client = factory.CreateClient();
        (await client.GetAsync("/health/ready")).EnsureSuccessStatusCode();
        var now = DateTime.UtcNow;
        var avatarId = Guid.NewGuid();
        var orphanId = Guid.NewGuid();
        Guid userId;
        await using (var scope = factory.Services.CreateAsyncScope())
        {
            var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
            var user = await db.Users.OrderBy(x => x.CreatedAt).FirstAsync();
            userId = user.Id;
            var server = await db.Servers.FirstAsync();
            var url = $"/assets/{avatarId}";
            if (kind == "avatar") user.AvatarUrl = url;
            else if (kind == "banner") user.BannerUrl = url;
            else server.IconUrl = url;
            db.ImageAttachments.AddRange(
                Attachment(avatarId, userId, "proof/avatar.bin", now.AddDays(-100)),
                Attachment(orphanId, userId, "proof/orphan.bin", now.AddDays(-90)));
            db.OperationsSettings.Add(new OperationsSettings
            {
                ServerId = await db.Servers.Select(x => x.Id).FirstAsync(),
                AutomaticCleanupEnabled = true,
                OrphanObjectRetentionDays = 1,
                UpdatedAt = now,
            });
            await db.SaveChangesAsync();
            var storage = scope.ServiceProvider.GetRequiredService<IObjectStorage>();
            await storage.PutAsync("proof/avatar.bin", new MemoryStream([1]), default);
            await storage.PutAsync("proof/orphan.bin", new MemoryStream([2]), default);
        }
        await using (var scope = factory.Services.CreateAsyncScope())
        {
            var result = await scope.ServiceProvider.GetRequiredService<InstanceMaintenance>().RunAsync(now, default);
            Assert.Equal(1, result.OrphanAttachments);
            var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
            Assert.False(await db.ImageAttachments.AnyAsync(x => x.Id == orphanId));
            Assert.True(await db.ImageAttachments.AnyAsync(x => x.Id == avatarId));
        }
        await using var verification = factory.Services.CreateAsyncScope();
        Assert.False(await verification.ServiceProvider.GetRequiredService<AppDbContext>().ImageAttachments.AnyAsync(x => x.Id == orphanId));
    }

    private static ImageAttachment Attachment(Guid id, Guid userId, string key, DateTime createdAt) => new()
    {
        Id = id, UploadedById = userId, ObjectKey = key, FileName = "proof.bin",
        ContentType = "application/octet-stream", SizeBytes = 1, CreatedAt = createdAt,
    };
}
