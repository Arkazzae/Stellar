using System.Net.Mail;
using System.Text.RegularExpressions;
using Microsoft.AspNetCore.SignalR;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Auth.Security;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;
using Stellarr.Api.Realtime;

namespace Stellarr.Api.Endpoints;

/// <summary>
/// Creates accounts. The first one makes the instance: a server, its two
/// roles and channels, and an owner, and it takes the setup code from the
/// log. Every later one needs the code of an invite and joins the server that
/// invite belongs to.
///
/// A later one may also come from the identity a provider confirmed for this
/// browser, when the owner lets that provider create accounts: it has no
/// password, it is linked to the identity, and it signs in as a sign-in
/// through the provider would.
/// </summary>
public static partial class RegistrationEndpoints
{
    [GeneratedRegex(@"^[\p{L}\p{N}_.-]+$", RegexOptions.CultureInvariant)]
    private static partial Regex UsernameShape();

    public static IEndpointRouteBuilder MapRegistrationEndpoints(this IEndpointRouteBuilder endpoints)
    {
        // An instance waits for its owner until it has an account, whatever
        // happens to its servers later.
        endpoints.MapGet("/api/auth/registration", async (AppDbContext db, HttpContext http, CancellationToken ct) =>
        {
            http.Response.Headers.CacheControl = "no-store";
            return Results.Ok(new { setupRequired = !await db.Users.AnyAsync(ct) });
        });
        endpoints.MapPost("/api/auth/register", RegisterAsync).RequireRateLimiting(RateLimitPolicies.AuthWrite);
        return endpoints;
    }

    private static async Task<IResult> RegisterAsync(
        RegisterRequest request,
        AppDbContext db,
        PasswordService passwords,
        RegistrationGate gate,
        SetupCode setup,
        ExternalOidcOptions oidc,
        Messages messages,
        IHubContext<AppHub> hub,
        HttpContext context,
        CancellationToken cancellationToken)
    {
        var problems = Validate(request, messages);
        if (problems.Count > 0)
        {
            return Results.ValidationProblem(problems);
        }

        PendingExternalIdentity? pending = null;
        ExternalOidcProvider? provider = null;
        if (request.External)
        {
            pending = await PendingIdentities.FindAsync(db, context, cancellationToken);
            provider = pending is null ? null : oidc.Find(pending.ProviderId);
            if (pending is null || provider is null)
            {
                return Results.Problem(
                    statusCode: StatusCodes.Status400BadRequest,
                    title: messages.Get(MessageKey.ProviderSignUpExpired));
            }

            if (!await ProviderSignUp.AllowedAsync(db, provider.Id, cancellationToken))
            {
                return Results.Problem(
                    statusCode: StatusCodes.Status403Forbidden,
                    title: messages.Get(
                        MessageKey.ProviderSignUpClosed,
                        ("provider", AccountSecurityEndpoints.ProviderName(provider, messages))));
            }
        }

        var username = request.Username.Trim();
        var email = request.Email.Trim();
        var normalizedUsername = username.ToUpperInvariant();
        var normalizedEmail = email.ToUpperInvariant();

        // Whether the instance is still empty is read and acted on inside one
        // critical section, so two first registrations cannot both found it.
        await using var turn = await gate.EnterAsync(cancellationToken);

        // Empty means no account at all. An instance with accounts but no
        // server, which only an edit of the database could leave now, is not
        // empty: it takes an invite like any other, and nobody can found it
        // again.
        var founding = !await db.Users.AnyAsync(cancellationToken);
        if (founding && pending is not null)
        {
            // The owner's account is made with a password and the setup code.
            return Results.Problem(
                statusCode: StatusCodes.Status409Conflict,
                title: messages.Get(MessageKey.ProviderSignUpNotForSetup));
        }

        if (founding && !setup.Matches(request.SetupCode))
        {
            return Results.ValidationProblem(new Dictionary<string, string[]>
            {
                ["setupCode"] = [messages.Get(MessageKey.SetupCodeInvalid)],
            });
        }

        var taken = await FindTakenAsync(db, normalizedUsername, normalizedEmail, messages, cancellationToken);
        if (taken.Count > 0)
        {
            return Results.ValidationProblem(taken);
        }

        if (pending is not null && await IdentityTakenAsync(db, pending, cancellationToken))
        {
            return IdentityLinkedProblem(provider!, messages);
        }

        var now = DateTime.UtcNow;
        ServerInvite? invite = null;
        StellarrServer server;
        ServerRole role;
        if (founding)
        {
            var configuration = request.InitialServer ?? new CreateServerRequest("Stellarr", null);
            if (ServerDirectoryEndpoints.PrepareStarter(configuration, messages, out var starter) is { } problem)
            {
                return problem;
            }
            (server, role) = ServerDirectoryEndpoints.AddServer(db, starter, messages, now);
        }
        else
        {
            if (string.IsNullOrWhiteSpace(request.InviteCode))
            {
                return InviteProblem(messages.Get(MessageKey.InviteRequired));
            }

            invite = await ServerDirectoryEndpoints.FindUsableInviteAsync(db, request.InviteCode, now, cancellationToken);
            if (invite is null)
            {
                return InviteProblem(messages.Get(MessageKey.InviteUnusable));
            }

            if (await db.Servers.FindAsync([invite.ServerId], cancellationToken) is not { } invited)
            {
                return InviteProblem(messages.Get(MessageKey.InviteServerGone));
            }

            if (await ServerAccess.FindMemberRoleAsync(db, invited.Id, cancellationToken) is not { } memberRole)
            {
                return Results.Problem(
                    statusCode: StatusCodes.Status503ServiceUnavailable,
                    title: messages.Get(MessageKey.InstanceHasNoMemberRole));
            }

            (server, role) = (invited, memberRole);
        }

        var id = Guid.NewGuid();
        var user = new User
        {
            Id = id,
            Username = username,
            NormalizedUsername = normalizedUsername,
            DisplayName = request.DisplayName.Trim(),
            Email = email,
            NormalizedEmail = normalizedEmail,
            // The address counts as verified only when it is the one the
            // provider said it verified, not merely one it knows.
            ProviderVerifiedEmail = pending is { EmailVerified: true, Email: { } vouched } &&
                string.Equals(vouched, email, StringComparison.OrdinalIgnoreCase)
                    ? normalizedEmail
                    : null,
            PasswordHash = pending is null ? passwords.Hash(request.Password!) : null,
            AvatarColor = $"oklch(0.58 0.14 {(uint)id.GetHashCode() % 360})",
            CreatedAt = now,
        };
        db.Users.Add(user);
        if (pending is not null)
        {
            // A new account is told nothing by mail about its first link.
            IdentityLinking.Stage(db, context, id, provider!, pending.Issuer, pending.Subject, alertRecipient: null);
        }
        // The first account owns the new instance server; anybody else joins
        // it with the base role.
        MemberRoles.Admit(db, new ServerMember
        {
            ServerId = server.Id,
            UserId = id,
            RoleId = role.Id,
            OwnerSlot = role.IsOwner ? 1 : null,
            JoinedAt = now,
        }, isBaseRole: !role.IsOwner);
        var welcome = await ServerWelcome.CreateAsync(db, server.Id, user, now, messages, cancellationToken);
        if (invite is not null)
        {
            invite.Uses++;
        }

        // The first owner cannot skip an instance-wide requirement by
        // registering: the decision is made against the role just assigned.
        var policy = await MfaPolicy.LoadAsync(db, cancellationToken);
        bool? required = policy.MfaRequirement switch
        {
            MfaRequirements.Everyone => true,
            MfaRequirements.Admins => role.IsOwner ||
                (role.Permissions & ServerPrivileges.ManageServer) == ServerPrivileges.ManageServer,
            _ => null,
        };
        // An account made through a provider signs in as a sign-in through it
        // would: the provider's second step counts when the instance trusts
        // it and it is still recent now.
        var shortLived = pending?.ShortLived ?? false;
        var start = await AuthenticationFlow.StartAsync(
            db, context, user, pending is null ? "registration" : "oidc", policy, messages, cancellationToken,
            required,
            shortLived: shortLived,
            secondStepAtProvider: pending is not null && provider!.PassedSecondStep(
                pending.Amr.Split(',', StringSplitOptions.RemoveEmptyEntries), pending.AuthTime, DateTimeOffset.UtcNow));
        if (start.Session is { } session)
        {
            db.UserSessions.Add(session);
        }

        try
        {
            await db.SaveChangesAsync(cancellationToken);
        }
        catch (DbUpdateException exception) when (DatabaseErrors.IsUniqueConstraint(exception))
        {
            // Somebody else wrote the same username or address after our check:
            // answer as the check would have, not with a server error.
            db.ChangeTracker.Clear();
            taken = await FindTakenAsync(db, normalizedUsername, normalizedEmail, messages, cancellationToken);
            if (taken.Count > 0)
            {
                return Results.ValidationProblem(taken);
            }

            if (pending is not null && await IdentityTakenAsync(db, pending, cancellationToken))
            {
                return IdentityLinkedProblem(provider!, messages);
            }

            throw;
        }

        if (pending is not null)
        {
            // Used up: the identity is the account's now.
            await PendingIdentities.TakeAsync(db, context, cancellationToken, pending.Id);
        }

        if (founding)
        {
            // The instance has its owner. Later registrations do not ask for
            // the code, and none could pass with it anyway: it is gone.
            setup.Forget();
        }

        await ServerWelcome.PublishAsync(hub, server.Id, welcome, messages, CancellationToken.None);
        if (start.Session is not null)
        {
            // A registration with a password offers no choice and the account
            // stays signed in; one through a provider keeps the choice made
            // before leaving for it.
            await SessionAuthentication.SignInAsync(context, user, start.Session.Id, shortLived);
            return Results.Created("/api/me", user.ToDto("online"));
        }

        return Results.Created("/api/me", start.Challenge);
    }

    private static Dictionary<string, string[]> Validate(RegisterRequest request, Messages messages)
    {
        var errors = new Dictionary<string, string[]>(StringComparer.Ordinal);
        if (!request.External && PasswordPolicy.Violation(request.Password, messages) is { } passwordProblem)
        {
            errors["password"] = [passwordProblem];
        }

        var username = request.Username?.Trim() ?? "";
        if (username.Length is < 3 or > 32 || !UsernameShape().IsMatch(username))
        {
            errors["username"] = [messages.Get(MessageKey.UsernameInvalid)];
        }

        var displayName = request.DisplayName?.Trim() ?? "";
        if (displayName.Length is < 1 or > 64)
        {
            errors["displayName"] = [messages.Get(MessageKey.RegistrationDisplayNameLength)];
        }

        var email = request.Email?.Trim() ?? "";
        if (email.Length > 320 ||
            !MailAddress.TryCreate(email, out var address) ||
            !string.Equals(address.Address, email, StringComparison.OrdinalIgnoreCase))
        {
            errors["email"] = [messages.Get(MessageKey.EmailInvalid)];
        }

        return errors;
    }

    private static async Task<Dictionary<string, string[]>> FindTakenAsync(
        AppDbContext db,
        string normalizedUsername,
        string normalizedEmail,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var existing = await db.Users.AsNoTracking()
            .Where(x => x.NormalizedUsername == normalizedUsername || x.NormalizedEmail == normalizedEmail)
            .Select(x => new { x.NormalizedUsername, x.NormalizedEmail })
            .ToArrayAsync(cancellationToken);

        var errors = new Dictionary<string, string[]>(StringComparer.Ordinal);
        if (existing.Any(x => x.NormalizedUsername == normalizedUsername))
        {
            errors["username"] = [messages.Get(MessageKey.UsernameTaken)];
        }

        if (existing.Any(x => x.NormalizedEmail == normalizedEmail))
        {
            errors["email"] = [messages.Get(MessageKey.EmailTaken)];
        }

        return errors;
    }

    private static Task<bool> IdentityTakenAsync(
        AppDbContext db,
        PendingExternalIdentity pending,
        CancellationToken cancellationToken) =>
        db.ExternalIdentities.AsNoTracking()
            .AnyAsync(x => x.Issuer == pending.Issuer && x.Subject == pending.Subject, cancellationToken);

    /// <summary>
    /// The identity got linked to an account meanwhile. It is the person's
    /// own, so saying so tells them nothing about anybody else.
    /// </summary>
    private static IResult IdentityLinkedProblem(ExternalOidcProvider provider, Messages messages) =>
        Results.Problem(
            statusCode: StatusCodes.Status409Conflict,
            title: messages.Get(
                MessageKey.ProviderIdentityAlreadyLinked,
                ("provider", AccountSecurityEndpoints.ProviderName(provider, messages))));

    private static IResult InviteProblem(string message) =>
        Results.ValidationProblem(new Dictionary<string, string[]> { ["inviteCode"] = [message] });
}
