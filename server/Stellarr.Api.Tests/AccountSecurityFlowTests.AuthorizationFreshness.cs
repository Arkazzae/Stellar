using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

public sealed partial class AccountSecurityFlowTests
{
    private static readonly string[] AuthorizationFreshnessMessagePermission = ["manageMessages"];

    [Theory]
    [InlineData("override-put")]
    [InlineData("override-delete")]
    [InlineData("role-delete")]
    [InlineData("reparent")]
    public async Task InstancePermissionChangesRequireFreshMfaIncludingInheritedChanges(string operation)
    {
        var reparents = operation == "reparent";
        using var factory = new StellarrWebApplicationFactory();
        using var owner = Client(factory);
        await LoginAsync(owner, "maja", "Aurora!2026");
        Guid ownerId;
        var roleId = Guid.NewGuid();
        var destination = Guid.NewGuid();
        await using (var scope = factory.OpenScope(out var db))
        {
            ownerId = await db.Users.Where(x => x.Username == "maja").Select(x => x.Id).SingleAsync();
            db.ServerRoles.Add(new ServerRole
            {
                Id = roleId, ServerId = TestData.ServerId, Name = "Freshness target", Color = "#123456",
                Position = 25, Permissions = ServerPrivileges.ManageMessages,
            });
            db.Sections.Add(new Section { Id = destination, ServerId = TestData.ServerId, Name = "Destination", Position = 20 });
            if (operation == "override-delete" || reparents)
            {
                db.PermissionOverrides.Add(new PermissionOverride
                {
                    Id = Guid.NewGuid(), ServerId = TestData.ServerId, RoleId = roleId,
                    TargetType = reparents ? PermissionTargetType.Section : PermissionTargetType.Channel,
                    TargetId = reparents ? destination : TestData.GeneralChannelId,
                    DenyMask = ServerPrivileges.ManageMessages,
                });
            }
            await db.SaveChangesAsync();
        }
        await AddConfirmedTotpAsync(factory, await FindUserAsync(factory, ownerId));
        await SetSessionMfaTimeAsync(factory, ownerId, DateTime.UtcNow.AddMinutes(-15));

        Task<HttpResponseMessage> ChangeAsync() => operation switch
        {
            "override-put" => owner.PutAsJsonAsync($"/api/admin/channels/{TestData.GeneralChannelId}/permissions/{roleId}",
                new { allow = Array.Empty<string>(), deny = AuthorizationFreshnessMessagePermission }),
            "override-delete" => owner.DeleteAsync($"/api/admin/channels/{TestData.GeneralChannelId}/permissions/{roleId}"),
            "role-delete" => owner.DeleteAsync($"/api/admin/roles/{roleId}"),
            _ => ChannelSections.FileAsync(owner, TestData.GeneralChannelId, destination),
        };
        using var stale = await ChangeAsync();
        Assert.Equal(HttpStatusCode.Forbidden, stale.StatusCode);
        Assert.Equal(FreshSecondStepFilter.Code,
            (await stale.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("code").GetString());
        await using (var scope = factory.OpenScope(out var db))
        {
            Assert.True(await db.ServerRoles.AnyAsync(x => x.Id == roleId));
            Assert.Null((await db.Channels.SingleAsync(x => x.Id == TestData.GeneralChannelId)).SectionId);
            Assert.Equal(operation == "override-delete" || reparents ? 1 : 0,
                await db.PermissionOverrides.CountAsync(x => x.RoleId == roleId));
        }

        await SetSessionMfaTimeAsync(factory, ownerId, DateTime.UtcNow);
        using var fresh = await ChangeAsync();
        Assert.Equal(operation == "override-put" ? HttpStatusCode.OK : HttpStatusCode.NoContent, fresh.StatusCode);
    }

    [Fact]
    public async Task OrganizationWithoutPermissionChangesDoesNotRequireFreshMfa()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var owner = Client(factory);
        await LoginAsync(owner, "maja", "Aurora!2026");
        Guid ownerId;
        var destination = Guid.NewGuid();
        await using (var scope = factory.OpenScope(out var db))
        {
            ownerId = await db.Users.Where(x => x.Username == "maja").Select(x => x.Id).SingleAsync();
            db.Sections.Add(new Section { Id = destination, ServerId = TestData.ServerId, Name = "Ordinary move", Position = 30 });
            await db.SaveChangesAsync();
        }
        await AddConfirmedTotpAsync(factory, await FindUserAsync(factory, ownerId));
        await SetSessionMfaTimeAsync(factory, ownerId, DateTime.UtcNow.AddMinutes(-15));

        using var response = await ChannelSections.FileAsync(owner, TestData.GeneralChannelId, destination);
        Assert.Equal(HttpStatusCode.NoContent, response.StatusCode);
        await using var check = factory.OpenScope(out var checkDb);
        Assert.Equal(destination, (await checkDb.Channels.SingleAsync(x => x.Id == TestData.GeneralChannelId)).SectionId);
    }
}
