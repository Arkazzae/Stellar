using System.Globalization;
using System.Net;
using System.Text.Json;
using System.Text.Json.Nodes;
using Stellarr.Api.Integrations.Discord;

namespace Stellarr.Api.Tests;

/// <summary>
/// A Discord server as data, answering the bot's reads as Discord does: its
/// channels, people and threads, and each channel's history, newest first,
/// paged by <c>before</c>. Tests read it with the real reader, so the writer is
/// tested against what the reader really produces.
/// </summary>
internal sealed class DiscordFakeGuild
{
    internal const string Id = "41771983423143936";

    private readonly List<JsonObject> _channels = [];
    private readonly List<JsonObject> _roles = [];
    private readonly List<JsonObject> _members = [];
    private readonly List<JsonObject> _threads = [];
    private readonly Dictionary<string, List<JsonObject>> _messages = new(StringComparer.Ordinal);
    private readonly Dictionary<string, List<(string PinnedAt, JsonObject Message)>> _pins = new(StringComparer.Ordinal);

    public DiscordFakeGuild(string name = "Krew")
    {
        Name = name;
        // @everyone sees every channel unless an overwrite says otherwise.
        _roles.Add(new JsonObject { ["id"] = Id, ["name"] = "@everyone", ["position"] = 0, ["permissions"] = "1024" });
    }

    public string Name { get; }

    public DiscordFakeGuild Role(string id, string name)
    {
        _roles.Add(new JsonObject { ["id"] = id, ["name"] = name, ["position"] = _roles.Count, ["permissions"] = "0" });
        return this;
    }

    public DiscordFakeGuild Category(string id, string name, int position)
    {
        _channels.Add(new JsonObject { ["id"] = id, ["type"] = 4, ["name"] = name, ["position"] = position });
        return this;
    }

    /// <summary>A channel of Discord's type: 0 text, 2 voice, 5 announcements, 13 stage, 15 forum, 16 media.</summary>
    public DiscordFakeGuild Channel(
        string id,
        string name,
        int type = 0,
        string? parent = null,
        int position = 0,
        string? topic = null,
        int slowMode = 0,
        int userLimit = 0,
        bool hidden = false)
    {
        var channel = new JsonObject
        {
            ["id"] = id,
            ["type"] = type,
            ["name"] = name,
            ["position"] = position,
            ["parent_id"] = parent,
            ["topic"] = topic,
            ["rate_limit_per_user"] = slowMode,
            ["user_limit"] = userLimit,
        };
        if (hidden)
        {
            channel["permission_overwrites"] = new JsonArray(
                new JsonObject { ["id"] = Id, ["type"] = 0, ["allow"] = "0", ["deny"] = "1024" });
        }

        _channels.Add(channel);
        _messages.TryAdd(id, []);
        return this;
    }

    public DiscordFakeGuild Member(string id, string username, string? globalName = null, string? nick = null)
    {
        _members.Add(new JsonObject
        {
            ["user"] = new JsonObject { ["id"] = id, ["username"] = username, ["global_name"] = globalName },
            ["nick"] = nick,
            ["roles"] = new JsonArray(),
            ["joined_at"] = "2023-01-01T00:00:00+00:00",
        });
        return this;
    }

    /// <summary>A thread: 11 public, 12 private. A public thread has the id of the message it grew from.</summary>
    public DiscordFakeGuild Thread(string id, string parent, string name, int type = 11)
    {
        _threads.Add(new JsonObject
        {
            ["id"] = id,
            ["type"] = type,
            ["parent_id"] = parent,
            ["name"] = name,
            ["thread_metadata"] = new JsonObject { ["archived"] = false, ["locked"] = false },
        });
        _messages.TryAdd(id, []);
        return this;
    }

    /// <summary>Adds a message to a channel's history; <paramref name="minute"/> is when, in minutes after 2024-01-01.</summary>
    public JsonObject Message(
        string id,
        string channel,
        string authorId,
        string content,
        int minute,
        string username = "jo",
        string? globalName = null,
        int type = 0,
        string? replyTo = null,
        bool pinned = false,
        int? editedMinute = null,
        bool bot = false,
        string? webhookId = null,
        int attachments = 0,
        bool sticker = false,
        bool embed = false,
        string? forwarded = null)
    {
        var message = new JsonObject
        {
            ["id"] = id,
            ["channel_id"] = channel,
            ["type"] = replyTo is null ? type : 19,
            ["content"] = content,
            ["timestamp"] = At(minute),
            ["edited_timestamp"] = editedMinute is { } edited ? At(edited) : null,
            ["pinned"] = pinned,
            ["webhook_id"] = webhookId,
            ["author"] = new JsonObject
            {
                ["id"] = webhookId ?? authorId,
                ["username"] = username,
                ["global_name"] = globalName,
                ["bot"] = bot || webhookId is not null,
            },
            ["attachments"] = new JsonArray([.. Enumerable.Range(0, attachments).Select(index => (JsonNode)new JsonObject
            {
                ["id"] = $"{id}{index}",
                ["filename"] = $"photo{index}.png",
                ["size"] = 1024,
                ["url"] = $"https://cdn.discordapp.com/attachments/{channel}/{id}{index}/photo{index}.png",
                ["content_type"] = "image/png",
            })]),
            ["embeds"] = embed
                ? new JsonArray(new JsonObject { ["type"] = "rich", ["title"] = "Status", ["description"] = "All good" })
                : new JsonArray(),
            ["sticker_items"] = sticker
                ? new JsonArray(new JsonObject { ["id"] = "749054660769218631", ["name"] = "Wave", ["format_type"] = 1 })
                : new JsonArray(),
        };
        if (replyTo is not null)
        {
            message["message_reference"] = new JsonObject { ["type"] = 0, ["message_id"] = replyTo, ["channel_id"] = channel };
        }
        else if (forwarded is not null)
        {
            message["message_reference"] = new JsonObject { ["type"] = 1, ["message_id"] = "1", ["channel_id"] = channel };
            message["message_snapshots"] = new JsonArray(new JsonObject
            {
                ["message"] = new JsonObject { ["content"] = forwarded, ["attachments"] = new JsonArray() },
            });
        }

        _messages[channel].Add(message);
        return message;
    }

    public void Pin(JsonObject message, int minute) =>
        (_pins.TryGetValue(message["channel_id"]!.GetValue<string>(), out var list) ? list
            : _pins[message["channel_id"]!.GetValue<string>()] = []).Add((At(minute), message));

    /// <summary>Removes a message, as somebody deleting it on Discord would.</summary>
    public void Delete(string channel, string id) => _messages[channel].RemoveAll(x => x["id"]!.GetValue<string>() == id);

    internal static string At(int minute) =>
        new DateTimeOffset(2024, 1, 1, 0, 0, 0, TimeSpan.Zero).AddMinutes(minute).ToString("O", CultureInfo.InvariantCulture);

    internal static DateTime Utc(int minute) => new DateTime(2024, 1, 1, 0, 0, 0, DateTimeKind.Utc).AddMinutes(minute);

    public DiscordFakeApi Api() => new(Respond);

    public DiscordBotReader Bot() => DiscordClientTests.Bot(Api());

    private HttpResponseMessage Respond(SentRequest request)
    {
        var path = request.Uri.AbsolutePath.Replace("/api/v10/", "", StringComparison.Ordinal);
        var parts = path.Split('/');
        return path switch
        {
            $"guilds/{Id}" => Json(new JsonObject { ["id"] = Id, ["name"] = Name }),
            $"guilds/{Id}/channels" => Json(new JsonArray([.. _channels.Select(Copy)])),
            $"guilds/{Id}/roles" => Json(new JsonArray([.. _roles.Select(Copy)])),
            $"guilds/{Id}/emojis" or $"guilds/{Id}/stickers" => Json(new JsonArray()),
            $"guilds/{Id}/members" => Json(new JsonArray([.. _members
                .Where(x => request.Query("after") is not { } after ||
                    Compare(x["user"]!["id"]!.GetValue<string>(), after) > 0)
                .Select(Copy)])),
            $"guilds/{Id}/threads/active" => Json(new JsonObject { ["threads"] = new JsonArray([.. _threads.Select(Copy)]) }),
            _ when parts is ["channels", _, "threads", "archived", _] =>
                Json(new JsonObject { ["has_more"] = false, ["threads"] = new JsonArray() }),
            _ when parts is ["channels", var channel, "messages", "pins"] =>
                Json(new JsonObject
                {
                    ["has_more"] = false,
                    ["items"] = new JsonArray([.. (_pins.GetValueOrDefault(channel) ?? [])
                        .OrderByDescending(x => x.PinnedAt, StringComparer.Ordinal)
                        .Select(x => (JsonNode)new JsonObject { ["pinned_at"] = x.PinnedAt, ["message"] = Copy(x.Message) })]),
                }),
            _ when parts is ["channels", var channel, "messages"] => Json(History(channel, request)),
            _ => throw new InvalidOperationException($"Not expected: {path}"),
        };
    }

    private JsonArray History(string channel, SentRequest request)
    {
        var limit = int.Parse(request.Query("limit")!, CultureInfo.InvariantCulture);
        var before = request.Query("before");
        return new JsonArray([.. (_messages.GetValueOrDefault(channel) ?? [])
            .Where(x => before is null || Compare(x["id"]!.GetValue<string>(), before) < 0)
            .OrderByDescending(x => x["id"]!.GetValue<string>(), Comparer<string>.Create(Compare))
            .Take(limit)
            .Select(Copy)]);
    }

    private static int Compare(string left, string right) =>
        ulong.Parse(left, CultureInfo.InvariantCulture).CompareTo(ulong.Parse(right, CultureInfo.InvariantCulture));

    private static JsonNode Copy(JsonObject node) => node.DeepClone();

    private static HttpResponseMessage Json(JsonNode body) => DiscordFakeApi.Json(body.ToJsonString(), HttpStatusCode.OK);
}
