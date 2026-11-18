using System.Net.Http.Json;
using System.Text.Json;

namespace Stellarr.Api.Tests;

/// <summary>
/// Files a channel under a section, or under none, the way the rail's channel
/// menu does: the whole channel order, unchanged, with the move.
/// </summary>
internal static class ChannelSections
{
    internal static async Task<HttpResponseMessage> FileAsync(HttpClient client, object channelId, object? sectionId)
    {
        using var bootstrap = JsonDocument.Parse(await client.GetStringAsync("/api/bootstrap"));
        var order = bootstrap.RootElement.GetProperty("channels").EnumerateArray()
            .Select(x => x.GetProperty("id").GetString())
            .ToArray();
        return await client.PutAsJsonAsync(
            "/api/admin/channels/order",
            new { channelIds = order, move = new { channelId, sectionId } });
    }
}
