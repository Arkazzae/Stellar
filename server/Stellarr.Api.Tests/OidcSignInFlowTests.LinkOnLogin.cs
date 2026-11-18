using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth.Security;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// A password sign-in on the page for an unlinked identity asks to link it.
/// The link is made only once the whole sign-in is done, second step and all,
/// under the rules of a link from the settings, and a refusal leaves the
/// person signed in.
/// </summary>
public sealed partial class OidcSignInFlowTests
{
    [Fact]
    public async Task PasswordSignInLinksTheIdentityWaitingForThisBrowser()
    {
        using var provider = new FakeProvider();
        using var host = Host(provider);
        using var client = BrowserClient(host);
        var account = await CreatePasswordAccountAsync();
        (await SignInThroughProviderAsync(client, provider, "/")).Dispose();

        var answer = await LoginAndLinkAsync(client, account.Username, LinkPassword);

        Assert.Equal(IdentityLinkOutcomes.Linked, answer.GetProperty("identityLink").GetString());
        Assert.Equal(account.Id.ToString(), answer.GetProperty("user").GetProperty("id").GetString());
        using (var pending = await client.GetAsync("/api/auth/oidc/pending"))
        {
            Assert.Equal(HttpStatusCode.NotFound, pending.StatusCode);
        }

        await using (var scope = factory.OpenScope(out var db))
        {
            var identity = await db.ExternalIdentities.SingleAsync(x => x.UserId == account.Id);
            Assert.Equal((provider.IssuerUrl, provider.Subject, "default"), (identity.Issuer, identity.Subject, identity.ProviderId));
            Assert.True(await db.SecurityEvents.AnyAsync(
                x => x.UserId == account.Id && x.Kind == SecurityEventKinds.IdentityLinked));
        }

        using var next = BrowserClient(host);
        (await SignInThroughProviderAsync(next, provider, "/")).Dispose();
        var session = await next.GetFromJsonAsync<JsonElement>("/api/session");
        Assert.Equal(account.Id.ToString(), session.GetProperty("user").GetProperty("id").GetString());
    }

    [Fact]
    public async Task LinkWaitsForTheSecondStepAndARealOne()
    {
        using var provider = new FakeProvider();
        using var host = Host(provider);
        using var client = BrowserClient(host);
        var account = await CreatePasswordAccountAsync();
        IReadOnlyList<string> codes;
        await using (var scope = factory.OpenScope(out var db))
        {
            db.MfaMethods.Add(new MfaMethod
            {
                Id = Guid.NewGuid(),
                UserId = account.Id,
                Type = MfaMethodTypes.Totp,
                Name = "Phone",
                CreatedAt = DateTime.UtcNow,
                ConfirmedAt = DateTime.UtcNow,
            });
            codes = await RecoveryCodes.RegenerateAsync(db, account.Id, DateTime.UtcNow, CancellationToken.None);
            await db.SaveChangesAsync();
        }

        (await SignInThroughProviderAsync(client, provider, "/")).Dispose();

        var challenge = await LoginAndLinkAsync(client, account.Username, LinkPassword);

        // The password alone links nothing.
        Assert.Equal("mfa_required", challenge.GetProperty("status").GetString());
        Assert.False(challenge.GetProperty("trustedDeviceAvailable").GetBoolean());
        await AssertNotLinkedAsync(account.Id);
        var challengeId = challenge.GetProperty("challengeId").GetString();
        using (var trusted = await client.PostAsJsonAsync(
                   "/api/auth/mfa/verify", new { challengeId, useTrustedDevice = true }))
        {
            Assert.Equal(HttpStatusCode.Conflict, trusted.StatusCode);
        }

        await AssertNotLinkedAsync(account.Id);

        using var verify = await client.PostAsJsonAsync("/api/auth/mfa/verify", new { challengeId, code = codes[0] });

        verify.EnsureSuccessStatusCode();
        Assert.Equal(
            IdentityLinkOutcomes.Linked,
            (await verify.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("identityLink").GetString());
        await using var check = factory.OpenScope(out var checkDb);
        Assert.True(await checkDb.ExternalIdentities.AnyAsync(x => x.UserId == account.Id && x.Subject == provider.Subject));
    }

    [Fact]
    public async Task SecondStepSetUpOnTheWayLinksOnceItIsDone()
    {
        using var provider = new FakeProvider();
        using var host = Host(provider);
        using var client = BrowserClient(host);
        var account = await CreatePasswordAccountAsync();
        (await SignInThroughProviderAsync(client, provider, "/")).Dispose();
        await SetRequirementAsync(MfaRequirements.Everyone);
        try
        {
            var challenge = await LoginAndLinkAsync(client, account.Username, LinkPassword);

            Assert.Equal("mfa_enrollment_required", challenge.GetProperty("status").GetString());
            await AssertNotLinkedAsync(account.Id);
            var challengeId = challenge.GetProperty("challengeId").GetString();
            using var start = await client.PostAsJsonAsync(
                "/api/auth/mfa/enroll/totp/start", new { challengeId, name = "Phone" });
            var draft = await start.Content.ReadFromJsonAsync<JsonElement>();
            using var confirm = await client.PostAsJsonAsync(
                "/api/auth/mfa/enroll/totp/confirm",
                new
                {
                    challengeId,
                    methodId = draft.GetProperty("methodId").GetString(),
                    code = TotpService.ComputeAt(draft.GetProperty("secret").GetString()!, DateTime.UtcNow),
                    name = "Phone",
                });

            confirm.EnsureSuccessStatusCode();
            Assert.Equal(
                IdentityLinkOutcomes.Linked,
                (await confirm.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("identityLink").GetString());
        }
        finally
        {
            await SetRequirementAsync(MfaRequirements.Optional);
        }
    }

    [Fact]
    public async Task RefusedLinkLeavesTheSignInStandingAndDropsTheIdentity()
    {
        using var provider = new FakeProvider();
        using var host = Host(provider);
        var someoneElse = await CreatePasswordAccountAsync();
        var account = await CreatePasswordAccountAsync();

        // Linked to another account between the provider's answer and this sign-in.
        using var raced = BrowserClient(host);
        (await SignInThroughProviderAsync(raced, provider, "/")).Dispose();
        await using (var scope = factory.OpenScope(out var db))
        {
            db.ExternalIdentities.Add(new ExternalIdentity
            {
                Id = Guid.NewGuid(),
                UserId = someoneElse.Id,
                Issuer = provider.IssuerUrl,
                Subject = provider.Subject,
                CreatedAt = DateTime.UtcNow,
            });
            await db.SaveChangesAsync();
        }

        var taken = await LoginAndLinkAsync(raced, account.Username, LinkPassword);
        Assert.Equal(IdentityLinkOutcomes.Taken, taken.GetProperty("identityLink").GetString());

        // Another identity of the same provider is already the account's.
        provider.Subject = $"subject-{Guid.NewGuid():N}";
        using var other = BrowserClient(host);
        (await SignInThroughProviderAsync(other, provider, "/")).Dispose();
        await using (var scope = factory.OpenScope(out var db))
        {
            db.ExternalIdentities.Add(new ExternalIdentity
            {
                Id = Guid.NewGuid(),
                UserId = account.Id,
                Issuer = provider.IssuerUrl,
                Subject = $"older-{Guid.NewGuid():N}",
                CreatedAt = DateTime.UtcNow,
            });
            await db.SaveChangesAsync();
        }

        var second = await LoginAndLinkAsync(other, account.Username, LinkPassword);
        Assert.Equal(IdentityLinkOutcomes.Other, second.GetProperty("identityLink").GetString());

        foreach (var client in new[] { raced, other })
        {
            var session = await client.GetFromJsonAsync<JsonElement>("/api/session");
            Assert.Equal(account.Id.ToString(), session.GetProperty("user").GetProperty("id").GetString());
            using var pending = await client.GetAsync("/api/auth/oidc/pending");
            Assert.Equal(HttpStatusCode.NotFound, pending.StatusCode);
        }

        await using var check = factory.OpenScope(out var checkDb);
        Assert.False(await checkDb.ExternalIdentities.AnyAsync(x => x.UserId == account.Id && x.Subject == provider.Subject));
    }

    [Fact]
    public async Task IdentityThatRanOutOrWaitsElsewhereLinksNothing()
    {
        using var provider = new FakeProvider();
        using var host = Host(provider);
        var account = await CreatePasswordAccountAsync();
        using var client = BrowserClient(host);
        (await SignInThroughProviderAsync(client, provider, "/")).Dispose();

        // Another browser, which the identity is not waiting for, signs in without it.
        using var elsewhere = BrowserClient(host);
        var foreign = await LoginAndLinkAsync(elsewhere, account.Username, LinkPassword);
        Assert.Equal(IdentityLinkOutcomes.Failed, foreign.GetProperty("identityLink").GetString());

        // A wrong password leaves the identity waiting.
        using (var wrong = await client.PostAsJsonAsync(
                   "/api/auth/login", new { username = account.Username, password = "Wrong!2026x", linkPending = true }))
        {
            Assert.Equal(HttpStatusCode.Unauthorized, wrong.StatusCode);
        }

        using (var still = await client.GetAsync("/api/auth/oidc/pending"))
        {
            Assert.Equal(HttpStatusCode.OK, still.StatusCode);
        }

        await using (var scope = factory.OpenScope(out var db))
        {
            await db.PendingExternalIdentities.Where(x => x.Subject == provider.Subject)
                .ExecuteUpdateAsync(setters => setters.SetProperty(x => x.ExpiresAt, DateTime.UtcNow.AddSeconds(-1)));
        }

        var expired = await LoginAndLinkAsync(client, account.Username, LinkPassword);

        Assert.Equal(IdentityLinkOutcomes.Failed, expired.GetProperty("identityLink").GetString());
        await AssertNotLinkedAsync(account.Id);
    }

    private static async Task<JsonElement> LoginAndLinkAsync(HttpClient client, string username, string password)
    {
        using var login = await client.PostAsJsonAsync(
            "/api/auth/login", new { username, password, linkPending = true });
        login.EnsureSuccessStatusCode();
        return await login.Content.ReadFromJsonAsync<JsonElement>();
    }

    private async Task AssertNotLinkedAsync(Guid userId)
    {
        await using var scope = factory.OpenScope(out var db);
        Assert.False(await db.ExternalIdentities.AnyAsync(x => x.UserId == userId));
    }
}
