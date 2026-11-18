using System.Threading.RateLimiting;
using Microsoft.AspNetCore.RateLimiting;

namespace Stellarr.Api.Auth;

/// <summary>
/// Named limits endpoints opt into. Each policy counts requests in a window of
/// its own: per client address, except for uploads, which share one budget
/// across the instance because their cost is bytes, not callers.
/// </summary>
public static class RateLimitPolicies
{
    public const string Login = "login";
    public const string LoginPoll = "login-poll";
    public const string AuthWrite = "auth-write";

    /// <summary>
    /// Changes to a signed-in or challenged account's own second factor. They
    /// send no mail and already run inside attempt and failure budgets, so
    /// they count apart from <see cref="AuthWrite"/>: one method added after
    /// a fresh proof takes three calls, which left the shared budget empty.
    /// </summary>
    public const string SecurityWrite = "security-write";

    /// <summary>
    /// The anonymous look at an invite before joining. A page asks once per
    /// visit; the budget leaves room for a few people behind one address and
    /// none for walking through codes.
    /// </summary>
    public const string InvitePreview = "invite-preview";
    public const string Uploads = "uploads";

    /// <summary>
    /// Tries of the encryption PIN, counted per address here and per account
    /// by <see cref="AccountAttemptLimitFilter"/>: the limiter runs before
    /// authentication, so it cannot tell whose PIN is being tried. Both only
    /// slow guessing down; the backup itself is deleted after ten wrong PINs.
    /// </summary>
    public const string PinUnlock = "pin-unlock";

    public static IServiceCollection AddRateLimitPolicies(
        this IServiceCollection services,
        IConfiguration configuration,
        IHostEnvironment environment)
    {
        // Tests sign in over and over from one address.
        int PermitLimit(string setting, int fallback) => environment.IsEnvironment("Testing")
            ? 10_000
            : Math.Max(1, configuration.GetValue<int?>($"RateLimiting:{setting}:PermitLimit") ?? fallback);

        var login = PermitLimit("Login", 10);
        // The desktop client polls for the browser's approval every second or
        // two. On the password budget that locked the whole address out of
        // signing in within seconds, so polling counts apart and wider.
        var loginPoll = PermitLimit("LoginPoll", 120);
        var authWrite = PermitLimit("AuthWrite", 5);
        var securityWrite = PermitLimit("SecurityWrite", 30);
        var invitePreview = PermitLimit("InvitePreview", 30);
        var uploadsPerMinute = PermitLimit("Uploads", 30);
        var pinUnlock = PermitLimit("PinUnlock", 5);
        var concurrentUploads = environment.IsEnvironment("Testing")
            ? 100
            : Math.Clamp(configuration.GetValue<int?>("Uploads:MaxConcurrentUploads") ?? 4, 1, 32);

        services.AddSingleton(new AccountAttemptLimiter(pinUnlock, PinUnlockWindow));
        return services.AddRateLimiter(options =>
        {
            options.RejectionStatusCode = StatusCodes.Status429TooManyRequests;
            AddPerAddressPolicy(options, Login, login, TimeSpan.FromMinutes(1));
            AddPerAddressPolicy(options, LoginPoll, loginPoll, TimeSpan.FromMinutes(1));
            AddPerAddressPolicy(options, AuthWrite, authWrite, TimeSpan.FromMinutes(15));
            AddPerAddressPolicy(options, SecurityWrite, securityWrite, TimeSpan.FromMinutes(15));
            AddPerAddressPolicy(options, InvitePreview, invitePreview, TimeSpan.FromMinutes(1));
            AddPerAddressPolicy(options, PinUnlock, pinUnlock, PinUnlockWindow);
            AddInstanceUploadPolicy(options, concurrentUploads, uploadsPerMinute);
        });
    }

    private static readonly TimeSpan PinUnlockWindow = TimeSpan.FromMinutes(15);

    /// <summary>
    /// Uploads that run at once, queued briefly, and uploads per minute, both
    /// counted for the whole instance so that no number of callers adds up to
    /// more than the disk and the network can take.
    /// </summary>
    private static void AddInstanceUploadPolicy(RateLimiterOptions options, int concurrent, int perMinute) =>
        options.AddPolicy(Uploads, _ => RateLimitPartition.Get(
            "instance",
            _ => RateLimiter.CreateChained(
                new ConcurrencyLimiter(new ConcurrencyLimiterOptions
                {
                    PermitLimit = concurrent,
                    QueueLimit = concurrent * 4,
                    QueueProcessingOrder = QueueProcessingOrder.OldestFirst,
                }),
                new FixedWindowRateLimiter(new FixedWindowRateLimiterOptions
                {
                    PermitLimit = perMinute,
                    Window = TimeSpan.FromMinutes(1),
                    QueueLimit = 0,
                    AutoReplenishment = true,
                }))));

    private static void AddPerAddressPolicy(RateLimiterOptions options, string name, int permitLimit, TimeSpan window) =>
        options.AddPolicy(name, context => RateLimitPartition.GetFixedWindowLimiter(
            context.Connection.RemoteIpAddress?.ToString() ?? "unknown",
            _ => new FixedWindowRateLimiterOptions
            {
                PermitLimit = permitLimit,
                Window = window,
                QueueLimit = 0,
                AutoReplenishment = true,
            }));
}

/// <summary>
/// A fixed window of attempts for each signed-in account, for routes that
/// <see cref="RateLimitPolicies"/> can only count per address.
/// </summary>
public sealed class AccountAttemptLimiter(int permitLimit, TimeSpan window) : IDisposable
{
    private readonly PartitionedRateLimiter<Guid> _limiter = PartitionedRateLimiter.Create<Guid, Guid>(
        userId => RateLimitPartition.GetFixedWindowLimiter(
            userId,
            _ => new FixedWindowRateLimiterOptions
            {
                PermitLimit = permitLimit,
                Window = window,
                QueueLimit = 0,
                AutoReplenishment = true,
            }));

    public bool TryAcquire(Guid userId)
    {
        using var lease = _limiter.AttemptAcquire(userId);
        return lease.IsAcquired;
    }

    public void Dispose() => _limiter.Dispose();
}

/// <summary>Answers 429, like the address limits do, once the account has used its attempts.</summary>
public sealed class AccountAttemptLimitFilter(AccountAttemptLimiter limiter) : IEndpointFilter
{
    public async ValueTask<object?> InvokeAsync(EndpointFilterInvocationContext context, EndpointFilterDelegate next) =>
        limiter.TryAcquire(context.HttpContext.User.RequireUserId())
            ? await next(context)
            : Results.StatusCode(StatusCodes.Status429TooManyRequests);
}
