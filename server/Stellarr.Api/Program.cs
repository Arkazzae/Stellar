using Amazon.Runtime;
using Amazon.S3;
using Microsoft.AspNetCore.Authentication.Cookies;
using Microsoft.AspNetCore.DataProtection;
using Microsoft.AspNetCore.Http.Features;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Options;
using Stellarr.Api.Auth;
using Stellarr.Api.Auth.Security;
using Stellarr.Api.Backups;
using Stellarr.Api.Data;
using Stellarr.Api.Embeds;
using Stellarr.Api.Endpoints;
using Stellarr.Api.Integrations;
using Stellarr.Api.Integrations.Cloudflare;
using Stellarr.Api.Localization;
using Stellarr.Api.Logging;
using Stellarr.Api.Mail;
using Stellarr.Api.Maintenance;
using Stellarr.Api.Notifications;
using Stellarr.Api.Operations;
using Stellarr.Api.Realtime;
using Stellarr.Api.Secrets;
using Stellarr.Api.Storage;

var builder = WebApplication.CreateBuilder(args);

// Without a secret of its own the relay would accept credentials that anyone
// can compute from the published development secret.
if (!builder.Environment.IsDevelopment() &&
    !builder.Environment.IsEnvironment("Testing") &&
    string.IsNullOrWhiteSpace(builder.Configuration["Rtc:TurnSharedSecret"]))
{
    throw new InvalidOperationException("Rtc:TurnSharedSecret is required outside development.");
}

var dataRoot = builder.Configuration["DataRoot"] ?? "data";
var keysDirectory = Directory.CreateDirectory(Path.Combine(dataRoot, "keys"));

// Kestrel's default request cap is below a useful video attachment. Each kind
// of upload still has a limit of its own, checked once the form is parsed; this
// is only the outer ceiling, form overhead included.
const long MaxRequestBodyBytes = 128 * 1024 * 1024;
builder.WebHost.ConfigureKestrel(options => options.Limits.MaxRequestBodySize = MaxRequestBodyBytes);
builder.Services.Configure<FormOptions>(options => options.MultipartBodyLengthLimit = MaxRequestBodyBytes);

builder.Services.AddProblemDetails();
// Bound and checked once the host starts, so that a language set by a test or
// by the environment is the one validated.
builder.Services.AddSingleton<IValidateOptions<LanguageOptions>, LanguageOptionsValidator>();
builder.Services.AddOptions<LanguageOptions>()
    .Bind(builder.Configuration.GetSection(LanguageOptions.SectionName))
    .ValidateOnStart();
builder.Services.AddSingleton(services => new Messages(services.GetRequiredService<IOptions<LanguageOptions>>().Value));
// The operator panel reads the host's recent log lines from this provider.
builder.Services.AddHttpContextAccessor();
builder.Services.AddSingleton<CockpitLogBuffer>();
builder.Services.AddSingleton<RuntimeMetrics>();
builder.Services.AddHostedService<RuntimeSampler>();
builder.Services.AddSingleton<ILoggerProvider>(services => services.GetRequiredService<CockpitLogBuffer>());
// The container's process manager asks /health/ready every ten seconds; those
// requests log nothing below a warning, here or in the console.
builder.Services.AddQuietHealthProbes();
// The keys that protect the session cookie live on the data volume, so a
// restart or a new container does not sign everybody out.
builder.Services.AddDataProtection()
    .SetApplicationName("Stellarr")
    .PersistKeysToFileSystem(keysDirectory);

// Read when a context is built rather than here, so that the configuration a
// host is finally given, a test's included, names the database.
builder.Services.AddDbContext<AppDbContext>((services, options) => options.UseSqlite(DatabaseConnection.From(
    services.GetRequiredService<IConfiguration>().GetConnectionString("Database")
        ?? throw new InvalidOperationException("ConnectionStrings:Database is required."))));
builder.Services.AddSingleton<PasswordService>();
builder.Services.AddSingleton<PasskeyRelyingParty>();
builder.Services.AddSingleton<RegistrationGate>();
builder.Services.AddSingleton<SetupCode>();
builder.Services.AddSingleton<ServerMutationGate>();
builder.Services.AddSingleton<PresenceTracker>();
builder.Services.AddSingleton<HubConnectionRegistry>();
builder.Services.AddScoped<SecurityNotificationDispatcher>();
builder.Services.AddHostedService<SecurityNotificationWorker>();
builder.Services.AddHostedService<SessionRevocationWatcher>();
builder.Services.AddSingleton<TurnCredentialService>();
AddMediaServer(builder.Services);
builder.Services.AddSingleton<SlowModeGate>();
builder.Services.AddSingleton<SoundboardPlaybackGate>();
builder.Services.AddSingleton<HubThrottleGate>();
builder.Services.AddSingleton(TimeProvider.System);
builder.Services.AddSingleton(services => ClientDiscoveryIdentity.Load(services.GetRequiredService<IConfiguration>()));
builder.Services.AddSingleton<DesktopReleaseCatalogue>();
builder.Services.AddSingleton<AndroidReleaseCatalogue>();
builder.Services.AddSingleton<DesktopUpdateCatalogue>();
builder.Services.AddSignalR(options =>
{
    options.MaximumReceiveMessageSize = 160 * 1024;
    options.KeepAliveInterval = TimeSpan.FromSeconds(15);
    options.ClientTimeoutInterval = TimeSpan.FromSeconds(45);
});

// Uploads are files on the data volume. An S3-compatible store can hold them
// instead, for an instance whose disk is too small.
var storageSection = builder.Configuration.GetSection(StorageOptions.SectionName);
builder.Services.Configure<StorageOptions>(storageSection);
if (storageSection.GetValue(nameof(StorageOptions.Provider), StorageProvider.FileSystem) == StorageProvider.S3)
{
    builder.Services.AddSingleton<IAmazonS3>(services =>
    {
        var options = services.GetRequiredService<IOptions<StorageOptions>>().Value;
        if (string.IsNullOrWhiteSpace(options.Endpoint) || string.IsNullOrWhiteSpace(options.Bucket))
        {
            throw new InvalidOperationException("Storage:Provider is S3, so Storage:Endpoint and Storage:Bucket are required.");
        }

        var config = new AmazonS3Config
        {
            ServiceURL = options.Endpoint,
            ForcePathStyle = true,
            AuthenticationRegion = "us-east-1",
        };
        return new AmazonS3Client(new BasicAWSCredentials(options.AccessKey, options.SecretKey), config);
    });
    builder.Services.AddSingleton<IObjectStorage, S3ObjectStorage>();
}
else
{
    builder.Services.AddSingleton<IObjectStorage>(
        new FileSystemObjectStorage(Path.Combine(dataRoot, FileSystemObjectStorage.FolderName)));
}
builder.Services.Configure<UploadOptions>(builder.Configuration.GetSection(UploadOptions.SectionName));
builder.Services.AddSingleton<UploadAdmissionGate>();
builder.Services.AddScoped<MediaStorage>();

builder.Services.Configure<MaintenanceOptions>(builder.Configuration.GetSection(MaintenanceOptions.SectionName));
builder.Services.AddScoped<InstanceMaintenance>();
builder.Services.AddHostedService<MaintenanceWorker>();

builder.Services.Configure<MailOptions>(builder.Configuration.GetSection(MailOptions.SectionName));
builder.Services.AddScoped<InstanceSecrets>();
// Activity mail and push share one durable outbox. The dispatcher is separate
// from the send endpoints, so no provider is ever called inside a message
// transaction. Web Push is optional configuration; the extension registers a
// delivery that reports itself unavailable when it has no keys.
builder.Services.AddWebPushNotifications(builder.Configuration);
builder.Services.AddScoped<NotificationDeliveryDispatcher>();
builder.Services.AddHostedService<NotificationDeliveryWorker>();
builder.Services.AddSingleton<BackupJobs>();
builder.Services.AddOptions<AutomaticBackupOptions>()
    .Bind(builder.Configuration.GetSection(AutomaticBackupOptions.SectionName))
    .ValidateDataAnnotations()
    .ValidateOnStart();
builder.Services.AddOptions<CloudflareR2Options>()
    .Bind(builder.Configuration.GetSection(CloudflareR2Options.SectionName))
    .Validate(settings => settings.IsValid(requireCredentials: false),
        "Cloudflare R2 requires HTTPS, a safe prefix and the credentials of its connection mode.")
    .ValidateOnStart();
builder.Services.AddHttpClient(R2BackupDestination.WorkerClientName, http =>
{
    // A large archive may take a while to cross a slow link. The request
    // itself streams from disk; this only stops a stuck connection.
    http.Timeout = TimeSpan.FromHours(2);
    http.DefaultRequestHeaders.UserAgent.ParseAdd("Stellarr/2.0");
});
builder.Services.AddSingleton<IBackupDestination, R2BackupDestination>();
builder.Services.AddSingleton<CloudflareR2PolicyStore>();
builder.Services.AddSingleton<BackupPolicyStore>();
builder.Services.AddSingleton<AutomaticBackupWorker>();
builder.Services.AddHostedService(services => services.GetRequiredService<AutomaticBackupWorker>());
builder.Services.AddSingleton<SmtpEmailDelivery>();
builder.Services.AddScoped<IEmailDelivery, MailDelivery>();
// GIPHY takes its key in the query string: this client logs nothing, so the
// key cannot reach the log Cockpit shows.
builder.Services.AddHttpClient<GiphyClient>(http =>
{
    http.BaseAddress = new Uri("https://api.giphy.com/");
    http.Timeout = TimeSpan.FromSeconds(10);
    http.DefaultRequestHeaders.UserAgent.ParseAdd("Stellarr/2.0");
    http.DefaultRequestHeaders.Accept.ParseAdd("application/json");
}).RemoveAllLoggers();
builder.Services.AddHttpClient<ResendEmailDelivery>(http =>
{
    http.BaseAddress = new Uri("https://api.resend.com/");
    http.Timeout = TimeSpan.FromSeconds(15);
    http.DefaultRequestHeaders.UserAgent.ParseAdd("Stellarr/2.0");
    http.DefaultRequestHeaders.Accept.ParseAdd("application/json");
});

builder.Services.Configure<EmbedOptions>(builder.Configuration.GetSection(EmbedOptions.SectionName));
builder.Services.AddSingleton<EmbedInbox>();
builder.Services.AddScoped<EmbedBuilder>();
builder.Services.AddHostedService<EmbedWorker>();
builder.Services
    .AddHttpClient<ILinkUnfurler, LinkUnfurler>(ConfigureEmbedClient(
        // Ask for a page and say so: a host that would rather not be previewed
        // can answer 406 instead of guessing what we want.
        "text/html,application/xhtml+xml,image/*;q=0.8",
        "pl,en;q=0.8"))
    .ConfigurePrimaryHttpMessageHandler(CreateEmbedHandler);
builder.Services
    .AddHttpClient<IYouTubeMetadataResolver, YouTubeMetadataResolver>(ConfigureEmbedClient("application/json"))
    .ConfigurePrimaryHttpMessageHandler(CreateEmbedHandler);

var externalOidc = builder.Configuration
    .GetSection(ExternalOidcOptions.SectionName)
    .Get<ExternalOidcOptions>() ?? new ExternalOidcOptions();
builder.Services.AddSingleton(externalOidc);
builder.Services
    .AddAuthentication(CookieAuthenticationDefaults.AuthenticationScheme)
    .AddSessionCookie(
        allowPlainHttp: builder.Environment.IsDevelopment() || builder.Environment.IsEnvironment("Testing"))
    .AddExternalOidc(externalOidc);
builder.Services.AddAuthorization();
builder.Services.AddRateLimitPolicies(builder.Configuration, builder.Environment);

if (builder.Environment.IsDevelopment())
{
    builder.Services.AddCors(options => options.AddDefaultPolicy(policy => policy
        .WithOrigins("http://localhost:3000", "http://127.0.0.1:3000")
        .AllowAnyHeader()
        .AllowAnyMethod()
        .AllowCredentials()));
}

var app = builder.Build();

// Read once before anything is served, so an unusable media server or proxy
// setting stops the API at start instead of at the first call that needs it.
// They are read from the built host because only there do they see every
// source of configuration.
app.Services.GetRequiredService<MediaServerSettings>();
var trustedProxies = TrustedProxies.Load(app.Configuration);

// Outermost, so it counts the status the client gets, the exception handler's 500 included.
var runtimeMetrics = app.Services.GetRequiredService<RuntimeMetrics>();
app.Use(async (context, next) =>
{
    try
    {
        await next(context);
    }
    finally
    {
        runtimeMetrics.CountResponse(context.Response.StatusCode);
    }
});
// One line per request, outside the exception handler so that it logs the 500.
app.UseRequestLog();
app.UseExceptionHandler();
// Before the rate limiter, authentication and the hub, so all of them see the
// client's address and scheme rather than those of the container's nginx.
app.UseForwardedHeaders(trustedProxies);
if (app.Environment.IsDevelopment())
{
    app.UseCors();
}
app.UseRateLimiter();
app.UseClientGate();
app.UseAuthentication();
app.UseAuthorization();
// Sets how often WebSocket connections, the hub's included, are pinged.
// Reverse proxies in front of existing installations rely on it to keep an
// idle connection open.
app.UseWebSockets(new WebSocketOptions { KeepAliveInterval = TimeSpan.FromSeconds(20) });

app.MapGet("/health/live", () => Results.Ok(new { status = "live" }));
app.MapGet("/health/ready", async (AppDbContext db, CancellationToken cancellationToken) =>
    await db.Database.CanConnectAsync(cancellationToken)
        ? Results.Ok(new { status = "ready" })
        : Results.Problem(statusCode: StatusCodes.Status503ServiceUnavailable, title: "Database unavailable"));

app.MapClientDiscoveryEndpoints();
app.MapDesktopReleaseEndpoints();
app.MapAndroidReleaseEndpoints();
app.MapDesktopUpdateEndpoints();
app.MapAuthEndpoints();
app.MapMfaEndpoints();
app.MapPasskeySignInEndpoints();
app.MapEmailChangeEndpoints();
app.MapCliRecoveryEndpoints();
app.MapPasswordRecoveryEndpoints();
app.MapRegistrationEndpoints();
app.MapDesktopBrowserLoginEndpoints();
app.MapSessionEndpoints();
app.MapAccountEndpoints();
app.MapClientSettingsEndpoints();
app.MapAccountSecurityEndpoints();
app.MapProfileImageEndpoints();
app.MapBootstrapEndpoints();
app.MapUploadEndpoints();
app.MapAssetEndpoints();
app.MapMessageEndpoints();
app.MapMessageHistoryEndpoints();
app.MapMessageReactionEndpoints();
app.MapMessagePinEndpoints();
app.MapMessageThreadEndpoints();
app.MapMessageIndexEndpoints();
app.MapChannelReadEndpoints();
app.MapChannelEndpoints();
app.MapSectionEndpoints();
app.MapRoleEndpoints();
app.MapPermissionOverrideEndpoints();
app.MapServerDirectoryEndpoints();
app.MapServerEndpoints();
app.MapServerAdministrationEndpoints();
app.MapModerationEndpoints();
app.MapVoiceModerationEndpoints();
app.MapNotificationEndpoints();
app.MapPushNotificationEndpoints();
app.MapSocialEndpoints();
app.MapDirectMessageEndpoints();
app.MapEncryptionEndpoints();
app.MapEmojiEndpoints();
app.MapSoundboardEndpoints();
app.MapCockpitEndpoints();
app.MapHub<AppHub>("/hubs/app");

await InitializeApplicationAsync(app);
await app.RunAsync();

// Runs before the host starts serving, so hosted services and requests only
// ever see a database and an object store that are ready.
static async Task InitializeApplicationAsync(WebApplication app)
{
    var stopping = app.Lifetime.ApplicationStopping;
    await using (var scope = app.Services.CreateAsyncScope())
    {
        var db = scope.ServiceProvider.GetRequiredService<AppDbContext>();
        await DatabaseSchema.PrepareAsync(db, stopping);
        await IssueSetupCodeAsync(app, db, stopping);
        await app.Services.GetRequiredService<BackupPolicyStore>().LoadAsync(db, stopping);
        await app.Services.GetRequiredService<CloudflareR2PolicyStore>().LoadAsync(db, stopping);
        // A backup cut short by the last stop left a partial file; it is no backup.
        BackupJobs.RemoveLeftovers(BackupJobs.Directory(app.Configuration));
    }

    // The development seed cannot keep accounts connected, so it leaves a small
    // presence fixture on the volume. Reading it here means the first bootstrap
    // response already shows the mixed roster instead of everybody offline.
    DevelopmentPresenceSeed.Apply(app.Services);

    if (app.Configuration.GetValue<bool>("Storage:SkipInitialization"))
    {
        return;
    }

    // A store on another host may still be starting, or briefly unreachable.
    const int maxAttempts = 30;
    var storage = app.Services.GetRequiredService<IObjectStorage>();
    for (var attempt = 1; ; attempt++)
    {
        try
        {
            await storage.PrepareAsync(stopping);
            return;
        }
        catch (Exception exception) when (attempt < maxAttempts && !stopping.IsCancellationRequested)
        {
            StartupLog.StorageNotReady(app.Logger, attempt, maxAttempts, exception.Message);
            await Task.Delay(TimeSpan.FromSeconds(1), stopping);
        }
    }
}

// An instance without accounts is founded by whoever creates the first one.
// The code that registration asks for is shown here, in the log, which only
// the operator reads; a new one at every start until the owner exists.
static async Task IssueSetupCodeAsync(WebApplication app, AppDbContext db, CancellationToken cancellationToken)
{
    if (await db.Users.AnyAsync(cancellationToken))
    {
        return;
    }

    var code = app.Services.GetRequiredService<SetupCode>().Issue();
    var publicUrl = CockpitEndpoints.PublicUrlOf(app.Configuration);
    if (publicUrl.Length > 0)
    {
        StartupLog.SetupCodeIssued(app.Logger, code, $"{publicUrl}/setup");
    }
    else
    {
        StartupLog.SetupCodeIssuedWithoutAddress(app.Logger, code);
    }
}

// Stellarr Media, the selective forwarding unit every voice room goes through.
// The background services follow its events and close the sessions of seats
// that ended, from the moment the API starts.
static void AddMediaServer(IServiceCollection services)
{
    services.AddSingleton(provider => MediaServerSettings.Load(provider.GetRequiredService<IConfiguration>()));
    services.AddSingleton<MediaControlTiming>();
    services
        .AddHttpClient(MediaControlClient.HttpClientName, (provider, http) =>
        {
            http.BaseAddress = provider.GetRequiredService<MediaServerSettings>().ControlUrl;
            // Each call sets its own, shorter limit; this only catches one
            // that slipped past it.
            http.Timeout = TimeSpan.FromMinutes(1);
        })
        // The control API listens next to the API, never behind a proxy, and a
        // refused connection there means the media server is down, which is
        // worth knowing at once.
        .ConfigurePrimaryHttpMessageHandler(() => new SocketsHttpHandler
        {
            UseProxy = false,
            ConnectTimeout = TimeSpan.FromSeconds(2),
        })
        // The event poll alone would log four lines every 25 seconds, and a
        // media server that is down several each retry. The media code logs
        // the changes of state that matter itself.
        .RemoveAllLoggers();
    services.AddSingleton<MediaControlClient>();
    services.AddSingleton<MediaEventOutbox>();
    services.AddSingleton<MediaPlane>();
    services.AddSingleton<MediaSessionCloser>();
    services.AddSingleton<IMediaSessionCloser>(provider => provider.GetRequiredService<MediaSessionCloser>());
    services.AddHostedService(provider => provider.GetRequiredService<MediaSessionCloser>());
    services.AddHostedService<MediaEventPump>();
}

// Both clients that read other people's pages connect through the handler that
// refuses private addresses, and both give up after the same time.
static Action<IServiceProvider, HttpClient> ConfigureEmbedClient(string accept, string? acceptLanguage = null) =>
    (services, http) =>
    {
        var embeds = services.GetRequiredService<IOptions<EmbedOptions>>().Value;
        http.Timeout = TimeSpan.FromSeconds(Math.Max(1, embeds.TimeoutSeconds));
        http.DefaultRequestHeaders.UserAgent.ParseAdd(embeds.UserAgent);
        http.DefaultRequestHeaders.Accept.ParseAdd(accept);
        if (acceptLanguage is not null)
        {
            http.DefaultRequestHeaders.AcceptLanguage.ParseAdd(acceptLanguage);
        }
    };

static HttpMessageHandler CreateEmbedHandler(IServiceProvider services) =>
    LinkUnfurler.CreateHandler(services.GetRequiredService<IOptions<EmbedOptions>>().Value);

public partial class Program;
