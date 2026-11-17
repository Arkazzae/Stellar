using System.Globalization;
using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Data;

namespace Stellarr.Api.Tests;

public sealed class ProfileUploadQuotaTests
{
    private static readonly byte[] Png = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];

    [Theory]
    [InlineData("Uploads:MaxStoredBytesPerUser", 1L)]
    [InlineData("Uploads:MaxStoredBytes", 1L)]
    [InlineData("Uploads:MinimumFreeDataRootBytes", long.MaxValue)]
    public async Task ProfileImagesRespectTheSameStorageLimitsAsOrdinaryUploads(string setting, long limit)
    {
        using var factory = new StellarrWebApplicationFactory(true, new Dictionary<string, string?>
        {
            ["Uploads:MaxStoredBytesPerUser"] = "0",
            ["Uploads:MaxStoredBytes"] = "0",
            ["Uploads:MinimumFreeDataRootBytes"] = "0",
            [setting] = limit.ToString(CultureInfo.InvariantCulture),
        });
        using var client = factory.CreateClient();
        (await client.PostAsJsonAsync("/api/auth/login", new { username = "maja", password = "Aurora!2026" })).EnsureSuccessStatusCode();
        using (var ordinary = Form())
            Assert.Equal(HttpStatusCode.InsufficientStorage, (await client.PostAsync("/api/uploads", ordinary)).StatusCode);
        using (var avatar = Form())
            Assert.Equal(HttpStatusCode.InsufficientStorage, (await client.PostAsync("/api/me/images/avatar", avatar)).StatusCode);
        using (var banner = Form())
            Assert.Equal(HttpStatusCode.InsufficientStorage, (await client.PostAsync("/api/me/images/banner", banner)).StatusCode);
        using (var icon = Form())
            Assert.Equal(HttpStatusCode.InsufficientStorage, (await client.PostAsync("/api/admin/server/icon", icon)).StatusCode);
        using (var emoji = Form())
        {
            emoji.Add(new StringContent("quota_test"), "shortcode");
            Assert.Equal(HttpStatusCode.InsufficientStorage, (await client.PostAsync("/api/emoji", emoji)).StatusCode);
        }
        await using var scope = factory.Services.CreateAsyncScope();
        var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
        Assert.False(await db.ImageAttachments.AnyAsync(x => x.ObjectKey.StartsWith("profiles/")));
        Assert.False(await db.ServerEmoji.AnyAsync(x => x.Shortcode == "quota_test"));
    }

    [Theory]
    [InlineData("Uploads:MaxStoredBytesPerUser")]
    [InlineData("Uploads:MaxStoredBytes")]
    public async Task StoredEmojiBytesCountTowardsLaterUploadAdmission(string setting)
    {
        using var factory = new StellarrWebApplicationFactory(true, new Dictionary<string, string?>
        {
            ["Uploads:MaxStoredBytesPerUser"] = "0",
            ["Uploads:MaxStoredBytes"] = "0",
            ["Uploads:MinimumFreeDataRootBytes"] = "0",
            [setting] = Png.Length.ToString(CultureInfo.InvariantCulture),
        });
        using var client = factory.CreateClient();
        (await client.PostAsJsonAsync("/api/auth/login", new { username = "maja", password = "Aurora!2026" })).EnsureSuccessStatusCode();
        using (var emoji = Form())
        {
            emoji.Add(new StringContent("quota_test"), "shortcode");
            Assert.Equal(HttpStatusCode.Created, (await client.PostAsync("/api/emoji", emoji)).StatusCode);
        }
        using (var ordinary = Form())
            Assert.Equal(HttpStatusCode.InsufficientStorage, (await client.PostAsync("/api/uploads", ordinary)).StatusCode);
        using (var avatar = Form())
            Assert.Equal(HttpStatusCode.InsufficientStorage, (await client.PostAsync("/api/me/images/avatar", avatar)).StatusCode);
    }

    private static MultipartFormDataContent Form()
    {
        var image = new ByteArrayContent(Png);
        image.Headers.ContentType = new MediaTypeHeaderValue("image/png");
        return new MultipartFormDataContent { { image, "file", "profile.png" } };
    }
}
