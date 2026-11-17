using System.Net.Http.Json;
using System.Text.RegularExpressions;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth.Security;

namespace Stellarr.Api.Tests;

public sealed partial class AccountSecurityFlowTests
{
    [Theory]
    [InlineData("password-reset")]
    [InlineData("password-change")]
    [InlineData("recovery")]
    [InlineData("logout")]
    [InlineData("revoke-session")]
    [InlineData("legacy-link")]
    [InlineData("factor-enrollment")]
    public async Task PendingAddressChangeCannotOutliveItsSecurityAuthority(string action)
    {
        using var factory = new StellarrWebApplicationFactory();
        var account = await CreateAccountAsync(factory, "original@example.test");
        await SetEmailPolicyAsync(factory);
        using var initiating = Client(factory);
        await LoginAsync(initiating, account.Username);
        Assert.Equal("sent", await ChangeEmailAsync(initiating, "pending@example.test", Password));
        var token = LastToken(factory);
        using var other = Client(factory);

        switch (action)
        {
            case "password-reset":
                (await other.PostAsJsonAsync("/api/auth/password/forgot", new { email = "original@example.test" }))
                    .EnsureSuccessStatusCode();
                var resetToken = Uri.UnescapeDataString(Regex.Match(
                    factory.EmailDelivery.Messages[^1].Text, @"reset-password\?token=(\S+)").Groups[1].Value);
                (await other.PostAsJsonAsync("/api/auth/password/reset", new
                {
                    token = resetToken,
                    newPassword = "Recovered!2026",
                })).EnsureSuccessStatusCode();
                break;
            case "password-change":
                // Even preserving the initiating session must not preserve its old link.
                (await initiating.PostAsJsonAsync("/api/me/password", new
                {
                    currentPassword = Password,
                    newPassword = "Changed!2026",
                })).EnsureSuccessStatusCode();
                break;
            case "recovery":
                await using (var scope = factory.OpenScope(out var db))
                {
                    Assert.NotNull(await AccountRecovery.RecoverAsync(
                        db, account.Id, "regression", SecurityPolicy.Default, _ => Task.CompletedTask, CancellationToken.None));
                }
                break;
            case "logout":
                (await initiating.PostAsync("/api/auth/logout", null)).EnsureSuccessStatusCode();
                break;
            case "revoke-session":
                await LoginAsync(other, account.Username);
                (await other.PostAsync("/api/me/security/sessions/revoke-others", null)).EnsureSuccessStatusCode();
                break;
            case "legacy-link":
                await using (var scope = factory.OpenScope(out var db))
                {
                    await db.EmailChangeRequests.Where(x => x.UserId == account.Id)
                        .ExecuteUpdateAsync(setters => setters
                            .SetProperty(x => x.SecurityVersion, (int?)null)
                            .SetProperty(x => x.SessionId, (Guid?)null));
                }
                break;
            case "factor-enrollment":
                await EnrollTotpAsync(initiating);
                break;
        }

        Assert.Equal("invalid", await ConfirmEmailAsync(other, token));
        await using var check = factory.OpenScope(out var checkDb);
        Assert.Equal("original@example.test", (await checkDb.Users.SingleAsync(x => x.Id == account.Id)).Email);
    }
}
