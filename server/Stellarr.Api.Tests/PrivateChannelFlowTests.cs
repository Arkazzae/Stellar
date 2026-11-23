using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.SignalR;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Domain;
using Stellarr.Api.Notifications;

namespace Stellarr.Api.Tests;

/// <summary>
/// What a private channel means beyond its routes: how a channel becomes
/// private and visible again, what the bootstrap, mentions and their
/// deliveries, invites, attachments, typing and voice make of it, and how
/// the reorders and the server's muted channels treat what a manager cannot
/// see. The places and the people are those of <see cref="PrivateChannelScenario"/>.
/// </summary>
public sealed class PrivateChannelFlowTests
{
    private static readonly Guid Hidden = PrivateChannelScenario.Hidden;
    private static readonly Guid Open = PrivateChannelScenario.Open;
    private static readonly Guid JoId = PrivateChannelScenario.JoId;

    public static TheoryData<string, bool> Mechanisms() => new()
    {
        { "the base role denied at the channel", false },
        { "the base role denied at the section", false },
        { "the member's own deny", false },
        { "the base role without view anywhere", false },
        { "a held role's allow", true },
        { "the member's own allow", true },
        { "a held role's allow past the member's own deny elsewhere", true },
        { "administrator", true },
    };

    [Theory]
    [MemberData(nameof(Mechanisms))]
    public async Task AChannelIsPrivateOrVisibleByEachMechanism(string mechanism, bool sees)
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        var channel = mechanism switch
        {
            "the base role denied at the section" => state.SectionChannel,
            "the member's own deny" or "the base role without view anywhere" => TestData.MemesChannelId,
            _ => Hidden,
        };
        await state.WithDbAsync(async db =>
        {
            switch (mechanism)
            {
                case "the member's own deny":
                    db.MemberPermissionOverrides.Add(MemberOverride(PermissionTargetType.Channel, channel, deny: ServerPrivileges.ViewChannel));
                    break;
                case "the base role without view anywhere":
                    await db.ServerRoles.Where(x => x.Id == PrivateChannelScenario.MemberRoleId)
                        .ExecuteUpdateAsync(s => s.SetProperty(x => x.Permissions, ServerRole.BaseRoleDefaults & ~ServerPrivileges.ViewChannel));
                    break;
                case "a held role's allow":
                    await HoldAsync(db, ServerPrivileges.None, allowViewAt: Hidden);
                    break;
                case "the member's own allow":
                    db.MemberPermissionOverrides.Add(MemberOverride(PermissionTargetType.Channel, Hidden, allow: ServerPrivileges.ViewChannel));
                    break;
                case "a held role's allow past the member's own deny elsewhere":
                    // His own deny is at the section level of another place: it
                    // does not reach this channel, which the role opens.
                    db.MemberPermissionOverrides.Add(MemberOverride(PermissionTargetType.Section, state.HiddenSection, deny: ServerPrivileges.ViewChannel));
                    await HoldAsync(db, ServerPrivileges.None, allowViewAt: Hidden);
                    break;
                case "administrator":
                    db.MemberPermissionOverrides.Add(MemberOverride(PermissionTargetType.Channel, Hidden, deny: ServerPrivileges.ViewChannel));
                    await HoldAsync(db, ServerPrivileges.Administrator, allowViewAt: null);
                    break;
            }

            await db.SaveChangesAsync();
        });

        var history = await state.Jo.GetAsync($"/api/channels/{channel}/messages");
        Assert.Equal(sees ? HttpStatusCode.OK : HttpStatusCode.NotFound, history.StatusCode);
        Assert.Equal(sees, (await ChannelIdsAsync(state.Jo)).Contains(channel.ToString()));
        // The owner sees it whatever anybody else is denied.
        Assert.Equal(HttpStatusCode.OK, (await state.Owner.GetAsync($"/api/channels/{channel}/messages")).StatusCode);
        if (mechanism == "the base role without view anywhere")
        {
            Assert.Empty(await ChannelIdsAsync(state.Jo));
        }
    }

    private static MemberPermissionOverride MemberOverride(
        PermissionTargetType type, Guid target, ServerPrivileges allow = ServerPrivileges.None, ServerPrivileges deny = ServerPrivileges.None) => new()
    {
        Id = Guid.NewGuid(), ServerId = TestData.ServerId, UserId = JoId, TargetType = type, TargetId = target,
        AllowMask = allow, DenyMask = deny,
    };

    /// <summary>Gives Jo a role of his own with these permissions, allowed to see one channel when it names one.</summary>
    private static async Task HoldAsync(Stellarr.Api.Data.AppDbContext db, ServerPrivileges permissions, Guid? allowViewAt)
    {
        var role = new ServerRole
        {
            Id = Guid.NewGuid(), ServerId = TestData.ServerId, Name = $"Rola {Guid.NewGuid():N}", Color = "#123456",
            Position = 2, Permissions = permissions,
        };
        db.ServerRoles.Add(role);
        db.ServerMemberRoles.Add(new ServerMemberRole { ServerId = TestData.ServerId, UserId = JoId, RoleId = role.Id });
        if (allowViewAt is { } channel)
        {
            db.PermissionOverrides.Add(PrivateChannelScenario.RoleOverride(
                PermissionTargetType.Channel, channel, role.Id, allow: ServerPrivileges.ViewChannel));
        }

        await db.SaveChangesAsync();
        await db.ServerMembers.Where(x => x.ServerId == TestData.ServerId && x.UserId == JoId)
            .ExecuteUpdateAsync(s => s.SetProperty(x => x.RoleId, role.Id));
    }

    private static async Task<string?[]> ChannelIdsAsync(HttpClient client)
    {
        var bootstrap = await client.GetFromJsonAsync<JsonElement>("/api/bootstrap");
        return bootstrap.GetProperty("channels").EnumerateArray().Select(x => x.GetProperty("id").GetString()).ToArray();
    }

    [Fact]
    public async Task TheBootstrapCarriesOnlyWhatTheCallerSees()
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        // The private channel comes first, so the first page of somebody who
        // does not see it has to come from the first one they do.
        await state.WithDbAsync(db => db.Channels.Where(x => x.Id == Hidden)
            .ExecuteUpdateAsync(s => s.SetProperty(x => x.Position, -1)));
        await using (var seated = await state.ListenAsync("ada"))
        {
            await seated.Connection.InvokeAsync("JoinVoice", PrivateChannelScenario.HiddenVoice.ToString());

            var jo = await state.Jo.GetFromJsonAsync<JsonElement>("/api/bootstrap");
            var owner = await state.Owner.GetFromJsonAsync<JsonElement>("/api/bootstrap");

            var hiddenIds = new[] { Hidden, PrivateChannelScenario.HiddenVoice, state.SectionChannel }.Select(x => x.ToString()).ToArray();
            Assert.DoesNotContain(jo.GetProperty("channels").EnumerateArray(), x => hiddenIds.Contains(x.GetProperty("id").GetString()));
            Assert.Equal(6, owner.GetProperty("channels").GetArrayLength());
            Assert.Empty(jo.GetProperty("sections").EnumerateArray());
            Assert.Single(owner.GetProperty("sections").EnumerateArray());
            Assert.DoesNotContain(jo.GetProperty("channelReads").EnumerateObject(), x => hiddenIds.Contains(x.Name));
            Assert.DoesNotContain(PrivateChannelScenario.HiddenVoice.ToString(), jo.GetProperty("voice").EnumerateObject().Select(x => x.Name));
            Assert.Equal(1, owner.GetProperty("voice").GetProperty(PrivateChannelScenario.HiddenVoice.ToString()).GetArrayLength());
            Assert.All(jo.GetProperty("messages").EnumerateArray(), x => Assert.Equal(Open.ToString(), x.GetProperty("channelId").GetString()));
            Assert.All(owner.GetProperty("messages").EnumerateArray(), x => Assert.Equal(Hidden.ToString(), x.GetProperty("channelId").GetString()));
            foreach (var secret in PrivateChannelScenario.Secrets)
            {
                Assert.DoesNotContain(secret, jo.GetRawText(), StringComparison.Ordinal);
            }
        }

        // No read marker was started for him in a channel he does not see.
        Assert.False(await state.WithDbAsync(db => db.ChannelReads.AnyAsync(x => x.UserId == JoId && x.ChannelId == Hidden)));
    }

    [Fact]
    public async Task AMentionIsWrittenOnlyForThoseWhoSeeTheMessageAndSoIsItsDelivery()
    {
        await using var state = await PrivateChannelScenario.CreateAsync(configuration: NoWorker);
        // Signed in first: a confirmed address is a second step at sign-in.
        await using var jo = await state.ListenAsync("jo", "NotificationInboxChanged");
        await using var owner = await state.ListenAsync("maja", "NotificationInboxChanged");
        foreach (var userId in new[] { JoId, PrivateChannelScenario.MajaId })
        {
            await AcceptsMailAsync(state, userId);
        }

        var secret = await PostAsync(state.Ada, Hidden, "tajne @jo @maja");

        await owner.NextAsync("NotificationInboxChanged");
        Assert.Equal(
            [PrivateChannelScenario.MajaId],
            await state.WithDbAsync(db => db.UserNotifications.Where(x => x.TargetChannelId == Hidden).Select(x => x.UserId).ToArrayAsync()));
        Assert.Equal(
            [PrivateChannelScenario.MajaId],
            await state.WithDbAsync(db => db.NotificationDeliveries.Where(x => x.SourceMessageId == secret)
                .Select(x => x.RecipientId).Distinct().ToArrayAsync()));
        await jo.SettleAsync();
        Assert.Equal(0, jo.Pending("NotificationInboxChanged"));

        // In the open channel he is told as before.
        await PostAsync(state.Ada, Open, "jawne @jo");
        await jo.NextAsync("NotificationInboxChanged");
    }

    [Fact]
    public async Task ADeliveryForSomebodyWhoHasSinceLostViewIsCancelled()
    {
        await using var state = await PrivateChannelScenario.CreateAsync(configuration: NoWorker);
        await AcceptsMailAsync(state, JoId);
        await state.WithDbAsync(db =>
        {
            db.MemberPermissionOverrides.Add(MemberOverride(PermissionTargetType.Channel, Hidden, allow: ServerPrivileges.ViewChannel));
            return db.SaveChangesAsync();
        });
        var mentioned = await PostAsync(state.Ada, Hidden, "tajne @jo");
        Assert.NotEmpty(await state.WithDbAsync(db => db.NotificationDeliveries.Where(x => x.SourceMessageId == mentioned).ToArrayAsync()));

        await state.WithDbAsync(db => db.MemberPermissionOverrides.Where(x => x.UserId == JoId).ExecuteDeleteAsync());
        await using (var scope = state.Factory.Services.CreateAsyncScope())
        {
            await scope.ServiceProvider.GetRequiredService<NotificationDeliveryDispatcher>()
                .DispatchAsync(DateTime.UtcNow.AddMinutes(1), CancellationToken.None);
        }

        Assert.Empty(state.Factory.EmailDelivery.Messages);
        Assert.All(
            await state.WithDbAsync(db => db.NotificationDeliveries.Where(x => x.SourceMessageId == mentioned).ToArrayAsync()),
            row => Assert.Equal(NotificationDeliveryStatuses.Cancelled, row.Status));
    }

    private static readonly Dictionary<string, string?> NoWorker = new() { ["Notifications:WorkerEnabled"] = "false" };

    /// <summary>Gives the member a confirmed address and asks for mail of mentions.</summary>
    private static Task AcceptsMailAsync(PrivateChannelScenario state, Guid userId) => state.WithDbAsync(async db =>
    {
        var user = await db.Users.SingleAsync(x => x.Id == userId);
        var address = $"{user.Username}@example.test";
        user.Email = address;
        user.NormalizedEmail = address.ToUpperInvariant();
        db.MfaMethods.Add(new MfaMethod
        {
            Id = Guid.NewGuid(), UserId = userId, Type = MfaMethodTypes.Email, Name = "E-mail",
            CreatedAt = DateTime.UtcNow, ConfirmedAt = DateTime.UtcNow, EmailAddress = address.ToUpperInvariant(),
        });
        var preferences = await db.UserPreferences.SingleOrDefaultAsync(x => x.UserId == userId);
        if (preferences is null)
        {
            preferences = new UserPreferenceSettings { UserId = userId };
            db.UserPreferences.Add(preferences);
        }

        preferences.EmailNotifications = true;
        await db.SaveChangesAsync();
    });

    [Fact]
    public async Task AnInviteToAChannelTheNewcomerWouldNotSeeLandsThemWithoutNamingIt()
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        var toHidden = await InviteAsync(state.Owner, Hidden);
        var toOpen = await InviteAsync(state.Owner, Open);
        using var anonymous = state.Factory.CreateClient();

        var hiddenPreview = await anonymous.GetFromJsonAsync<JsonElement>($"/api/invites/{toHidden}");
        Assert.Equal(JsonValueKind.Null, hiddenPreview.GetProperty("channel").ValueKind);
        var openPreview = await anonymous.GetFromJsonAsync<JsonElement>($"/api/invites/{toOpen}");
        Assert.Equal(Open.ToString(), openPreview.GetProperty("channel").GetProperty("id").GetString());

        // Ilya, a member who does not see it, follows the link as well.
        using var ilya = await state.SignedInAsync("ilya");
        var joined = await (await ilya.PostAsync($"/api/invites/{toHidden}/join", null)).Content.ReadFromJsonAsync<JsonElement>();
        Assert.Equal(JsonValueKind.Null, joined.GetProperty("channelId").ValueKind);
        Assert.Equal(JsonValueKind.Null, joined.GetProperty("channelName").ValueKind);
        var preview = await ilya.GetFromJsonAsync<JsonElement>($"/api/invites/{toHidden}");
        Assert.Equal(JsonValueKind.Null, preview.GetProperty("channel").ValueKind);
        // Ada sees it.
        var adas = await state.Ada.GetFromJsonAsync<JsonElement>($"/api/invites/{toHidden}");
        Assert.Equal(Hidden.ToString(), adas.GetProperty("channel").GetProperty("id").GetString());

        // A manager who does not see it lists the invite, unnamed.
        var listed = await state.Nils.GetFromJsonAsync<JsonElement>("/api/admin/invites");
        var unnamed = listed.EnumerateArray().Single(x => x.GetProperty("url").GetString()!.EndsWith(toHidden, StringComparison.Ordinal));
        Assert.Equal(JsonValueKind.Null, unnamed.GetProperty("channelId").ValueKind);
        Assert.Equal(JsonValueKind.Null, unnamed.GetProperty("channelName").ValueKind);
        var named = (await state.Owner.GetFromJsonAsync<JsonElement>("/api/admin/invites")).EnumerateArray()
            .Single(x => x.GetProperty("url").GetString()!.EndsWith(toHidden, StringComparison.Ordinal));
        Assert.Equal("projekty", named.GetProperty("channelName").GetString());
    }

    private static async Task<string> InviteAsync(HttpClient client, Guid channel)
    {
        var created = await client.PostAsJsonAsync("/api/admin/invites", new { channelId = channel });
        Assert.Equal(HttpStatusCode.Created, created.StatusCode);
        return (await created.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("url").GetString()!.Split('/')[^1];
    }

    [Fact]
    public async Task TheWelcomeGoesToTheFirstChannelTheNewcomerWillSee()
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        await state.WithDbAsync(db => db.Channels.Where(x => x.Id == Hidden)
            .ExecuteUpdateAsync(s => s.SetProperty(x => x.Position, -1)));
        var invite = await InviteAsync(state.Owner, Open);
        await state.WithDbAsync(db => db.ServerMembers
            .Where(x => x.ServerId == TestData.ServerId && x.UserId == PrivateChannelScenario.IlyaId).ExecuteDeleteAsync());
        using var ilya = await state.SignedInAsync("ilya");

        Assert.Equal(HttpStatusCode.OK, (await ilya.PostAsync($"/api/invites/{invite}/join", null)).StatusCode);

        var welcome = await state.WithDbAsync(db => db.Messages.SingleAsync(x => x.IsSystem && x.AuthorId == PrivateChannelScenario.IlyaId));
        Assert.Equal(Open, welcome.ChannelId);
    }

    [Fact]
    public async Task AnAttachmentIsForThoseWhoSeeAChannelItIsInAndForWhoeverUploadedIt()
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        var asset = $"/assets/{state.Attachment}";
        Assert.Equal(HttpStatusCode.NotFound, (await state.Jo.GetAsync(asset)).StatusCode);
        Assert.Equal(HttpStatusCode.OK, (await state.Owner.GetAsync(asset)).StatusCode);

        // Ada uploaded it and keeps it, even once she no longer sees the channel.
        await state.WithDbAsync(db =>
        {
            db.MemberPermissionOverrides.Add(new MemberPermissionOverride
            {
                Id = Guid.NewGuid(), ServerId = TestData.ServerId, UserId = PrivateChannelScenario.AdaId,
                TargetType = PermissionTargetType.Channel, TargetId = Hidden, DenyMask = ServerPrivileges.ViewChannel,
            });
            return db.SaveChangesAsync();
        });
        Assert.Equal(HttpStatusCode.NotFound, (await state.Ada.GetAsync($"/api/channels/{Hidden}/messages")).StatusCode);
        Assert.Equal(HttpStatusCode.OK, (await state.Ada.GetAsync(asset)).StatusCode);

        // Posted in an open channel too, it is everybody's there.
        await state.WithDbAsync(db =>
        {
            db.Messages.Add(new Message
            {
                Id = Guid.NewGuid(), ChannelId = Open, AuthorId = PrivateChannelScenario.AdaId, Content = "", CreatedAt = DateTime.UtcNow,
                Attachments = [new MessageAttachment { AttachmentId = state.Attachment }],
            });
            return db.SaveChangesAsync();
        });
        Assert.Equal(HttpStatusCode.OK, (await state.Jo.GetAsync(asset)).StatusCode);
    }

    [Fact]
    public async Task TypingNeedsToSeeTheChannelAndToSendInIt()
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        await state.WithDbAsync(db =>
        {
            db.PermissionOverrides.Add(PrivateChannelScenario.RoleOverride(
                PermissionTargetType.Channel, TestData.MemesChannelId, PrivateChannelScenario.MemberRoleId, deny: ServerPrivileges.SendMessages));
            return db.SaveChangesAsync();
        });
        await using var jo = await state.ListenAsync("jo");

        await jo.Connection.InvokeAsync("SendTyping", Open.ToString());
        var muted = await Assert.ThrowsAsync<HubException>(() => jo.Connection.InvokeAsync("SendTyping", TestData.MemesChannelId.ToString()));
        Assert.Contains("Not allowed to send messages here.", muted.Message);
        var hidden = await Assert.ThrowsAsync<HubException>(() => jo.Connection.InvokeAsync("SendTyping", Hidden.ToString()));
        Assert.Contains("Unknown channel.", hidden.Message);
    }

    [Fact]
    public async Task JoiningVoiceNeedsToSeeTheRoomAndToConnect()
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        await state.WithDbAsync(db =>
        {
            db.PermissionOverrides.Add(PrivateChannelScenario.RoleOverride(
                PermissionTargetType.Channel, PrivateChannelScenario.OpenVoice, PrivateChannelScenario.MemberRoleId, deny: ServerPrivileges.Connect));
            return db.SaveChangesAsync();
        });
        await using var jo = await state.ListenAsync("jo");
        await using var ada = await state.ListenAsync("ada");

        var unseen = await Assert.ThrowsAsync<HubException>(() => jo.Connection.InvokeAsync("JoinVoice", PrivateChannelScenario.HiddenVoice.ToString()));
        Assert.Contains("Unknown voice room.", unseen.Message);
        var refused = await Assert.ThrowsAsync<HubException>(() => jo.Connection.InvokeAsync("JoinVoice", PrivateChannelScenario.OpenVoice.ToString()));
        Assert.Contains("Not allowed to join this voice room.", refused.Message);
        await ada.Connection.InvokeAsync("JoinVoice", PrivateChannelScenario.HiddenVoice.ToString());
    }

    [Fact]
    public async Task AReorderNamesWhatTheCallerSeesAndLeavesTheRestWhereItWas()
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        var before = await OrderAsync(state);
        var seen = (await ChannelIdsAsync(state.Nils)).Select(x => Guid.Parse(x!)).ToArray();
        Assert.DoesNotContain(Hidden, seen);

        // All of them would name what he does not see; it is as unknown as any id.
        Assert.Equal(HttpStatusCode.BadRequest, (await state.Nils.PutAsJsonAsync(
            "/api/admin/channels/order", new { channelIds = before })).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await state.Nils.PutAsJsonAsync(
            "/api/admin/channels/order", new { channelIds = seen.Reverse().ToArray() })).StatusCode);

        var after = await OrderAsync(state);
        var hidden = new HashSet<Guid> { Hidden, PrivateChannelScenario.HiddenVoice, state.SectionChannel };
        // What he cannot see is where it was; what he can is in his order, in the places it had.
        Assert.Equal(before.Select(x => hidden.Contains(x)), after.Select(x => hidden.Contains(x)));
        Assert.Equal(before.Where(hidden.Contains), after.Where(hidden.Contains));
        Assert.Equal(seen.Reverse(), after.Where(x => !hidden.Contains(x)));
    }

    private static Task<Guid[]> OrderAsync(PrivateChannelScenario state) => state.WithDbAsync(db =>
        db.Channels.Where(x => x.ServerId == TestData.ServerId).OrderBy(x => x.Position).ThenBy(x => x.Id).Select(x => x.Id).ToArrayAsync());

    [Fact]
    public async Task ASectionReorderLeavesTheSectionsTheCallerCannotSeeWhereTheyWere()
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        var first = await SectionAsync(state.Owner, "Pierwsza");
        var last = await SectionAsync(state.Owner, "Ostatnia");

        Assert.Equal(HttpStatusCode.NoContent, (await state.Nils.PutAsJsonAsync(
            "/api/admin/sections/order", new { sectionIds = new[] { last, first } })).StatusCode);

        var order = await state.WithDbAsync(db =>
            db.Sections.Where(x => x.ServerId == TestData.ServerId).OrderBy(x => x.Position).Select(x => x.Id).ToArrayAsync());
        Assert.Equal([state.HiddenSection, last, first], order);
    }

    private static async Task<Guid> SectionAsync(HttpClient client, string name)
    {
        var created = await client.PostAsJsonAsync("/api/admin/sections", new { name });
        return Guid.Parse((await created.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetString()!);
    }

    [Fact]
    public async Task TheMutedChannelsAManagerCannotSeeStayMutedUnnamed()
    {
        await using var state = await PrivateChannelScenario.CreateAsync();
        Assert.Equal(HttpStatusCode.OK, (await state.Owner.PatchAsJsonAsync(
            "/api/admin/notifications", new { mutedChannelIds = new[] { Hidden, Open } })).StatusCode);

        string[] Muted(JsonElement settings) =>
            settings.GetProperty("mutedChannelIds").EnumerateArray().Select(x => x.GetString()!).ToArray();
        Assert.Equal([Open.ToString()], Muted(await state.Nils.GetFromJsonAsync<JsonElement>("/api/admin/notifications")));

        var saved = await state.Nils.PatchAsJsonAsync("/api/admin/notifications", new { mutedChannelIds = new[] { TestData.MemesChannelId } });
        Assert.Equal([TestData.MemesChannelId.ToString()], Muted(await saved.Content.ReadFromJsonAsync<JsonElement>()));
        Assert.Equal(
            new[] { TestData.MemesChannelId, Hidden }.Select(x => x.ToString()).Order(),
            Muted(await state.Owner.GetFromJsonAsync<JsonElement>("/api/admin/notifications")).Order());
    }

    private static async Task<Guid> PostAsync(HttpClient client, Guid channel, string content)
    {
        var posted = await client.PostAsJsonAsync($"/api/channels/{channel}/messages", new { content });
        Assert.Equal(HttpStatusCode.Created, posted.StatusCode);
        return Guid.Parse((await posted.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetString()!);
    }
}
