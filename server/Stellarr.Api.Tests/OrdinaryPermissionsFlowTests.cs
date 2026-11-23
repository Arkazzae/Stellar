using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// The permissions an ordinary member uses, each admitted in one place and
/// decided in the channel, where overrides count: posting, attaching,
/// reacting and creating an invite. Jo acts and holds only the base role
/// unless a case gives him more. Each case has a server of its own.
/// </summary>
public sealed class OrdinaryPermissionsFlowTests
{
    private static readonly Guid AdaId = Guid.Parse("30000000-0000-0000-0000-000000000002");
    private static readonly Guid NilsId = Guid.Parse("30000000-0000-0000-0000-000000000004");
    private static readonly Guid JoId = Guid.Parse("30000000-0000-0000-0000-000000000005");
    private static readonly Guid MajaId = Guid.Parse("30000000-0000-0000-0000-000000000001");
    private static readonly Guid MemberRoleId = Guid.Parse("60000000-0000-0000-0000-000000000003");
    private static readonly Guid FixtureMessageId = Guid.Parse("40000000-0000-0000-0000-000000000001");
    private static readonly string[] Administrator = ["administrator"];
    private static readonly string[] Nothing = [];

    public static TheoryData<string, string> Gates()
    {
        var data = new TheoryData<string, string>();
        foreach (var gate in new[] { "send", "reply", "attach", "voice-message", "react", "channel-invite" })
        {
            foreach (var way in new[]
                     {
                         "default", "base-role-loses-it", "held-role", "channel-allow", "channel-deny",
                         "member-deny", "administrator-past-channel-deny",
                     })
            {
                data.Add(gate, way);
            }
        }

        return data;
    }

    [Theory]
    [MemberData(nameof(Gates))]
    public async Task EachPermissionIsDecidedWhereItIsUsed(string gate, string way)
    {
        using var state = await Scenario.CreateAsync();
        var bit = gate switch
        {
            "send" or "reply" => ServerPrivileges.SendMessages,
            "attach" or "voice-message" => ServerPrivileges.AttachFiles,
            "react" => ServerPrivileges.AddReactions,
            _ => ServerPrivileges.CreateInvite,
        };
        // Creating invites is not the base role's by default; here an
        // administrator has given it to everybody, so every gate starts open.
        if (bit == ServerPrivileges.CreateInvite)
        {
            await state.BaseRoleAsync(held => held | ServerPrivileges.CreateInvite);
        }

        switch (way)
        {
            case "base-role-loses-it":
                await state.BaseRoleAsync(held => held & ~bit);
                break;
            case "held-role":
                await state.BaseRoleAsync(held => held & ~bit);
                await state.HoldAsync(JoId, bit);
                break;
            case "channel-allow":
                await state.BaseRoleAsync(held => held & ~bit);
                await state.RoleOverrideAsync(MemberRoleId, allow: bit);
                break;
            case "channel-deny":
                await state.RoleOverrideAsync(MemberRoleId, deny: bit);
                break;
            case "member-deny":
                await state.MemberOverrideAsync(JoId, deny: bit);
                break;
            case "administrator-past-channel-deny":
                await state.RoleOverrideAsync(MemberRoleId, deny: bit);
                await state.MemberOverrideAsync(JoId, deny: bit);
                await state.HoldAsync(JoId, ServerPrivileges.Administrator);
                break;
        }

        var allowed = way is "default" or "held-role" or "channel-allow" or "administrator-past-channel-deny";
        using var response = await state.UseAsync(gate);
        if (allowed)
        {
            Assert.True(response.IsSuccessStatusCode, $"{gate} by {way}: {response.StatusCode}");
            return;
        }

        Assert.Equal(HttpStatusCode.Forbidden, response.StatusCode);
        var problem = await response.Content.ReadFromJsonAsync<JsonElement>();
        Assert.Equal(ServerAccess.ToNames(bit)[0], problem.GetProperty("permission").GetString());
        Assert.Equal(
            gate switch
            {
                "send" or "reply" => "Nie możesz wysyłać wiadomości na tym kanale.",
                "attach" or "voice-message" => "Nie możesz dołączać plików na tym kanale.",
                "react" => "Nie możesz dodawać reakcji na tym kanale.",
                _ => "Nie możesz tu tworzyć zaproszeń.",
            },
            problem.GetProperty("title").GetString());
        await state.WithDbAsync(async db =>
        {
            Assert.False(await db.Messages.AnyAsync(x => x.AuthorId == JoId));
            Assert.False(await db.MessageReactions.AnyAsync(x => x.UserId == JoId));
            Assert.False(await db.ServerInvites.AnyAsync(x => x.CreatedById == JoId));
        });
    }

    [Fact]
    public async Task WithoutSendingAMemberStillEditsAndDeletesWhatTheyPostedAndTakesTheirReactionBack()
    {
        using var state = await Scenario.CreateAsync();
        using var posted = await state.UseAsync("send");
        var messageId = (await posted.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetString();
        using (var reacted = await state.UseAsync("react"))
        {
            reacted.EnsureSuccessStatusCode();
        }

        await state.BaseRoleAsync(_ => ServerPrivileges.None);

        using var edited = await state.Jo.PatchAsJsonAsync($"/api/messages/{messageId}", new { content = "poprawione" });
        Assert.Equal(HttpStatusCode.OK, edited.StatusCode);
        using var unreacted = await state.Jo.DeleteAsync($"/api/messages/{FixtureMessageId}/reactions/{Uri.EscapeDataString("👍")}");
        Assert.Equal(HttpStatusCode.OK, unreacted.StatusCode);
        using var deleted = await state.Jo.DeleteAsync($"/api/messages/{messageId}");
        Assert.Equal(HttpStatusCode.NoContent, deleted.StatusCode);
        await state.WithDbAsync(async db => Assert.False(await db.MessageReactions.AnyAsync(x => x.UserId == JoId)));
    }

    [Fact]
    public async Task ARetryOfASendThatWentThroughIsAnsweredEvenAfterThePermissionIsGone()
    {
        using var state = await Scenario.CreateAsync();
        var send = new { content = "raz", clientMessageId = Guid.NewGuid() };
        using var first = await state.Jo.PostAsJsonAsync($"/api/channels/{TestData.GeneralChannelId}/messages", send);
        Assert.Equal(HttpStatusCode.Created, first.StatusCode);

        await state.BaseRoleAsync(held => held & ~ServerPrivileges.SendMessages);

        using var retry = await state.Jo.PostAsJsonAsync($"/api/channels/{TestData.GeneralChannelId}/messages", send);
        Assert.Equal(HttpStatusCode.OK, retry.StatusCode);
        using var another = await state.Jo.PostAsJsonAsync(
            $"/api/channels/{TestData.GeneralChannelId}/messages", new { content = "dwa", clientMessageId = Guid.NewGuid() });
        Assert.Equal(HttpStatusCode.Forbidden, another.StatusCode);
    }

    [Fact]
    public async Task ByDefaultInvitesAreCreatedByThoseWhoManageTheServerAndNobodyElse()
    {
        using var state = await Scenario.CreateAsync();
        using var ada = await state.SignedInAsync("ada", "Quasar!2026");

        Assert.Equal(HttpStatusCode.Created, (await state.Owner.PostAsJsonAsync("/api/admin/invites", new { })).StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden, (await ada.PostAsJsonAsync("/api/admin/invites", new { })).StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden, (await state.Jo.PostAsJsonAsync("/api/admin/invites", new { })).StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden, (await state.Jo.PostAsJsonAsync(
            "/api/admin/invites", new { channelId = TestData.GeneralChannelId })).StatusCode);
    }

    [Fact]
    public async Task CreatingInvitesLeavesListingAndRevokingThemWithManagingTheServer()
    {
        using var state = await Scenario.CreateAsync();
        await state.BaseRoleAsync(held => held | ServerPrivileges.CreateInvite);

        using var created = await state.Jo.PostAsJsonAsync("/api/admin/invites", new { });
        Assert.Equal(HttpStatusCode.Created, created.StatusCode);
        var inviteId = (await created.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetString();

        Assert.Equal(HttpStatusCode.Forbidden, (await state.Jo.GetAsync("/api/admin/invites")).StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden, (await state.Jo.DeleteAsync($"/api/admin/invites/{inviteId}")).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await state.Owner.DeleteAsync($"/api/admin/invites/{inviteId}")).StatusCode);
    }

    [Fact]
    public async Task AChannelAllowLetsAMemberInviteToThatChannelButNotToTheServer()
    {
        using var state = await Scenario.CreateAsync();
        await state.RoleOverrideAsync(MemberRoleId, allow: ServerPrivileges.CreateInvite);

        Assert.Equal(HttpStatusCode.Created, (await state.Jo.PostAsJsonAsync(
            "/api/admin/invites", new { channelId = TestData.GeneralChannelId })).StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden, (await state.Jo.PostAsJsonAsync(
            "/api/admin/invites", new { channelId = TestData.ProjectsChannelId })).StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden, (await state.Jo.PostAsJsonAsync("/api/admin/invites", new { })).StatusCode);
    }

    [Theory]
    [InlineData("role-allow")]
    [InlineData("role-deny")]
    [InlineData("member-allow")]
    [InlineData("member-deny")]
    public async Task AdministratorIsNeverPartOfAnOverride(string attempt)
    {
        using var state = await Scenario.CreateAsync();
        var (allow, deny) = attempt.EndsWith("allow", StringComparison.Ordinal) ? (Administrator, Nothing) : (Nothing, Administrator);
        var url = attempt.StartsWith("role", StringComparison.Ordinal)
            ? $"{Scenario.OverridesUrl}/{MemberRoleId}"
            : $"{Scenario.OverridesUrl}/members/{JoId}";

        using var response = await state.Owner.PutAsJsonAsync(url, new { allow, deny });

        Assert.Equal(HttpStatusCode.BadRequest, response.StatusCode);
        var problem = await response.Content.ReadFromJsonAsync<JsonElement>();
        Assert.Equal(
            "Uprawnienie administratora obowiązuje na całym serwerze i nie da się go ustawić dla kanału ani sekcji.",
            problem.GetProperty("errors").GetProperty("permissions")[0].GetString());
        await state.WithDbAsync(async db =>
        {
            Assert.False(await db.PermissionOverrides.AnyAsync(x => x.TargetId == TestData.GeneralChannelId));
            Assert.False(await db.MemberPermissionOverrides.AnyAsync(x => x.TargetId == TestData.GeneralChannelId));
        });
    }

    [Fact]
    public async Task AnAdministratorHoldsEverythingButStillOutranksNobodyAboveThem()
    {
        using var state = await Scenario.CreateAsync();
        await state.HoldAsync(JoId, ServerPrivileges.Administrator);

        await state.WithDbAsync(async db =>
            Assert.Equal(ServerPrivileges.All, await ServerAccess.GetAsync(db, JoId, TestData.ServerId)));
        // Ada's moderator role ranks above the role that makes Jo an administrator.
        Assert.Equal(HttpStatusCode.Forbidden, (await state.Jo.DeleteAsync($"/api/admin/members/{AdaId}")).StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden, (await state.Jo.DeleteAsync($"/api/admin/members/{MajaId}")).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await state.Jo.DeleteAsync($"/api/admin/members/{NilsId}")).StatusCode);
    }

    [Fact]
    public async Task GrantingAdministratorTakesHoldingIt()
    {
        using var state = await Scenario.CreateAsync();
        // Ada manages roles but is no administrator.
        await state.WithDbAsync(db => db.ServerRoles.Where(x => x.Name == "Moderacja")
            .ExecuteUpdateAsync(s => s.SetProperty(x => x.Permissions, ServerPrivileges.ManageRoles)));
        using var ada = await state.SignedInAsync("ada", "Quasar!2026");
        await state.HoldAsync(JoId, ServerPrivileges.Administrator | ServerPrivileges.ManageRoles);

        Assert.Equal(HttpStatusCode.Forbidden, (await ada.PostAsJsonAsync(
            "/api/admin/roles", new { name = "Admini", color = "#123456", permissions = Administrator })).StatusCode);
        Assert.Equal(HttpStatusCode.Created, (await state.Jo.PostAsJsonAsync(
            "/api/admin/roles", new { name = "Admini", color = "#123456", permissions = Administrator })).StatusCode);
    }

    /// <summary>
    /// The fixture's server with Jo and Maja, the owner, signed in. Jo's role,
    /// when a case gives him one, ranks below Ada's and above the base role.
    /// </summary>
    private sealed class Scenario : IDisposable
    {
        internal StellarrWebApplicationFactory Factory { get; } = new();
        internal HttpClient Jo { get; private set; } = null!;
        internal HttpClient Owner { get; private set; } = null!;

        internal static string OverridesUrl => $"/api/admin/channels/{TestData.GeneralChannelId}/permissions";

        internal static async Task<Scenario> CreateAsync()
        {
            var state = new Scenario();
            state.Jo = await state.SignedInAsync("jo", "Corona!2026");
            state.Owner = await state.SignedInAsync("maja", "Aurora!2026");
            return state;
        }

        /// <summary>Does one of the things a gate admits, as Jo, in #ogólny.</summary>
        internal async Task<HttpResponseMessage> UseAsync(string gate)
        {
            var messages = $"/api/channels/{TestData.GeneralChannelId}/messages";
            switch (gate)
            {
                case "send":
                    return await Jo.PostAsJsonAsync(messages, new { content = "cześć" });
                case "reply":
                    return await Jo.PostAsJsonAsync(messages, new { content = "w odpowiedzi", replyToMessageId = FixtureMessageId });
                case "attach":
                case "voice-message":
                    var attachmentId = Guid.NewGuid();
                    await WithDbAsync(db =>
                    {
                        db.ImageAttachments.Add(new ImageAttachment
                        {
                            Id = attachmentId,
                            UploadedById = JoId,
                            ObjectKey = $"uploads/{attachmentId:N}",
                            FileName = gate == "attach" ? "notatka.txt" : "glos.ogg",
                            ContentType = gate == "attach" ? "text/plain" : "audio/ogg",
                            SizeBytes = 12,
                            IsVoiceMessage = gate == "voice-message",
                            CreatedAt = DateTime.UtcNow,
                        });
                        return db.SaveChangesAsync();
                    });
                    return await Jo.PostAsJsonAsync(messages, new { content = "", attachmentIds = new[] { attachmentId } });
                case "react":
                    return await Jo.PutAsync($"/api/messages/{FixtureMessageId}/reactions/{Uri.EscapeDataString("👍")}", content: null);
                default:
                    return await Jo.PostAsJsonAsync("/api/admin/invites", new { channelId = TestData.GeneralChannelId });
            }
        }

        internal Task BaseRoleAsync(Func<ServerPrivileges, ServerPrivileges> change) => WithDbAsync(async db =>
        {
            var role = await db.ServerRoles.SingleAsync(x => x.Id == MemberRoleId);
            role.Permissions = change(role.Permissions);
            await db.SaveChangesAsync();
        });

        /// <summary>Gives the member a role of their own with these permissions, ranked just above the base role.</summary>
        internal Task HoldAsync(Guid userId, ServerPrivileges permissions) => WithDbAsync(async db =>
        {
            var role = new ServerRole
            {
                Id = Guid.NewGuid(), ServerId = TestData.ServerId, Name = $"Rola {Guid.NewGuid():N}", Color = "#123456",
                Position = 2, Permissions = permissions,
            };
            db.ServerRoles.Add(role);
            db.ServerMemberRoles.Add(new ServerMemberRole { ServerId = TestData.ServerId, UserId = userId, RoleId = role.Id });
            await db.SaveChangesAsync();
            await db.ServerRoles.Where(x => x.Id == MemberRoleId).ExecuteUpdateAsync(s => s.SetProperty(x => x.Position, 3));
            await db.ServerMembers.Where(x => x.ServerId == TestData.ServerId && x.UserId == userId)
                .ExecuteUpdateAsync(s => s.SetProperty(x => x.RoleId, role.Id));
        });

        internal Task RoleOverrideAsync(
            Guid roleId, ServerPrivileges allow = ServerPrivileges.None, ServerPrivileges deny = ServerPrivileges.None) =>
            WithDbAsync(db =>
            {
                db.PermissionOverrides.Add(new PermissionOverride
                {
                    Id = Guid.NewGuid(), ServerId = TestData.ServerId, RoleId = roleId,
                    TargetType = PermissionTargetType.Channel, TargetId = TestData.GeneralChannelId,
                    AllowMask = allow, DenyMask = deny,
                });
                return db.SaveChangesAsync();
            });

        internal Task MemberOverrideAsync(Guid userId, ServerPrivileges deny) => WithDbAsync(db =>
        {
            db.MemberPermissionOverrides.Add(new MemberPermissionOverride
            {
                Id = Guid.NewGuid(), ServerId = TestData.ServerId, UserId = userId,
                TargetType = PermissionTargetType.Channel, TargetId = TestData.GeneralChannelId, DenyMask = deny,
            });
            return db.SaveChangesAsync();
        });

        internal async Task WithDbAsync(Func<AppDbContext, Task> action)
        {
            await using var scope = Factory.OpenScope(out var db);
            await action(db);
        }

        internal async Task<HttpClient> SignedInAsync(string username, string password)
        {
            var client = Factory.CreateClient(new WebApplicationFactoryClientOptions { AllowAutoRedirect = false, HandleCookies = true });
            (await client.PostAsJsonAsync("/api/auth/login", new { username, password })).EnsureSuccessStatusCode();
            return client;
        }

        public void Dispose()
        {
            Jo.Dispose();
            Owner.Dispose();
            Factory.Dispose();
        }
    }
}
