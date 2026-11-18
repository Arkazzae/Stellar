using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth.Security;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// Someone a provider confirmed, with no account here, creates one through
/// that provider: only with an invite, only when the owner lets the provider
/// create accounts, never as the instance's first account. The account has no
/// password, is linked to the identity and signs in as a sign-in through the
/// provider would.
/// </summary>
public sealed partial class OidcSignInFlowTests
{
    [Fact]
    public async Task ProviderAllowedToCreateAccountsMakesOneFromAnInvite()
    {
        var username = DataModelSupport.UniqueName();
        using var provider = new FakeProvider { Subject = $"new-{Guid.NewGuid():N}" };
        provider.TokenClaims["email"] = $"{username}.Nowak@Example.test";
        provider.TokenClaims["email_verified"] = true;
        using var host = Host(provider);
        using var client = BrowserClient(host);
        var invite = await AddInviteAsync();
        await AllowSignUpAsync(true);
        try
        {
            (await SignInThroughProviderAsync(client, provider, "/")).Dispose();

            using var register = await RegisterThroughProviderAsync(client, username, $"{username}.nowak@example.test", invite);

            Assert.Equal(HttpStatusCode.Created, register.StatusCode);
            var session = await client.GetFromJsonAsync<JsonElement>("/api/session");
            Assert.Equal(username, session.GetProperty("user").GetProperty("username").GetString());
            using (var pending = await client.GetAsync("/api/auth/oidc/pending"))
            {
                Assert.Equal(HttpStatusCode.NotFound, pending.StatusCode);
            }

            await using (var scope = factory.OpenScope(out var db))
            {
                var user = await db.Users.SingleAsync(x => x.Username == username);
                Assert.Null(user.PasswordHash);
                // The address the provider verified, typed in another case, counts as verified.
                Assert.Equal($"{username}.nowak@example.test".ToUpperInvariant(), user.ProviderVerifiedEmail);
                Assert.Equal($"{username}.nowak@example.test", await SecurityEvents.VerifiedEmailAsync(db, user.Id, CancellationToken.None));
                var identity = await db.ExternalIdentities.SingleAsync(x => x.UserId == user.Id);
                Assert.Equal((provider.Subject, "default"), (identity.Subject, identity.ProviderId));
                Assert.True(await db.ServerMembers.AnyAsync(x => x.UserId == user.Id && x.ServerId == TestData.ServerId));
                Assert.Equal(1, (await db.ServerInvites.SingleAsync(x => x.Code == invite)).Uses);
                Assert.Equal("oidc", (await db.UserSessions.SingleAsync(x => x.UserId == user.Id)).PrimarySource);
                Assert.False(await db.PendingExternalIdentities.AnyAsync(x => x.Subject == provider.Subject));
            }

            // It signs in through the provider from now on, and no password opens it.
            using var next = BrowserClient(host);
            (await SignInThroughProviderAsync(next, provider, "/")).Dispose();
            Assert.Equal(username, (await next.GetFromJsonAsync<JsonElement>("/api/session"))
                .GetProperty("user").GetProperty("username").GetString());
            using var password = await next.PostAsJsonAsync("/api/auth/login", new { username, password = "" });
            Assert.Equal(HttpStatusCode.Unauthorized, password.StatusCode);
        }
        finally
        {
            await AllowSignUpAsync(false);
        }
    }

    [Theory]
    [InlineData(false, true)]
    [InlineData(true, false)]
    public async Task OnlyTheAddressTheProviderVerifiedIsMarkedVerified(bool typedTheProvidersAddress, bool providerVerified)
    {
        var username = DataModelSupport.UniqueName();
        using var provider = new FakeProvider { Subject = $"new-{Guid.NewGuid():N}" };
        provider.TokenClaims["email"] = $"{username}@example.test";
        provider.TokenClaims["email_verified"] = providerVerified;
        using var host = Host(provider);
        using var client = BrowserClient(host);
        var invite = await AddInviteAsync();
        await AllowSignUpAsync(true);
        try
        {
            (await SignInThroughProviderAsync(client, provider, "/")).Dispose();
            var typed = typedTheProvidersAddress ? $"{username}@example.test" : $"{username}-other@example.test";

            using var register = await RegisterThroughProviderAsync(client, username, typed, invite);

            Assert.Equal(HttpStatusCode.Created, register.StatusCode);
            await using var scope = factory.OpenScope(out var db);
            var user = await db.Users.SingleAsync(x => x.Username == username);
            Assert.Null(user.ProviderVerifiedEmail);
            Assert.Null(await SecurityEvents.VerifiedEmailAsync(db, user.Id, CancellationToken.None));
        }
        finally
        {
            await AllowSignUpAsync(false);
        }
    }

    [Fact]
    public async Task ProviderSignUpIsRefusedWithoutEachThingItNeeds()
    {
        using var provider = new FakeProvider { Subject = $"new-{Guid.NewGuid():N}" };
        using var host = Host(provider);
        using var client = BrowserClient(host);
        var invite = await AddInviteAsync();
        (await SignInThroughProviderAsync(client, provider, "/")).Dispose();
        var username = DataModelSupport.UniqueName();
        var email = $"{username}@example.test";

        // The owner has not let the provider create accounts.
        using (var closed = await RegisterThroughProviderAsync(client, username, email, invite))
        {
            Assert.Equal(HttpStatusCode.Forbidden, closed.StatusCode);
        }

        await AllowSignUpAsync(true);
        try
        {
            // A browser the identity does not wait for has nothing to make the account from.
            using var stranger = BrowserClient(host);
            using (var withoutIdentity = await RegisterThroughProviderAsync(stranger, username, email, invite))
            {
                Assert.Equal(HttpStatusCode.BadRequest, withoutIdentity.StatusCode);
            }

            // Nor does the right browser without an invite that can still be used.
            foreach (var code in new[] { null, "no-such-invite", await AddInviteAsync(expired: true) })
            {
                using var withoutInvite = await RegisterThroughProviderAsync(client, username, email, code);
                Assert.Equal(HttpStatusCode.BadRequest, withoutInvite.StatusCode);
                Assert.True((await withoutInvite.Content.ReadFromJsonAsync<JsonElement>())
                    .GetProperty("errors").TryGetProperty("inviteCode", out _));
            }

            await using var scope = factory.OpenScope(out var db);
            Assert.False(await db.Users.AnyAsync(x => x.Username == username));
            Assert.False(await db.ExternalIdentities.AnyAsync(x => x.Subject == provider.Subject));
            // Still waiting, for the try that has everything.
            Assert.True(await db.PendingExternalIdentities.AnyAsync(x => x.Subject == provider.Subject));
        }
        finally
        {
            await AllowSignUpAsync(false);
        }
    }

    [Fact]
    public async Task ProviderSignUpNeverFoundsTheInstance()
    {
        using var empty = new StellarrWebApplicationFactory(loadTestData: false);
        using var provider = new FakeProvider();
        using var host = Host(provider, instance: empty);
        using var client = BrowserClient(host);
        await using (var scope = empty.OpenScope(out var db))
        {
            db.IdentityProviderSettings.Add(new IdentityProviderSetting { ProviderId = "default", SignUpAllowed = true });
            await db.SaveChangesAsync();
        }

        (await SignInThroughProviderAsync(client, provider, "/")).Dispose();
        using var register = await client.PostAsJsonAsync("/api/auth/register", new
        {
            external = true,
            username = "owner",
            displayName = "Owner",
            email = "owner@example.test",
            setupCode = empty.SetupCode,
            initialServer = new { name = "Nocna ekipa" },
        });

        Assert.Equal(HttpStatusCode.Conflict, register.StatusCode);
        await using var check = empty.OpenScope(out var checkDb);
        Assert.False(await checkDb.Users.AnyAsync());
    }

    [Fact]
    public async Task ProviderSignUpSignsInUnderTheProvidersSecondStepRules()
    {
        await SetRequirementAsync(MfaRequirements.Everyone);
        await AllowSignUpAsync(true);
        try
        {
            foreach (var trusted in new[] { false, true })
            {
                using var provider = new FakeProvider
                {
                    Subject = $"new-{Guid.NewGuid():N}",
                    Amr = ["pwd", "mfa"],
                    AuthTime = DateTimeOffset.UtcNow,
                };
                using var host = Host(provider, trustProviderMfa: trusted);
                using var client = BrowserClient(host);
                (await SignInThroughProviderAsync(client, provider, "/")).Dispose();
                var username = DataModelSupport.UniqueName();

                using var register = await RegisterThroughProviderAsync(
                    client, username, $"{username}@example.test", await AddInviteAsync());

                Assert.Equal(HttpStatusCode.Created, register.StatusCode);
                var answer = await register.Content.ReadFromJsonAsync<JsonElement>();
                await using var scope = factory.OpenScope(out var db);
                var user = await db.Users.SingleAsync(x => x.Username == username);
                if (!trusted)
                {
                    // The instance asks for its own step, set up here first.
                    Assert.Equal("mfa_enrollment_required", answer.GetProperty("status").GetString());
                    Assert.False(await db.UserSessions.AnyAsync(x => x.UserId == user.Id));
                    continue;
                }

                Assert.Equal(username, answer.GetProperty("username").GetString());
                var session = await db.UserSessions.SingleAsync(x => x.UserId == user.Id);
                Assert.Equal((AuthenticationFlow.ProviderEvidence, "oidc"), (session.MfaEvidence, session.PrimarySource));
            }
        }
        finally
        {
            await AllowSignUpAsync(false);
            await SetRequirementAsync(MfaRequirements.Optional);
        }
    }

    private static Task<HttpResponseMessage> RegisterThroughProviderAsync(
        HttpClient client,
        string username,
        string email,
        string? inviteCode) =>
        client.PostAsJsonAsync("/api/auth/register", new
        {
            external = true,
            username,
            displayName = "Ola Nowak",
            email,
            inviteCode,
        });

    private async Task AllowSignUpAsync(bool allowed)
    {
        await using var scope = factory.OpenScope(out var db);
        await db.IdentityProviderSettings.Where(x => x.ProviderId == "default").ExecuteDeleteAsync();
        if (allowed)
        {
            db.IdentityProviderSettings.Add(new IdentityProviderSetting
            {
                ProviderId = "default",
                SignUpAllowed = true,
                UpdatedAt = DateTime.UtcNow,
            });
            await db.SaveChangesAsync();
        }
    }

    /// <summary>An invite to the seeded server; an expired one when asked.</summary>
    private async Task<string> AddInviteAsync(bool expired = false)
    {
        await using var scope = factory.OpenScope(out var db);
        var code = $"invite{Guid.NewGuid():N}"[..16];
        db.ServerInvites.Add(new ServerInvite
        {
            Id = Guid.NewGuid(),
            ServerId = TestData.ServerId,
            CreatedById = Guid.Parse("30000000-0000-0000-0000-000000000001"),
            Code = code,
            ExpiresAt = expired ? DateTime.UtcNow.AddMinutes(-1) : null,
            CreatedAt = DateTime.UtcNow,
        });
        await db.SaveChangesAsync();
        return code;
    }
}
