using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Http.Connections;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// A server of its own with private places, made private the way an
/// administrator makes them: the base role denied viewChannel, and the
/// moderators' role allowed it. Projekty is a private text channel with a
/// conversation in it, Nocna zmiana a private voice channel, and Kadra a
/// private section with a channel of its own. Ogólny and Salon stay open.
/// </summary>
/// <remarks>
/// Ada moderates and sees everything. Jo holds the base role alone. Nils
/// holds a role that manages everything on the server, short of being an
/// administrator, and still sees none of the private places: what he may do
/// server-wide gives him nothing where he does not view. Maja owns the server.
/// </remarks>
internal sealed class PrivateChannelScenario : IAsyncDisposable
{
    internal static readonly Guid MajaId = Guid.Parse("30000000-0000-0000-0000-000000000001");
    internal static readonly Guid AdaId = Guid.Parse("30000000-0000-0000-0000-000000000002");
    internal static readonly Guid IlyaId = Guid.Parse("30000000-0000-0000-0000-000000000003");
    internal static readonly Guid NilsId = Guid.Parse("30000000-0000-0000-0000-000000000004");
    internal static readonly Guid JoId = Guid.Parse("30000000-0000-0000-0000-000000000005");
    internal static readonly Guid MemberRoleId = Guid.Parse("60000000-0000-0000-0000-000000000003");
    internal static readonly Guid ModeratorRoleId = Guid.Parse("60000000-0000-0000-0000-000000000002");

    internal static readonly Guid Hidden = TestData.ProjectsChannelId;
    internal static readonly Guid HiddenVoice = TestData.NightShiftChannelId;
    internal static readonly Guid Open = TestData.GeneralChannelId;
    internal static readonly Guid OpenVoice = TestData.VoiceChannelId;

    /// <summary>Every word that would tell somebody something of the private places.</summary>
    internal static readonly string[] Secrets = ["tajn", "projekty", "Nocna zmiana", "Kadra", "kadra-czat"];

    internal static readonly Dictionary<string, string> Passwords = new()
    {
        ["maja"] = "Aurora!2026",
        ["ada"] = "Quasar!2026",
        ["ilya"] = "Meteor!2026",
        ["nils"] = "Zenith!2026",
        ["jo"] = "Corona!2026",
    };

    internal static readonly Dictionary<string, Guid> UserIds = new()
    {
        ["maja"] = MajaId,
        ["ada"] = AdaId,
        ["ilya"] = IlyaId,
        ["nils"] = NilsId,
        ["jo"] = JoId,
    };

    private static readonly TimeSpan Patience = TimeSpan.FromSeconds(10);

    private readonly List<IDisposable> _clients = [];

    private PrivateChannelScenario(StellarrWebApplicationFactory factory) => Factory = factory;

    internal StellarrWebApplicationFactory Factory { get; }

    internal Guid ManagerRoleId { get; } = Guid.NewGuid();
    internal Guid HiddenSection { get; } = Guid.NewGuid();
    internal Guid SectionChannel { get; } = Guid.NewGuid();
    internal Guid Root { get; } = Guid.NewGuid();
    internal Guid Reply { get; } = Guid.NewGuid();
    internal Guid JosMessage { get; } = Guid.NewGuid();
    internal Guid NilsMessage { get; } = Guid.NewGuid();
    internal Guid Attachment { get; } = Guid.NewGuid();
    internal Guid Thumbnail { get; } = Guid.NewGuid();

    internal HttpClient Owner { get; private set; } = null!;
    internal HttpClient Ada { get; private set; } = null!;
    internal HttpClient Jo { get; private set; } = null!;
    internal HttpClient Nils { get; private set; } = null!;

    /// <summary>The scenario, made private; with <paramref name="hide"/> false, everything is open still.</summary>
    internal static async Task<PrivateChannelScenario> CreateAsync(
        bool hide = true,
        Action<IServiceCollection>? configureServices = null,
        IReadOnlyDictionary<string, string?>? configuration = null)
    {
        var scenario = new PrivateChannelScenario(new StellarrWebApplicationFactory(
            loadTestData: true, configuration: configuration, configureServices: configureServices));
        await scenario.SeedAsync(hide);
        scenario.Owner = await scenario.SignedInAsync("maja");
        scenario.Ada = await scenario.SignedInAsync("ada");
        scenario.Jo = await scenario.SignedInAsync("jo");
        scenario.Nils = await scenario.SignedInAsync("nils");
        return scenario;
    }

    private async Task SeedAsync(bool hide)
    {
        await WithDbAsync(async db =>
        {
            await db.ServerRoles.Where(x => x.Id == MemberRoleId).ExecuteUpdateAsync(s => s.SetProperty(x => x.Position, 3));
            db.ServerRoles.Add(new ServerRole
            {
                Id = ManagerRoleId,
                ServerId = TestData.ServerId,
                Name = "Kierownik",
                Color = "#123456",
                Position = 2,
                Permissions = ServerPrivileges.All & ~ServerPrivileges.Administrator & ~ServerPrivileges.ViewChannel,
            });
            db.ServerMemberRoles.Add(new ServerMemberRole { ServerId = TestData.ServerId, UserId = NilsId, RoleId = ManagerRoleId });
            db.Sections.Add(new Section { Id = HiddenSection, ServerId = TestData.ServerId, Name = "Kadra", Position = 0 });
            db.Channels.Add(new Channel
            {
                Id = SectionChannel, ServerId = TestData.ServerId, Name = "kadra-czat", Kind = ChannelKind.Text,
                Position = 5, SectionId = HiddenSection,
            });
            db.ImageAttachments.AddRange(
                Upload(Attachment, AdaId, "tajny-plan.png"),
                // A card's picture belongs to nobody, as the embed builder stores it.
                Upload(Thumbnail, Guid.Empty, "tajna-miniatura.png"));
            var now = DateTime.UtcNow;
            db.Messages.AddRange(
                new Message
                {
                    Id = Root, ChannelId = Hidden, AuthorId = AdaId, Content = "tajne plany @jo @nils", CreatedAt = now.AddMinutes(-5),
                    PinnedAt = now.AddMinutes(-4),
                    Attachments = [new MessageAttachment { AttachmentId = Attachment }],
                },
                new Message
                {
                    Id = Reply, ChannelId = Hidden, AuthorId = AdaId, Content = "tajna odpowiedź", CreatedAt = now.AddMinutes(-3),
                    ReplyToMessageId = Root,
                },
                new Message { Id = JosMessage, ChannelId = Hidden, AuthorId = JoId, Content = "tajne jo", CreatedAt = now.AddMinutes(-2) },
                new Message { Id = NilsMessage, ChannelId = Hidden, AuthorId = NilsId, Content = "tajne nils", CreatedAt = now.AddMinutes(-1) });
            await db.SaveChangesAsync();
            db.MessageEmbeds.Add(new MessageEmbed
            {
                Id = Guid.NewGuid(), MessageId = Root, Position = 0, Url = "https://example.com/tajne", Title = "tajne",
                Provider = "example.com", Domain = "example.com", ThumbnailAttachmentId = Thumbnail, CreatedAt = now,
            });
            db.MessageReactions.Add(new MessageReaction { MessageId = Root, UserId = JoId, Emoji = "👍", CreatedAt = now });
            await db.SaveChangesAsync();
            await db.ServerMembers.Where(x => x.ServerId == TestData.ServerId && x.UserId == NilsId)
                .ExecuteUpdateAsync(s => s.SetProperty(x => x.RoleId, ManagerRoleId));
        });

        // The bytes behind both pictures, so whoever may read them gets them.
        var storage = Factory.Services.GetRequiredService<Stellarr.Api.Storage.IObjectStorage>();
        foreach (var id in new[] { Attachment, Thumbnail })
        {
            using var bytes = new MemoryStream("tajne"u8.ToArray());
            await storage.PutAsync($"uploads/{id:N}", bytes, CancellationToken.None);
        }

        if (hide)
        {
            await HideAsync();
        }
    }

    /// <summary>Makes the private places private: the base role denied, the moderators allowed.</summary>
    internal Task HideAsync() => WithDbAsync(db =>
    {
        foreach (var (type, target) in new[]
                 {
                     (PermissionTargetType.Channel, Hidden),
                     (PermissionTargetType.Channel, HiddenVoice),
                     (PermissionTargetType.Section, HiddenSection),
                 })
        {
            db.PermissionOverrides.Add(RoleOverride(type, target, MemberRoleId, deny: ServerPrivileges.ViewChannel));
            db.PermissionOverrides.Add(RoleOverride(type, target, ModeratorRoleId, allow: ServerPrivileges.ViewChannel));
        }

        // Nils manages channels everywhere else; here he may not, so the
        // section is no more listed for him than its channel is.
        db.MemberPermissionOverrides.Add(new MemberPermissionOverride
        {
            Id = Guid.NewGuid(), ServerId = TestData.ServerId, UserId = NilsId,
            TargetType = PermissionTargetType.Section, TargetId = HiddenSection, DenyMask = ServerPrivileges.ManageChannels,
        });
        return db.SaveChangesAsync();
    });

    internal static PermissionOverride RoleOverride(
        PermissionTargetType type, Guid target, Guid roleId,
        ServerPrivileges allow = ServerPrivileges.None, ServerPrivileges deny = ServerPrivileges.None) => new()
    {
        Id = Guid.NewGuid(), ServerId = TestData.ServerId, TargetType = type, TargetId = target, RoleId = roleId,
        AllowMask = allow, DenyMask = deny,
    };

    private static ImageAttachment Upload(Guid id, Guid uploader, string name) => new()
    {
        Id = id,
        UploadedById = uploader,
        ObjectKey = $"uploads/{id:N}",
        FileName = name,
        ContentType = "image/png",
        SizeBytes = 12,
        CreatedAt = DateTime.UtcNow,
    };

    internal async Task WithDbAsync(Func<AppDbContext, Task> action)
    {
        await using var scope = Factory.OpenScope(out var db);
        await action(db);
    }

    internal async Task<T> WithDbAsync<T>(Func<AppDbContext, Task<T>> action)
    {
        await using var scope = Factory.OpenScope(out var db);
        return await action(db);
    }

    internal async Task<HttpClient> SignedInAsync(string username)
    {
        var client = Factory.CreateClient(new WebApplicationFactoryClientOptions { AllowAutoRedirect = false, HandleCookies = true });
        (await client.PostAsJsonAsync("/api/auth/login", new { username, password = Passwords[username] })).EnsureSuccessStatusCode();
        _clients.Add(client);
        return client;
    }

    /// <summary>A started hub connection of <paramref name="username"/> that records the named events.</summary>
    internal async Task<HubLog> ListenAsync(string username, params string[] methods)
    {
        using var client = Factory.CreateClient(new WebApplicationFactoryClientOptions { HandleCookies = false });
        using var login = await client.PostAsJsonAsync("/api/auth/login", new { username, password = Passwords[username] });
        login.EnsureSuccessStatusCode();
        var cookie = string.Join("; ", login.Headers.GetValues("Set-Cookie").Select(x => x.Split(';', 2)[0]));
        var connection = new HubConnectionBuilder()
            .WithUrl("https://localhost/hubs/app", options =>
            {
                options.Headers["Cookie"] = cookie;
                options.Transports = HttpTransportType.LongPolling;
                options.HttpMessageHandlerFactory = _ => Factory.Server.CreateHandler();
            })
            .Build();
        var log = new HubLog(connection, methods);
        var userId = UserIds[username].ToString();
        // The hub's first word to a connection comes after it has joined its groups.
        var joined = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        using var subscription = connection.On<JsonElement>("PresenceChanged", payload =>
        {
            if (payload.GetProperty("userId").GetString() == userId)
            {
                joined.TrySetResult();
            }
        });
        await connection.StartAsync();
        await joined.Task.WaitAsync(Patience);
        return log;
    }

    public ValueTask DisposeAsync()
    {
        foreach (var client in _clients)
        {
            client.Dispose();
        }

        Factory.Dispose();
        return ValueTask.CompletedTask;
    }

    /// <summary>The events a connection received, per method, in the order they arrived.</summary>
    internal sealed class HubLog : IAsyncDisposable
    {
        private readonly Dictionary<string, System.Threading.Channels.Channel<JsonElement>> _events = [];

        public HubLog(HubConnection connection, IEnumerable<string> methods)
        {
            Connection = connection;
            foreach (var method in methods)
            {
                var inbox = System.Threading.Channels.Channel.CreateUnbounded<JsonElement>();
                _events[method] = inbox;
                connection.On<JsonElement>(method, payload => inbox.Writer.TryWrite(payload));
            }
        }

        public HubConnection Connection { get; }

        public async Task<JsonElement> NextAsync(string method) =>
            await _events[method].Reader.ReadAsync().AsTask().WaitAsync(Patience);

        /// <summary>Events that arrived and that the test has not asked for yet.</summary>
        public int Pending(string method) => _events[method].Reader.Count;

        /// <summary>
        /// Lets everything sent before now arrive: a call to the hub is answered
        /// after the events the server sent this connection before it.
        /// </summary>
        public Task SettleAsync() => Connection.InvokeAsync<long>("GetRtcClock");

        public ValueTask DisposeAsync() => Connection.DisposeAsync();
    }
}
