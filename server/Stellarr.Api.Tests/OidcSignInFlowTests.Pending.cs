using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth.Security;

namespace Stellarr.Api.Tests;

/// <summary>
/// An identity a provider confirmed that no account is linked to waits a few
/// minutes for the browser it came back to, and only for that browser.
/// </summary>
public sealed partial class OidcSignInFlowTests
{
    [Fact]
    public async Task UnlinkedIdentityWaitsForTheBrowserItCameBackTo()
    {
        using var provider = new FakeProvider { Subject = $"ola-{Guid.NewGuid():N}" };
        provider.TokenClaims["name"] = "Ola Łęcka";
        provider.TokenClaims["email"] = "ola@example.test";
        provider.TokenClaims["email_verified"] = true;
        using var host = Host(provider);
        using var client = BrowserClient(host);

        using var callback = await SignInThroughProviderAsync(client, provider, "/invite/abc");

        Assert.Equal("/login?oidc=unlinked&returnUrl=%2Finvite%2Fabc", callback.Headers.Location!.OriginalString);
        var session = await client.GetFromJsonAsync<JsonElement>("/api/session");
        Assert.Equal(JsonValueKind.Null, session.GetProperty("user").ValueKind);
        var pending = await client.GetFromJsonAsync<JsonElement>("/api/auth/oidc/pending");
        // Whose identity at which provider, and nothing about the accounts here.
        Assert.Equal(
            ["provider", "displayName", "email", "username", "signUp"],
            pending.EnumerateObject().Select(x => x.Name).ToArray());
        Assert.False(pending.GetProperty("signUp").GetBoolean());
        Assert.Equal("default", pending.GetProperty("provider").GetProperty("id").GetString());
        Assert.Equal("Test IdP", pending.GetProperty("provider").GetProperty("displayName").GetString());
        Assert.Equal("Ola Łęcka", pending.GetProperty("displayName").GetString());
        Assert.Equal("ola@example.test", pending.GetProperty("email").GetString());
        Assert.Equal("ola.lecka", pending.GetProperty("username").GetString());

        using var otherBrowser = BrowserClient(host);
        using var elsewhere = await otherBrowser.GetAsync("/api/auth/oidc/pending");
        Assert.Equal(HttpStatusCode.NotFound, elsewhere.StatusCode);

        await using var scope = factory.OpenScope(out var db);
        var stored = await db.PendingExternalIdentities.SingleAsync(x => x.Subject == provider.Subject);
        Assert.True(stored.EmailVerified);
        Assert.Equal(provider.IssuerUrl, stored.Issuer);
    }

    [Fact]
    public async Task ProfileTheProviderSendsOnlyFromItsUserinfoEndpointIsKept()
    {
        using var provider = new FakeProvider
        {
            Subject = $"userinfo-{Guid.NewGuid():N}",
            UserInfo = new Dictionary<string, object>
            {
                ["name"] = "Jan Kowalski",
                ["preferred_username"] = "jkowal",
                ["email"] = "jan@example.test",
                ["email_verified"] = true,
            },
        };
        using var host = Host(provider);
        using var client = BrowserClient(host);

        using var callback = await SignInThroughProviderAsync(client, provider, "/", keepSignedIn: false);

        Assert.Equal("/login?oidc=unlinked&keep_signed_in=0", callback.Headers.Location!.OriginalString);
        var pending = await client.GetFromJsonAsync<JsonElement>("/api/auth/oidc/pending");
        Assert.Equal("Jan Kowalski", pending.GetProperty("displayName").GetString());
        Assert.Equal("jkowal", pending.GetProperty("username").GetString());
        await using var scope = factory.OpenScope(out var db);
        var stored = await db.PendingExternalIdentities.SingleAsync(x => x.Subject == provider.Subject);
        Assert.Equal("jan@example.test", stored.Email);
        Assert.True(stored.EmailVerified);
        Assert.True(stored.ShortLived);
    }

    [Fact]
    public async Task UserinfoSaysOnlyWhatItStatesAboutTheAddress()
    {
        using var provider = new FakeProvider { Subject = $"repeat-{Guid.NewGuid():N}" };
        provider.TokenClaims["email"] = "ewa@example.test";
        provider.TokenClaims["email_verified"] = true;
        provider.UserInfo = new Dictionary<string, object> { ["name"] = "Ewa Nowak", ["email"] = "Ewa@example.test" };
        using var host = Host(provider);
        using (var browser = BrowserClient(host))
        {
            (await SignInThroughProviderAsync(browser, provider, "/")).Dispose();
        }

        // Another address is only as verified as userinfo says it is.
        var repeated = provider.Subject;
        provider.Subject = $"other-{Guid.NewGuid():N}";
        provider.UserInfo = new Dictionary<string, object> { ["email"] = "ewa.work@example.test" };
        using (var browser = BrowserClient(host))
        {
            (await SignInThroughProviderAsync(browser, provider, "/")).Dispose();
        }

        await using var scope = factory.OpenScope(out var db);
        var kept = await db.PendingExternalIdentities.SingleAsync(x => x.Subject == repeated);
        var changed = await db.PendingExternalIdentities.SingleAsync(x => x.Subject == provider.Subject);
        Assert.True(kept.EmailVerified);
        Assert.Equal("ewa.work@example.test", changed.Email);
        Assert.False(changed.EmailVerified);
    }

    [Fact]
    public async Task UnlinkedIdentityRunsOutAndCanBeDropped()
    {
        using var provider = new FakeProvider();
        using var host = Host(provider);
        using var client = BrowserClient(host);
        (await SignInThroughProviderAsync(client, provider, "/")).Dispose();
        await using (var scope = factory.OpenScope(out var db))
        {
            var stored = await db.PendingExternalIdentities.SingleAsync(x => x.Subject == provider.Subject);
            Assert.InRange(stored.ExpiresAt - stored.CreatedAt, TimeSpan.FromMinutes(9), TimeSpan.FromMinutes(11));
            await db.PendingExternalIdentities.Where(x => x.Id == stored.Id)
                .ExecuteUpdateAsync(setters => setters.SetProperty(x => x.ExpiresAt, DateTime.UtcNow.AddSeconds(-1)));
        }

        using (var expired = await client.GetAsync("/api/auth/oidc/pending"))
        {
            Assert.Equal(HttpStatusCode.NotFound, expired.StatusCode);
        }

        // A new round trip replaces it; going back to the sign-in page drops that one.
        (await SignInThroughProviderAsync(client, provider, "/")).Dispose();
        using (var waiting = await client.GetAsync("/api/auth/oidc/pending"))
        {
            Assert.Equal(HttpStatusCode.OK, waiting.StatusCode);
        }

        using var drop = await client.DeleteAsync("/api/auth/oidc/pending");
        using var dropped = await client.GetAsync("/api/auth/oidc/pending");

        Assert.Equal(HttpStatusCode.NoContent, drop.StatusCode);
        Assert.Equal(HttpStatusCode.NotFound, dropped.StatusCode);
        await using var check = factory.OpenScope(out var checkDb);
        Assert.False(await checkDb.PendingExternalIdentities.AnyAsync(x => x.Subject == provider.Subject));
    }

    [Theory]
    [InlineData("Ola Nowak", null, null, "ola.nowak")]
    [InlineData("  Żółć  Gęślą ", null, null, "zolc.gesla")]
    [InlineData("A", "x", "maja.k@example.test", "maja.k")]
    [InlineData(null, "Łukasz_Ł", null, "lukasz_l")]
    [InlineData("!!", null, null, "")]
    public void SuggestedLoginComesFromWhatTheProviderSaid(
        string? name, string? preferred, string? email, string expected)
    {
        var identity = new Domain.PendingExternalIdentity
        {
            SecretHash = "",
            ProviderId = "default",
            Issuer = "https://identity.test",
            Subject = "s",
            DisplayName = name,
            PreferredUsername = preferred,
            Email = email,
        };

        Assert.Equal(expected, PendingIdentities.SuggestUsername(identity));
    }
}
