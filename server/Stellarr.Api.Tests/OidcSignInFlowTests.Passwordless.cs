using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.WebUtilities;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth.Security;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// An account made through a provider has no password. A password sign-in to
/// it fails like a wrong password, a sensitive change takes a fresh sign-in at
/// the provider instead of the password, and its last way in stays.
/// </summary>
public sealed partial class OidcSignInFlowTests
{
    [Fact]
    public async Task AccountWithoutAPasswordRefusesEveryPasswordLikeAWrongOne()
    {
        using var provider = new FakeProvider();
        using var host = Host(provider);
        using var client = BrowserClient(host);
        var passwordless = await CreatePasswordlessAccountAsync(provider);
        var withPassword = await CreatePasswordAccountAsync();

        using var wrong = await client.PostAsJsonAsync(
            "/api/auth/login", new { username = withPassword.Username, password = "Not-the-password-1" });
        var wrongTitle = (await wrong.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("title").GetString();
        foreach (var password in new[] { "Not-the-password-1", "", "hash" })
        {
            using var attempt = await client.PostAsJsonAsync(
                "/api/auth/login", new { username = passwordless.Username, password });

            Assert.Equal(wrong.StatusCode, attempt.StatusCode);
            Assert.Equal(wrongTitle, (await attempt.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("title").GetString());
        }

        await using var scope = factory.OpenScope(out var db);
        Assert.Equal(3, await db.SecurityEvents.CountAsync(
            x => x.UserId == passwordless.Id && x.Kind == SecurityEventKinds.LoginFailed));
        Assert.False(await db.UserSessions.AnyAsync(x => x.UserId == passwordless.Id));
    }

    [Fact]
    public async Task AccountWithoutAPasswordSetsItsFirstOneOnlyWithAFreshProof()
    {
        using var provider = new FakeProvider();
        using var host = Host(provider);
        using var client = BrowserClient(host);
        var account = await CreatePasswordlessAccountAsync(provider);
        (await SignInThroughProviderAsync(client, provider, "/")).Dispose();
        Assert.False((await client.GetFromJsonAsync<JsonElement>("/api/me/security")).GetProperty("hasPassword").GetBoolean());
        await AgeSessionsAsync(account.Id);

        using var stale = await client.PostAsJsonAsync("/api/me/security/password", new { newPassword = "First!Password2026" });
        Assert.Equal(HttpStatusCode.Forbidden, stale.StatusCode);
        Assert.Equal("fresh-mfa-required", (await stale.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("code").GetString());

        await RenewSessionsAsync(account.Id);
        using var tooShort = await client.PostAsJsonAsync("/api/me/security/password", new { newPassword = "short" });
        using var set = await client.PostAsJsonAsync("/api/me/security/password", new { newPassword = "First!Password2026" });
        using var again = await client.PostAsJsonAsync("/api/me/security/password", new { newPassword = "Second!Password2026" });

        Assert.Equal(HttpStatusCode.BadRequest, tooShort.StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, set.StatusCode);
        Assert.Equal(HttpStatusCode.Conflict, again.StatusCode);
        Assert.True((await client.GetFromJsonAsync<JsonElement>("/api/me/security")).GetProperty("hasPassword").GetBoolean());
        using var other = BrowserClient(host);
        using var login = await other.PostAsJsonAsync(
            "/api/auth/login", new { username = account.Username, password = "First!Password2026" });
        Assert.Equal(HttpStatusCode.OK, login.StatusCode);
        await using var scope = factory.OpenScope(out var db);
        Assert.True(await db.SecurityEvents.AnyAsync(
            x => x.UserId == account.Id && x.Kind == SecurityEventKinds.PasswordChanged));
    }

    [Fact]
    public async Task FreshSignInAtTheProviderStandsInForThePassword()
    {
        using var provider = new FakeProvider();
        using var work = new FakeProvider("https://work-identity.test");
        using var host = Host(provider, work);
        using var client = BrowserClient(host);
        var account = await CreatePasswordlessAccountAsync(provider);
        (await SignInThroughProviderAsync(client, provider, "/")).Dispose();
        await AgeSessionsAsync(account.Id);
        using (var refused = await client.PostAsJsonAsync("/api/me/security/identities/work/link", new { }))
        {
            Assert.Equal(HttpStatusCode.Forbidden, refused.StatusCode);
        }

        // Only a provider the account signs in with can stand in.
        using (var notLinked = await client.PostAsJsonAsync("/api/me/security/identities/work/reauthenticate", new { }))
        {
            Assert.Equal(HttpStatusCode.NotFound, notLinked.StatusCode);
        }

        // A provider that waves the person through on an old session of its own proves nothing new.
        provider.AuthTime = DateTimeOffset.UtcNow.AddHours(-2);
        var old = await ReauthenticateAsync(client, provider, "/channels/abc");
        Assert.Equal("/link-account?result=unconfirmed&returnUrl=%2Fchannels%2Fabc", old);
        using (var stillRefused = await client.PostAsJsonAsync("/api/me/security/identities/work/link", new { }))
        {
            Assert.Equal(HttpStatusCode.Forbidden, stillRefused.StatusCode);
        }

        provider.AuthTime = DateTimeOffset.UtcNow;
        var fresh = await ReauthenticateAsync(client, provider, "/channels/abc");

        Assert.Equal("/link-account?result=confirmed&returnUrl=%2Fchannels%2Fabc", fresh);
        using var allowed = await client.PostAsJsonAsync("/api/me/security/identities/work/link", new { });
        Assert.Equal(HttpStatusCode.OK, allowed.StatusCode);
    }

    [Fact]
    public async Task FreshSignInAsSomeoneElseAtTheProviderConfirmsNothing()
    {
        using var provider = new FakeProvider();
        using var host = Host(provider);
        using var client = BrowserClient(host);
        var account = await CreatePasswordlessAccountAsync(provider);
        (await SignInThroughProviderAsync(client, provider, "/")).Dispose();
        await AgeSessionsAsync(account.Id);
        provider.Subject = $"subject-{Guid.NewGuid():N}";
        provider.AuthTime = DateTimeOffset.UtcNow;

        var answer = await ReauthenticateAsync(client, provider, "/");

        Assert.Equal("/link-account?result=unconfirmed", answer);
        await using var scope = factory.OpenScope(out var db);
        var session = await db.UserSessions.SingleAsync(x => x.UserId == account.Id);
        Assert.True(session.AuthenticatedAt < DateTime.UtcNow.AddMinutes(-30));
    }

    [Fact]
    public async Task FreshSignInCountsFromWhenTheProviderSignedThePersonIn()
    {
        using var provider = new FakeProvider();
        using var host = Host(provider);
        using var client = BrowserClient(host);
        var account = await CreatePasswordlessAccountAsync(provider);
        using (var signIn = await client.GetAsync("/api/auth/oidc/login"))
        {
            // An ordinary sign-in leaves the provider to its own session.
            Assert.False(QueryHelpers.ParseQuery(signIn.Headers.Location!.Query).ContainsKey("prompt"));
        }

        (await SignInThroughProviderAsync(client, provider, "/")).Dispose();
        await AgeSessionsAsync(account.Id);

        // Recent enough for the provider check, but already past the fresh window.
        provider.AuthTime = DateTimeOffset.UtcNow.AddMinutes(-5.5);
        Assert.Equal("/link-account?result=unconfirmed", await ReauthenticateAsync(client, provider, "/"));

        var signedIn = DateTimeOffset.UtcNow.AddMinutes(-3);
        provider.AuthTime = signedIn;
        Assert.Equal("/link-account?result=confirmed", await ReauthenticateAsync(client, provider, "/"));

        await using var scope = factory.OpenScope(out var db);
        var session = await db.UserSessions.SingleAsync(x => x.UserId == account.Id);
        Assert.InRange(session.AuthenticatedAt!.Value, signedIn.UtcDateTime.AddSeconds(-1), signedIn.UtcDateTime);
    }

    [Fact]
    public async Task LastWayInCannotBeUnlinkedNorItsLastPasskeyRemoved()
    {
        using var provider = new FakeProvider();
        using var host = Host(provider);
        using var client = BrowserClient(host);
        var account = await CreatePasswordlessAccountAsync(provider);
        (await SignInThroughProviderAsync(client, provider, "/")).Dispose();
        var identityId = (await client.GetFromJsonAsync<JsonElement>("/api/me/security"))
            .GetProperty("identities")[0].GetProperty("id").GetString();

        using (var refused = await client.PostAsJsonAsync($"/api/me/security/identities/{identityId}/remove", new { }))
        {
            Assert.Equal(HttpStatusCode.Conflict, refused.StatusCode);
            var problem = await refused.Content.ReadFromJsonAsync<JsonElement>();
            Assert.Equal("last-way-in", problem.GetProperty("code").GetString());
            Assert.Contains(
                "Najpierw ustaw hasło albo dodaj klucz dostępu", problem.GetProperty("title").GetString(), StringComparison.Ordinal);
        }

        // A passkey is a way in of its own, so the identity can go; the
        // passkey, now the last way in, cannot.
        var passkeyId = Guid.NewGuid();
        await using (var scope = factory.OpenScope(out var db))
        {
            db.MfaMethods.Add(new MfaMethod
            {
                Id = passkeyId,
                UserId = account.Id,
                Type = MfaMethodTypes.Passkey,
                Name = "Laptop",
                CreatedAt = DateTime.UtcNow,
                ConfirmedAt = DateTime.UtcNow,
            });
            await db.SaveChangesAsync();
            await db.UserSessions.Where(x => x.UserId == account.Id)
                .ExecuteUpdateAsync(setters => setters.SetProperty(x => x.MfaCompletedAt, DateTime.UtcNow));
        }

        using var unlinked = await client.PostAsJsonAsync($"/api/me/security/identities/{identityId}/remove", new { });
        using var lastPasskey = await client.PostAsJsonAsync($"/api/me/security/mfa/{passkeyId}/remove", new { });

        Assert.Equal(HttpStatusCode.NoContent, unlinked.StatusCode);
        Assert.Equal(HttpStatusCode.Conflict, lastPasskey.StatusCode);
        Assert.Equal("last-way-in", (await lastPasskey.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("code").GetString());
        await using var check = factory.OpenScope(out var checkDb);
        Assert.Null((await checkDb.MfaMethods.SingleAsync(x => x.Id == passkeyId)).RevokedAt);
    }

    [Fact]
    public async Task TwoRemovalsAtOnceStillLeaveAWayIn()
    {
        using var provider = new FakeProvider();
        using var work = new FakeProvider("https://work-identity.test");
        using var host = Host(provider, work);
        using var client = BrowserClient(host);
        var account = await CreatePasswordlessAccountAsync(provider);
        await using (var scope = factory.OpenScope(out var db))
        {
            db.ExternalIdentities.Add(new ExternalIdentity
            {
                Id = Guid.NewGuid(),
                UserId = account.Id,
                Issuer = work.IssuerUrl,
                Subject = work.Subject,
                ProviderId = "work",
                CreatedAt = DateTime.UtcNow,
            });
            await db.SaveChangesAsync();
        }

        (await SignInThroughProviderAsync(client, provider, "/")).Dispose();
        var identities = (await client.GetFromJsonAsync<JsonElement>("/api/me/security")).GetProperty("identities")
            .EnumerateArray().Select(x => x.GetProperty("id").GetString()).ToArray();

        // Each alone would leave the other; both at once must not both go.
        var answers = await Task.WhenAll(identities.Select(
            id => client.PostAsJsonAsync($"/api/me/security/identities/{id}/remove", new { })));

        Assert.Equal(
            new[] { HttpStatusCode.NoContent, HttpStatusCode.Conflict },
            answers.Select(x => x.StatusCode).Order().ToArray());
        await using var check = factory.OpenScope(out var checkDb);
        Assert.Equal(1, await checkDb.ExternalIdentities.CountAsync(x => x.UserId == account.Id));
    }

    [Fact]
    public async Task LinkThatNamesNoProviderIsAtTheOriginalOneAndAGoneProviderIsNoWayIn()
    {
        using var provider = new FakeProvider();
        using var host = Host(provider);
        using var client = BrowserClient(host);
        var account = await CreatePasswordlessAccountAsync(provider, providerId: null);
        await using (var scope = factory.OpenScope(out var db))
        {
            db.ExternalIdentities.Add(new ExternalIdentity
            {
                Id = Guid.NewGuid(),
                UserId = account.Id,
                Issuer = "https://gone-identity.test",
                Subject = "gone",
                ProviderId = "gone",
                CreatedAt = DateTime.UtcNow.AddMinutes(1),
            });
            await db.SaveChangesAsync();
        }

        (await SignInThroughProviderAsync(client, provider, "/")).Dispose();
        var identities = (await client.GetFromJsonAsync<JsonElement>("/api/me/security")).GetProperty("identities");
        using var atOriginal = await client.PostAsJsonAsync("/api/me/security/identities/default/reauthenticate", new { });
        using var atGone = await client.PostAsJsonAsync("/api/me/security/identities/gone/reauthenticate", new { });
        using var removal = await client.PostAsJsonAsync(
            $"/api/me/security/identities/{identities[0].GetProperty("id").GetString()}/remove", new { });

        Assert.Equal(
            ["default", "gone"],
            identities.EnumerateArray().Select(x => x.GetProperty("providerId").GetString()!).ToArray());
        Assert.Equal(HttpStatusCode.OK, atOriginal.StatusCode);
        Assert.Equal(HttpStatusCode.NotFound, atGone.StatusCode);
        Assert.Equal(HttpStatusCode.Conflict, removal.StatusCode);
    }

    [Fact]
    public async Task AccountWithoutAPasswordChangesItsAddressWithAFreshProofInstead()
    {
        using var provider = new FakeProvider();
        using var host = Host(provider);
        using var client = BrowserClient(host);
        var account = await CreatePasswordlessAccountAsync(provider);
        (await SignInThroughProviderAsync(client, provider, "/")).Dispose();
        await AgeSessionsAsync(account.Id);
        var change = new { email = $"{account.Username}@example.test" };

        using var stale = await client.PostAsJsonAsync("/api/me/email", change);
        Assert.Equal("second-step-required", (await stale.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("result").GetString());

        await RenewSessionsAsync(account.Id);
        using var fresh = await client.PostAsJsonAsync("/api/me/email", change);
        Assert.Equal("sent", (await fresh.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("result").GetString());
    }

    [Fact]
    public async Task OperatorRecoveryOfAnAccountWithoutAPasswordStillLeadsToANewMethod()
    {
        using var provider = new FakeProvider();
        using var host = Host(provider);
        using var client = BrowserClient(host);
        var account = await CreatePasswordlessAccountAsync(provider);
        string credential;
        await using (var scope = factory.OpenScope(out var db))
        {
            var result = await AccountRecovery.RecoverAsync(
                db, account.Id, "lost phone", SecurityPolicy.Default, _ => Task.CompletedTask, CancellationToken.None);
            credential = result!.Credential;
        }

        // The provider sends the person to the recovery, which needs no password.
        using var callback = await SignInThroughProviderAsync(client, provider, "/");
        Assert.Equal("/login?security_recovery_required=1", callback.Headers.Location!.OriginalString);
        using var redeem = await client.PostAsJsonAsync("/api/auth/recovery/redeem", new { credential });
        redeem.EnsureSuccessStatusCode();
        Assert.Equal("recovery", (await redeem.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("purpose").GetString());
    }

    /// <summary>Starts a fresh sign-in at the provider as proof and answers it; returns where the browser lands.</summary>
    private static async Task<string> ReauthenticateAsync(HttpClient client, FakeProvider provider, string returnUrl)
    {
        using var start = await client.PostAsJsonAsync("/api/me/security/identities/default/reauthenticate", new { });
        start.EnsureSuccessStatusCode();
        var url = (await start.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("url").GetString()!;
        using var challenge = await client.GetAsync($"{url}&returnUrl={Uri.EscapeDataString(returnUrl)}");
        Assert.Equal(HttpStatusCode.Redirect, challenge.StatusCode);
        var query = QueryHelpers.ParseQuery(challenge.Headers.Location!.Query);
        // The provider is told how recent the sign-in must be, and to ask again.
        Assert.Equal("300", query["max_age"].ToString());
        Assert.Equal("login", query["prompt"].ToString());
        provider.Nonce = query["nonce"].ToString();
        using var answer = await client.GetAsync(
            $"/api/auth/oidc/callback?code=the-code&state={Uri.EscapeDataString(query["state"].ToString())}");
        Assert.Equal(HttpStatusCode.Redirect, answer.StatusCode);
        return answer.Headers.Location!.OriginalString;
    }

    /// <summary>An account made through the provider: an identity there and no password.</summary>
    private async Task<User> CreatePasswordlessAccountAsync(FakeProvider provider, string? providerId = "default")
    {
        await using var scope = factory.OpenScope(out var db);
        var user = (await DataModelSupport.AddUsersAsync(db, 1))[0];
        user.PasswordHash = null;
        db.ExternalIdentities.Add(new ExternalIdentity
        {
            Id = Guid.NewGuid(),
            UserId = user.Id,
            Issuer = provider.IssuerUrl,
            Subject = provider.Subject,
            ProviderId = providerId,
            CreatedAt = DateTime.UtcNow,
        });
        await db.SaveChangesAsync();
        return user;
    }

    /// <summary>Makes the account's sign-ins an hour old, past any fresh proof.</summary>
    private async Task AgeSessionsAsync(Guid userId)
    {
        await using var scope = factory.OpenScope(out var db);
        var longAgo = DateTime.UtcNow.AddHours(-1);
        await db.UserSessions.Where(x => x.UserId == userId)
            .ExecuteUpdateAsync(setters => setters
                .SetProperty(x => x.AuthenticatedAt, longAgo)
                .SetProperty(x => x.CreatedAt, longAgo));
    }

    private async Task RenewSessionsAsync(Guid userId)
    {
        await using var scope = factory.OpenScope(out var db);
        await db.UserSessions.Where(x => x.UserId == userId)
            .ExecuteUpdateAsync(setters => setters.SetProperty(x => x.AuthenticatedAt, DateTime.UtcNow));
    }
}
