using System.Buffers.Binary;
using System.Buffers.Text;
using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Domain;
using Stellarr.Api.Messaging;

namespace Stellarr.Api.Tests;

public sealed class HistoryKeysetTests
{
    private static readonly Guid AuthorId = Guid.Parse("30000000-0000-0000-0000-000000000001");
    private static readonly Guid PeerId = Guid.Parse("30000000-0000-0000-0000-000000000002");

    [Theory]
    [InlineData(false, false, false)]
    [InlineData(false, false, true)]
    [InlineData(false, true, false)]
    [InlineData(false, true, true)]
    [InlineData(true, false, false)]
    [InlineData(true, false, true)]
    [InlineData(true, true, false)]
    [InlineData(true, true, true)]
    public async Task DeletingADeliveredRowNeverSkipsUnreadRows(bool direct, bool tiedTimes, bool deleteAnchor)
    {
        using var factory = new StellarrWebApplicationFactory();
        using var client = await SignInAsync(factory);
        var scopeId = await SeedAsync(factory, direct, tiedTimes);
        var first = await ReadAsync(client, direct, scopeId);
        var seen = Ids(first).ToList();
        Assert.Equal(new[] { Id(5), Id(6) }, seen);

        await using (var scope = factory.OpenScope(out var db))
        {
            var deleted = Id(deleteAnchor ? 5 : 6);
            if (direct)
            {
                await db.DirectMessages.Where(x => x.Id == deleted).ExecuteDeleteAsync();
                db.DirectMessages.Add(new DirectMessage
                {
                    Id = Id(7), ConversationId = scopeId, AuthorId = AuthorId, Content = "new arrival", CreatedAt = DateTime.UtcNow,
                });
            }
            else
            {
                await db.Messages.Where(x => x.Id == deleted).ExecuteDeleteAsync();
                db.Messages.Add(new Message
                {
                    Id = Id(7), ChannelId = scopeId, AuthorId = AuthorId, Content = "new arrival", CreatedAt = DateTime.UtcNow,
                });
            }
            await db.SaveChangesAsync();
        }

        var cursor = first.GetProperty("nextCursor").GetString();
        for (var remaining = 5; cursor is not null && remaining > 0; remaining--)
        {
            var page = await ReadAsync(client, direct, scopeId, cursor);
            seen.AddRange(Ids(page));
            cursor = page.GetProperty("nextCursor").GetString();
        }
        Assert.Null(cursor);
        Assert.Equal(Enumerable.Range(1, 6).Select(Id), seen.Order());
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task LegacyOffsetsRestartTheirSnapshotAndIssueKeysetContinuations(bool direct)
    {
        using var factory = new StellarrWebApplicationFactory();
        using var client = await SignInAsync(factory);
        var scopeId = await SeedAsync(factory, direct, true);
        var cursor = LegacyCursor(scopeId, DateTime.UtcNow, 2);
        var first = await ReadAsync(client, direct, scopeId, cursor);
        Assert.Equal(new[] { Id(5), Id(6) }, Ids(first));
        Assert.True(HistoryCursor.TryDecode(first.GetProperty("nextCursor").GetString()!, out var next));
        Assert.Equal(Id(5), next.BeforeId);

        var second = await ReadAsync(client, direct, scopeId, next.Encode());
        Assert.Equal(new[] { Id(3), Id(4) }, Ids(second));
        using var wrongScope = await client.GetAsync(Url(direct, scopeId, LegacyCursor(Guid.NewGuid(), DateTime.UtcNow, 2)));
        Assert.Equal(HttpStatusCode.BadRequest, wrongScope.StatusCode);
    }

    [Fact]
    public void CursorRejectsUnknownVersionsAndAnAnchorPastTheSnapshot()
    {
        var snapshot = DateTime.UtcNow;
        var cursor = new HistoryCursor(Guid.NewGuid(), snapshot, snapshot.AddSeconds(-1), Id(1));
        Assert.True(HistoryCursor.TryDecode(cursor.Encode(), out var decoded));
        Assert.Equal(cursor, decoded);
        Assert.False(HistoryCursor.TryDecode((cursor with { BeforeCreatedAt = snapshot.AddSeconds(1) }).Encode(), out _));
        var bytes = Base64Url.DecodeFromChars(cursor.Encode());
        bytes[0] = 2;
        Assert.False(HistoryCursor.TryDecode(Base64Url.EncodeToString(bytes), out _));
        Assert.False(HistoryCursor.TryDecode("malformed", out _));
        Assert.False(HistoryCursor.TryDecode(LegacyCursor(cursor.ScopeId, snapshot, -1), out _));
    }

    private static async Task<Guid> SeedAsync(StellarrWebApplicationFactory factory, bool direct, bool tiedTimes)
    {
        await using var scope = factory.OpenScope(out var db);
        var scopeId = direct ? Guid.NewGuid() : TestData.GeneralChannelId;
        if (direct)
        {
            db.DirectConversations.Add(new DirectConversation
            {
                Id = scopeId, UserAId = AuthorId, UserBId = PeerId, CreatedAt = DateTime.UtcNow.AddHours(-1),
            });
        }
        else
        {
            await db.Messages.Where(x => x.ChannelId == scopeId).ExecuteDeleteAsync();
        }
        var start = DateTime.UtcNow.AddMinutes(-1);
        for (var i = 1; i <= 6; i++)
        {
            var time = start.AddSeconds(tiedTimes ? 0 : i);
            if (direct)
            {
                db.DirectMessages.Add(new DirectMessage
                {
                    Id = Id(i), ConversationId = scopeId, AuthorId = AuthorId, Content = $"message {i}", CreatedAt = time,
                });
            }
            else
            {
                db.Messages.Add(new Message
                {
                    Id = Id(i), ChannelId = scopeId, AuthorId = AuthorId, Content = $"message {i}", CreatedAt = time,
                });
            }
        }
        await db.SaveChangesAsync();
        return scopeId;
    }

    private static async Task<HttpClient> SignInAsync(StellarrWebApplicationFactory factory)
    {
        var client = factory.CreateClient();
        (await client.PostAsJsonAsync("/api/auth/login", new { username = "maja", password = "Aurora!2026" })).EnsureSuccessStatusCode();
        return client;
    }

    private static Guid Id(int i) => Guid.Parse($"a0000000-0000-0000-0000-{i:D12}");
    private static IEnumerable<Guid> Ids(JsonElement page) => page.GetProperty("messages").EnumerateArray()
        .Select(x => Guid.Parse(x.GetProperty("id").GetString()!));
    private static string Url(bool direct, Guid scopeId, string? cursor = null) =>
        $"/api/{(direct ? "social/conversations" : "channels")}/{scopeId}/messages?limit=2" +
        (cursor is null ? "" : "&cursor=" + Uri.EscapeDataString(cursor));
    private static Task<JsonElement> ReadAsync(HttpClient client, bool direct, Guid scopeId, string? cursor = null) =>
        client.GetFromJsonAsync<JsonElement>(Url(direct, scopeId, cursor));

    private static string LegacyCursor(Guid scopeId, DateTime snapshot, int offset)
    {
        Span<byte> bytes = stackalloc byte[28];
        scopeId.TryWriteBytes(bytes[..16]);
        BinaryPrimitives.WriteInt64BigEndian(bytes[16..24], snapshot.Ticks);
        BinaryPrimitives.WriteInt32BigEndian(bytes[24..], offset);
        return Base64Url.EncodeToString(bytes);
    }
}
