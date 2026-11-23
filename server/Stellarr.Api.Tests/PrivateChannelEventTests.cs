using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.SignalR;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Realtime;

namespace Stellarr.Api.Tests;

/// <summary>
/// An event about a private channel reaches the people who see it and
/// nobody else on the server, decided when it is sent: whoever loses view
/// stops receiving with the next event and is told, and whoever gains it
/// starts. Maja owns the server and sees everything; Jo and Nils see none of
/// the private places (see <see cref="PrivateChannelScenario"/>).
/// </summary>
public sealed class PrivateChannelEventTests
{
    private static readonly Guid Hidden = PrivateChannelScenario.Hidden;
    private static readonly Guid Open = PrivateChannelScenario.Open;
    private static readonly string[] ViewOnly = ["viewChannel"];
    private static readonly string[] NoNames = [];
    private static readonly string[] AdministratorOnly = ["administrator"];
    private static readonly string[] EverythingButView = ["sendMessages", "attachFiles", "addReactions", "connect"];

    public static TheoryData<string> Kinds() =>
        new("MessageCreated", "MessageUpdated", "MessageDeleted", "MessageReactionChanged", "MessageEmbedsUpdated",
            "TypingChanged", "VoiceParticipantChanged", "pin");

    [Theory]
    [MemberData(nameof(Kinds))]
    public async Task AnEventAboutAPrivateChannelReachesOnlyThoseWhoSeeIt(string kind)
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        var method = kind == "pin" ? "MessageUpdated" : kind;
        await using var owner = await state.ListenAsync("maja", method);
        await using var jo = await state.ListenAsync("jo", method);
        await using var nils = await state.ListenAsync("nils", method);
        await using var ada = await state.ListenAsync("ada");
        await using var ilya = await state.ListenAsync("ilya");
        using var ilyaClient = await state.SignedInAsync("ilya");

        // Ada sees the private channel; Ilya, like Jo and Nils, only the open one.
        await ActAsync(kind, state, state.Ada, ada, PrivateChannelScenario.Hidden, PrivateChannelScenario.HiddenVoice);
        Assert.Equal(
            kind == "VoiceParticipantChanged" ? PrivateChannelScenario.HiddenVoice.ToString() : Hidden.ToString(),
            ChannelOf(await owner.NextAsync(method)));

        // The same in the open channel reaches them, and is the first thing they get.
        await ActAsync(kind, state, ilyaClient, ilya, Open, PrivateChannelScenario.OpenVoice);
        var open = kind == "VoiceParticipantChanged" ? PrivateChannelScenario.OpenVoice.ToString() : Open.ToString();
        Assert.Equal(open, ChannelOf(await jo.NextAsync(method)));
        Assert.Equal(open, ChannelOf(await nils.NextAsync(method)));
    }

    /// <summary>Does one thing of the kind in the channel, or the voice room, as the given member.</summary>
    private static async Task ActAsync(
        string kind, PrivateChannelScenario state, HttpClient client, PrivateChannelScenario.HubLog hub, Guid channel, Guid room)
    {
        switch (kind)
        {
            case "TypingChanged":
                await hub.Connection.InvokeAsync("SendTyping", channel.ToString());
                return;
            case "VoiceParticipantChanged":
                await hub.Connection.InvokeAsync("JoinVoice", room.ToString());
                return;
        }

        var posted = await client.PostAsJsonAsync($"/api/channels/{channel}/messages", new { content = "nowa wiadomość" });
        Assert.Equal(HttpStatusCode.Created, posted.StatusCode);
        var id = (await posted.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetString();
        if (kind is "MessageCreated" or "MessageEmbedsUpdated")
        {
            // Posting is the one, and the embeds follow every post, links or not.
            return;
        }

        // Somebody who manages messages, so a pin is allowed as well.
        var manager = state.Owner;
        var response = kind switch
        {
            "MessageUpdated" => await client.PatchAsJsonAsync($"/api/messages/{id}", new { content = "poprawiona" }),
            "MessageDeleted" => await client.DeleteAsync($"/api/messages/{id}"),
            "MessageReactionChanged" => await client.PutAsync($"/api/messages/{id}/reactions/{Uri.EscapeDataString("🎉")}", null),
            _ => await manager.PutAsync($"/api/messages/{id}/pin", null),
        };
        Assert.True(response.IsSuccessStatusCode, $"{kind}: {response.StatusCode}");
    }

    private static string? ChannelOf(JsonElement payload) => payload.GetProperty("channelId").GetString();

    [Fact]
    public async Task ALayoutOrdersOnlyWhatEachMemberSees()
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        var openSection = (await (await state.Owner.PostAsJsonAsync("/api/admin/sections", new { name = "Otwarta" }))
            .Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetString();
        await using var jo = await state.ListenAsync("jo", "ChannelLayoutChanged", "SectionOrderChanged");
        await using var ada = await state.ListenAsync("ada", "ChannelLayoutChanged", "SectionOrderChanged");

        var order = await state.WithDbAsync(db => Task.FromResult(db.Channels.Where(x => x.ServerId == TestData.ServerId)
            .OrderByDescending(x => x.Position).Select(x => x.Id).ToArray()));
        Assert.Equal(HttpStatusCode.NoContent,
            (await state.Owner.PutAsJsonAsync("/api/admin/channels/order", new { channelIds = order })).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await state.Owner.PutAsJsonAsync(
            "/api/admin/sections/order", new { sectionIds = new[] { Guid.Parse(openSection!), state.HiddenSection } })).StatusCode);

        string[] Channels(JsonElement layout) =>
            layout.GetProperty("channels").EnumerateArray().Select(x => x.GetProperty("id").GetString()!).ToArray();
        string[] Sections(JsonElement layout) =>
            layout.GetProperty("sectionIds").EnumerateArray().Select(x => x.GetString()!).ToArray();
        var seenByAda = Channels(await ada.NextAsync("ChannelLayoutChanged"));
        Assert.Equal(order.Select(x => x.ToString()), seenByAda);
        var seenByJo = Channels(await jo.NextAsync("ChannelLayoutChanged"));
        Assert.Equal(
            order.Where(x => x != Hidden && x != PrivateChannelScenario.HiddenVoice && x != state.SectionChannel)
                .Select(x => x.ToString()),
            seenByJo);
        Assert.Equal([openSection!, state.HiddenSection.ToString()], Sections(await ada.NextAsync("SectionOrderChanged")));
        Assert.Equal([openSection!], Sections(await jo.NextAsync("SectionOrderChanged")));
    }

    [Fact]
    public async Task WhoLosesViewIsToldAndStopsReceivingAtOnceAndWhoGainsItStarts()
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        await using var jo = await state.ListenAsync("jo", "MessageCreated", "ChannelAccessChanged");
        await using var ilya = await state.ListenAsync("ilya", "ChannelAccessChanged");
        var allowJo = $"/api/admin/channels/{Hidden}/permissions/members/{PrivateChannelScenario.JoId}";

        // Allowed: told, and the next message reaches him.
        Assert.Equal(HttpStatusCode.OK, (await state.Owner.PutAsJsonAsync(
            allowJo, new { allow = ViewOnly, deny = NoNames })).StatusCode);
        Assert.Equal(TestData.ServerId.ToString(),
            (await jo.NextAsync("ChannelAccessChanged")).GetProperty("serverId").GetString());
        await PostAsync(state.Ada, Hidden, "widzisz");
        Assert.Equal("widzisz", (await jo.NextAsync("MessageCreated")).GetProperty("content").GetString());

        // Cleared: told, and the next message passes him by.
        Assert.Equal(HttpStatusCode.NoContent, (await state.Owner.DeleteAsync(allowJo)).StatusCode);
        await jo.NextAsync("ChannelAccessChanged");
        await PostAsync(state.Ada, Hidden, "już nie");
        await PostAsync(state.Ada, Open, "tutaj tak");
        Assert.Equal("tutaj tak", (await jo.NextAsync("MessageCreated")).GetProperty("content").GetString());

        // Nobody whose view did not change hears of either.
        await ilya.SettleAsync();
        Assert.Equal(0, ilya.Pending("ChannelAccessChanged"));
    }

    public static TheoryData<string> Changes() =>
        new("role given", "role taken", "role permissions", "base role permissions", "role deleted", "channel moved", "role override");

    [Theory]
    [MemberData(nameof(Changes))]
    public async Task EveryKindOfChangeThatAltersAViewTellsWhoseItAltered(string change)
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        await using var jo = await state.ListenAsync("jo", "ChannelAccessChanged", "MessageCreated");
        await using var owner = await state.ListenAsync("maja", "ChannelAccessChanged");
        var gift = await RoleAsync(state, "Wgląd", allowedAt: Hidden);
        var roles = $"/api/admin/members/{PrivateChannelScenario.JoId}/roles";
        Guid? empty = null;
        if (change is "role taken" or "base role permissions" or "role deleted")
        {
            (await state.Owner.PutAsJsonAsync(roles, new { roleIds = new[] { gift } })).EnsureSuccessStatusCode();
            await jo.NextAsync("ChannelAccessChanged");
        }
        else if (change == "role permissions")
        {
            // A role that grants nothing changes nothing Jo sees, so he hears nothing of it.
            empty = await RoleAsync(state, "Pusta", allowedAt: null);
            (await state.Owner.PutAsJsonAsync(roles, new { roleIds = new[] { empty.Value } })).EnsureSuccessStatusCode();
        }

        var response = change switch
        {
            "role given" => await state.Owner.PutAsJsonAsync(roles, new { roleIds = new[] { gift } }),
            "role taken" => await state.Owner.PutAsJsonAsync(roles, new { roleIds = Array.Empty<Guid>() }),
            "role permissions" => await state.Owner.PatchAsJsonAsync(
                $"/api/admin/roles/{empty}", new { permissions = AdministratorOnly }),
            // The base role keeps everything but view: Jo, who holds the
            // gift, sees only where it allows.
            "base role permissions" => await state.Owner.PatchAsJsonAsync(
                $"/api/admin/roles/{PrivateChannelScenario.MemberRoleId}",
                new { permissions = EverythingButView }),
            "role deleted" => await state.Owner.DeleteAsync($"/api/admin/roles/{gift}"),
            "channel moved" => await ChannelSections.FileAsync(state.Owner, Open, state.HiddenSection),
            _ => await state.Owner.PutAsJsonAsync(
                $"/api/admin/channels/{Hidden}/permissions/{PrivateChannelScenario.MemberRoleId}",
                new { allow = ViewOnly, deny = NoNames }),
        };
        Assert.True(response.IsSuccessStatusCode, $"{change}: {response.StatusCode}");
        Assert.Equal(TestData.ServerId.ToString(), (await jo.NextAsync("ChannelAccessChanged")).GetProperty("serverId").GetString());

        // The owner sees the same as before, and hears nothing of it.
        await owner.SettleAsync();
        Assert.Equal(0, owner.Pending("ChannelAccessChanged"));

        // The next messages follow what Jo sees now: the one he lost passes
        // him by, the one he gained reaches him.
        var (first, second, expected) = change switch
        {
            "role taken" or "role deleted" => (Hidden, TestData.MemesChannelId, "w drugim"),
            "base role permissions" => (TestData.MemesChannelId, Hidden, "w drugim"),
            "channel moved" => (Open, TestData.MemesChannelId, "w drugim"),
            _ => (Hidden, TestData.MemesChannelId, "w pierwszym"),
        };
        await PostAsync(state.Owner, first, "w pierwszym");
        await PostAsync(state.Owner, second, "w drugim");
        Assert.Equal(expected, (await jo.NextAsync("MessageCreated")).GetProperty("content").GetString());
    }

    /// <summary>A role nobody holds yet that grants nothing, allowed to see <paramref name="allowedAt"/> when it names a channel.</summary>
    private static async Task<Guid> RoleAsync(PrivateChannelScenario state, string name, Guid? allowedAt)
    {
        var created = await state.Owner.PostAsJsonAsync("/api/admin/roles", new { name, color = "#123456", permissions = NoNames });
        created.EnsureSuccessStatusCode();
        var id = Guid.Parse((await created.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetString()!);
        if (allowedAt is { } channel)
        {
            (await state.Owner.PutAsJsonAsync(
                $"/api/admin/channels/{channel}/permissions/{id}",
                new { allow = ViewOnly, deny = NoNames })).EnsureSuccessStatusCode();
        }

        return id;
    }

    [Fact]
    public async Task SomebodyMovedIntoARoomTheyDoNotSeeHearsItWhileSeated()
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        await using var ilya = await state.ListenAsync("ilya", "VoiceParticipantChanged", "VoiceChannelMoved");
        await using var ada = await state.ListenAsync("ada");
        await ilya.Connection.InvokeAsync("JoinVoice", PrivateChannelScenario.OpenVoice.ToString());
        await ilya.NextAsync("VoiceParticipantChanged");

        Assert.Equal(HttpStatusCode.NoContent, (await state.Owner.PatchAsJsonAsync(
            $"/api/admin/channels/{PrivateChannelScenario.OpenVoice}/members/{PrivateChannelScenario.IlyaId}",
            new { targetChannelId = PrivateChannelScenario.HiddenVoice })).StatusCode);
        await ilya.NextAsync("VoiceChannelMoved");
        // The client confirms a move by joining; the seat it has is kept.
        await ilya.Connection.InvokeAsync("JoinVoice", PrivateChannelScenario.HiddenVoice.ToString());

        await ada.Connection.InvokeAsync("JoinVoice", PrivateChannelScenario.HiddenVoice.ToString());
        JsonElement seen;
        do
        {
            seen = await ilya.NextAsync("VoiceParticipantChanged");
        }
        while (seen.GetProperty("channelId").GetString() != PrivateChannelScenario.HiddenVoice.ToString() ||
               seen.GetProperty("participants").GetArrayLength() < 2);

        // Once out, the room is out of reach again.
        await ilya.Connection.InvokeAsync("LeaveVoice");
        await ada.Connection.InvokeAsync("LeaveVoice");
        await ilya.SettleAsync();
        while (ilya.Pending("VoiceParticipantChanged") > 0)
        {
            await ilya.NextAsync("VoiceParticipantChanged");
        }

        await ada.Connection.InvokeAsync("JoinVoice", PrivateChannelScenario.HiddenVoice.ToString());
        await ilya.Connection.InvokeAsync("JoinVoice", PrivateChannelScenario.OpenVoice.ToString());
        Assert.Equal(PrivateChannelScenario.OpenVoice.ToString(),
            (await ilya.NextAsync("VoiceParticipantChanged")).GetProperty("channelId").GetString());
        var refused = await Assert.ThrowsAsync<HubException>(() =>
            ilya.Connection.InvokeAsync("JoinVoice", PrivateChannelScenario.HiddenVoice.ToString()));
        Assert.Contains("Unknown voice room.", refused.Message);
    }

    /// <summary>
    /// Where nobody is denied view the events go to the server's group, as
    /// they always have, and nobody is evaluated. A connection left only in
    /// the server's group shows the first, a trace of the queries the second;
    /// on a private channel both turn the other way, which proves the test
    /// can see the difference.
    /// </summary>
    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task WhereNobodyIsDeniedViewAnEventGoesToTheServerAndEvaluatesNobody(bool hidden)
    {
        var trace = new CommandTrace();
        await using var state = await PrivateChannelScenario.CreateAsync(
            hide: hidden,
            configureServices: services => services.ConfigureDbContext<Stellarr.Api.Data.AppDbContext>(o => o.AddInterceptors(trace)));
        await using var owner = await state.ListenAsync("maja", "MessageCreated");
        var groups = state.Factory.Services.GetRequiredService<IHubContext<AppHub>>().Groups;
        await groups.RemoveFromGroupAsync(owner.Connection.ConnectionId!, AppHub.FollowerGroup(TestData.ServerId, PrivateChannelScenario.MajaId));

        trace.Clear();
        await PostAsync(state.Ada, Hidden, "do serwera");
        await PostAsync(state.Ada, Open, "kontrola");

        var first = (await owner.NextAsync("MessageCreated")).GetProperty("content").GetString();
        Assert.Equal(hidden ? "kontrola" : "do serwera", first);
        Assert.Equal(hidden, trace.Saw(Stellarr.Api.Auth.PermissionSnapshot.EveryMemberTag));
    }

    private static async Task PostAsync(HttpClient client, Guid channel, string content) =>
        Assert.Equal(HttpStatusCode.Created, (await client.PostAsJsonAsync($"/api/channels/{channel}/messages", new { content })).StatusCode);
}

/// <summary>Every SQL command a host ran, to see what a request evaluated.</summary>
internal sealed class CommandTrace : Microsoft.EntityFrameworkCore.Diagnostics.DbCommandInterceptor
{
    private readonly List<string> _commands = [];

    public void Clear()
    {
        lock (_commands) _commands.Clear();
    }

    public bool Saw(string text)
    {
        lock (_commands) return _commands.Any(x => x.Contains(text, StringComparison.Ordinal));
    }

    public override ValueTask<System.Data.Common.DbDataReader> ReaderExecutedAsync(
        System.Data.Common.DbCommand command,
        Microsoft.EntityFrameworkCore.Diagnostics.CommandExecutedEventData eventData,
        System.Data.Common.DbDataReader result,
        CancellationToken cancellationToken = default)
    {
        lock (_commands) _commands.Add(command.CommandText);
        return base.ReaderExecutedAsync(command, eventData, result, cancellationToken);
    }
}
