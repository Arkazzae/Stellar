using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// The owner decides in Cockpit, per provider, whether it may create
/// accounts for people with an invite. It is off until turned on, the switch
/// asks for a fresh second step like other security changes, and each change
/// is in the audit log.
/// </summary>
public sealed class CockpitIdentityProviderTests
{
    [Fact]
    public async Task OwnerLetsOneProviderCreateAccountsAndTheSignInPageSaysSo()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var host = WithProviders(factory);
        using var owner = await SignInAsync(host, "maja", "Aurora!2026");

        var before = await owner.GetFromJsonAsync<JsonElement>("/api/cockpit/integrations/identity-providers");
        using var on = await owner.PutAsJsonAsync("/api/cockpit/integrations/identity-providers/work", new { signUp = true });
        var after = await owner.GetFromJsonAsync<JsonElement>("/api/cockpit/integrations/identity-providers");
        var signIn = await host.CreateClient().GetFromJsonAsync<JsonElement>("/api/auth/oidc");

        Assert.Equal(
            [("default", "Test IdP", false), ("work", "Work", false)],
            Providers(before));
        Assert.Equal(HttpStatusCode.OK, on.StatusCode);
        Assert.Equal(
            [("default", "Test IdP", false), ("work", "Work", true)],
            Providers(after));
        Assert.Equal(
            [false, true],
            signIn.GetProperty("providers").EnumerateArray().Select(x => x.GetProperty("signUp").GetBoolean()).ToArray());

        using var off = await owner.PutAsJsonAsync("/api/cockpit/integrations/identity-providers/work", new { signUp = false });
        Assert.False((await off.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("signUp").GetBoolean());
        await using var scope = factory.OpenScope(out var db);
        var audit = await db.ServerAuditEntries.Select(x => x.Summary).ToListAsync();
        Assert.Contains(audit, summary => summary.Contains("Work", StringComparison.Ordinal) && summary.Contains("zakładać konta", StringComparison.Ordinal));
        Assert.Contains(audit, summary => summary.Contains("Work", StringComparison.Ordinal) && summary.Contains("wyłączył", StringComparison.Ordinal));
        Assert.False((await db.IdentityProviderSettings.SingleAsync(x => x.ProviderId == "work")).SignUpAllowed);
    }

    [Fact]
    public async Task SwitchNeedsAFreshSecondStepAndAKnownProviderAndTheOwner()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var host = WithProviders(factory);
        using var member = await SignInAsync(host, "ada", "Quasar!2026");
        using var owner = await SignInAsync(host, "maja", "Aurora!2026");

        using var unknown = await owner.PutAsJsonAsync("/api/cockpit/integrations/identity-providers/nope", new { signUp = true });
        using var empty = await owner.PutAsJsonAsync("/api/cockpit/integrations/identity-providers/work", new { });
        using var notOwner = await member.PutAsJsonAsync("/api/cockpit/integrations/identity-providers/work", new { signUp = true });

        Assert.Equal(HttpStatusCode.NotFound, unknown.StatusCode);
        Assert.Equal(HttpStatusCode.BadRequest, empty.StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden, notOwner.StatusCode);

        // Once the owner has a second step, a session that has not passed one lately is refused.
        await using (var scope = factory.OpenScope(out var db))
        {
            var maja = await db.Users.SingleAsync(x => x.Username == "maja");
            db.MfaMethods.Add(new MfaMethod
            {
                Id = Guid.NewGuid(),
                UserId = maja.Id,
                Type = MfaMethodTypes.Totp,
                Name = "Phone",
                CreatedAt = DateTime.UtcNow,
                ConfirmedAt = DateTime.UtcNow,
            });
            await db.SaveChangesAsync();
        }

        using var stale = await owner.PutAsJsonAsync("/api/cockpit/integrations/identity-providers/work", new { signUp = true });

        Assert.Equal(HttpStatusCode.Forbidden, stale.StatusCode);
        Assert.Equal("fresh-mfa-required", (await stale.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("code").GetString());
        await using var check = factory.OpenScope(out var checkDb);
        Assert.False(await checkDb.IdentityProviderSettings.AnyAsync(x => x.SignUpAllowed));
    }

    private static (string?, string?, bool)[] Providers(JsonElement list) =>
        list.EnumerateArray()
            .Select(x => (x.GetProperty("id").GetString(), x.GetProperty("displayName").GetString(), x.GetProperty("signUp").GetBoolean()))
            .ToArray();

    /// <summary>The instance with two providers configured; nothing here goes to them.</summary>
    internal static WebApplicationFactory<Program> WithProviders(StellarrWebApplicationFactory factory) =>
        factory.WithWebHostBuilder(builder =>
        {
            builder.UseSetting("Oidc:Enabled", "true");
            builder.UseSetting("Oidc:Authority", "https://identity.test");
            builder.UseSetting("Oidc:ClientId", "stellarr-tests");
            builder.UseSetting("Oidc:ClientSecret", "test-client-secret");
            builder.UseSetting("Oidc:DisplayName", "Test IdP");
            builder.UseSetting("Oidc:Providers:work:Authority", "https://work-identity.test");
            builder.UseSetting("Oidc:Providers:work:ClientId", "stellarr-tests");
            builder.UseSetting("Oidc:Providers:work:ClientSecret", "work-client-secret");
            builder.UseSetting("Oidc:Providers:work:DisplayName", "Work");
        });

    private static async Task<HttpClient> SignInAsync(WebApplicationFactory<Program> host, string username, string password)
    {
        var client = host.CreateClient(new WebApplicationFactoryClientOptions
        {
            AllowAutoRedirect = false,
            HandleCookies = true,
        });
        (await client.PostAsJsonAsync("/api/auth/login", new { username, password })).EnsureSuccessStatusCode();
        return client;
    }
}
