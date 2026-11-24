// Imports a generated Discord server into a database with the first server's
// own writer, as the Rust runner does with the new one: from where an earlier,
// interrupted run stopped.
//
//   Harness DB GUILD.json SERVER_ID USER_ID
//
// STOP_AFTER_TX=n leaves the process before the n-th pause between
// transactions, that is at a boundary between two committed ones.
using System.Globalization;
using System.Net;
using System.Reflection;
using System.Text;
using System.Text.Json.Nodes;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Logging.Abstractions;
using Stellarr.Api.Data;
using Stellarr.Api.Integrations.Discord;

internal static class Program
{
    public static async Task<int> Main(string[] args)
    {
        AppDomain.CurrentDomain.AssemblyResolve += (_, e) =>
        {
            var path = Path.Combine(AppContext.BaseDirectory, new AssemblyName(e.Name).Name + ".dll");
            return File.Exists(path) ? Assembly.LoadFrom(path) : null;
        };
        try
        {
            SQLitePCL.Batteries_V2.Init();
            return await Run(args);
        }
        catch (Exception error)
        {
            // Said and left at once: a crash report would keep the container waiting.
            Console.Error.WriteLine(error);
            Environment.Exit(1);
            return 1;
        }
    }

    private static async Task<int> Run(string[] args)
    {
        var (database, guildPath) = (args[0], args[1]);
        var serverId = Guid.Parse(args[2]);
        var userId = Guid.Parse(args[3]);
        var stopAfter = int.Parse(Environment.GetEnvironmentVariable("STOP_AFTER_TX") ?? "0", CultureInfo.InvariantCulture);
        var pageSize = int.Parse(Environment.GetEnvironmentVariable("DISCORDW_PAGE") ?? "100", CultureInfo.InvariantCulture);
        var guild = JsonNode.Parse(File.ReadAllText(guildPath))!.AsObject();

        var clock = new Clock(stopAfter);
        var client = new DiscordClient(new HttpClient(new Fake(guild)), clock, NullLogger<DiscordClient>.Instance);
        var bot = client.ForBot(new DiscordBotToken("sample-bot-token"));
        var snapshot = await bot.ReadServerAsync("41771983423143936", CancellationToken.None);

        var options = new DbContextOptionsBuilder<AppDbContext>()
            .UseSqlite($"Data Source={database};Foreign Keys=True;Pooling=False").Options;
        await using var db = new AppDbContext(options);
        var writer = new DiscordImportWriter(db, clock);
        var import = await db.DiscordImports.AsNoTracking().Where(x => x.ServerId == serverId)
            .OrderBy(x => x.StartedAt).FirstOrDefaultAsync()
            ?? await writer.StartAsync(serverId, userId, snapshot, CancellationToken.None);
        await writer.WriteStructureAsync(import, snapshot, CancellationToken.None);

        foreach (var source in snapshot.Channels.Select(x => x.Id).Concat(snapshot.Threads.Select(x => x.Id)).ToList())
        {
            var record = await db.DiscordImportChannels.AsNoTracking()
                .SingleAsync(x => x.ServerId == serverId && x.DiscordId == source);
            // A channel read to its end may have been left before its links were
            // made: completing it again is how they are made.
            if (record.CompletedAt is not null)
            {
                await writer.CompleteChannelAsync(import, source, CancellationToken.None);
                continue;
            }

            var pins = new Dictionary<string, DateTimeOffset>(StringComparer.Ordinal);
            await foreach (var pin in bot.ReadPinsAsync(source))
            {
                pins[pin.Message.Id] = pin.PinnedAt;
            }

            var history = new List<MessageSnapshot>();
            await foreach (var message in bot.ReadMessagesAsync(source, record.OldestMessageId))
            {
                history.Add(message);
            }

            foreach (var page in history.Chunk(pageSize))
            {
                await writer.WriteMessagesAsync(import, snapshot, source, page, pins, CancellationToken.None);
            }

            await writer.CompleteChannelAsync(import, source, CancellationToken.None);
        }

        Console.WriteLine("done");
        return 0;
    }

    /// <summary>Every timer fires at once and moves the time on, so the pause between transactions costs nothing; the n-th ends the process.</summary>
    private sealed class Clock(int stopAfter) : TimeProvider
    {
        private readonly object _gate = new();
        private DateTimeOffset _now = new(2026, 11, 23, 12, 0, 0, TimeSpan.Zero);
        private int _timers;

        public override DateTimeOffset GetUtcNow()
        {
            lock (_gate) return _now;
        }

        public override ITimer CreateTimer(TimerCallback callback, object? state, TimeSpan dueTime, TimeSpan period)
        {
            lock (_gate)
            {
                _now += dueTime;
                if (stopAfter > 0 && ++_timers >= stopAfter)
                {
                    Environment.Exit(0);
                }
            }

            ThreadPool.QueueUserWorkItem(_ => callback(state));
            return new Spent();
        }

        private sealed class Spent : ITimer
        {
            public bool Change(TimeSpan dueTime, TimeSpan period) => false;
            public void Dispose() { }
            public ValueTask DisposeAsync() => ValueTask.CompletedTask;
        }
    }

    /// <summary>Discord's answers, from the generated server.</summary>
    private sealed class Fake(JsonObject guild) : HttpMessageHandler
    {
        private const string Id = "41771983423143936";

        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken) =>
            Task.FromResult(Respond(request.RequestUri!));

        private static int Compare(string left, string right) =>
            ulong.Parse(left, CultureInfo.InvariantCulture).CompareTo(ulong.Parse(right, CultureInfo.InvariantCulture));

        private HttpResponseMessage Respond(Uri uri)
        {
            var path = uri.AbsolutePath.Replace("/api/v10/", "", StringComparison.Ordinal);
            var query = System.Web.HttpUtility.ParseQueryString(uri.Query);
            var parts = path.Split('/');
            JsonNode body;
            if (path == $"guilds/{Id}")
            {
                body = new JsonObject { ["id"] = Id, ["name"] = "Krew" };
            }
            else if (path == $"guilds/{Id}/channels")
            {
                body = guild["channels"]!.DeepClone();
            }
            else if (path == $"guilds/{Id}/roles")
            {
                body = guild["roles"]!.DeepClone();
            }
            else if (path == $"guilds/{Id}/emojis" || path == $"guilds/{Id}/stickers")
            {
                body = new JsonArray();
            }
            else if (path == $"guilds/{Id}/members")
            {
                var after = query["after"];
                body = new JsonArray([.. guild["members"]!.AsArray()
                    .Where(x => after is null || Compare(x!["user"]!["id"]!.GetValue<string>(), after) > 0)
                    .Select(x => x!.DeepClone())]);
            }
            else if (path == $"guilds/{Id}/threads/active")
            {
                body = new JsonObject { ["threads"] = guild["threads"]!.DeepClone() };
            }
            else if (parts is ["channels", _, "threads", "archived", _])
            {
                body = new JsonObject { ["has_more"] = false, ["threads"] = new JsonArray() };
            }
            else if (parts is ["channels", var pinned, "messages", "pins"])
            {
                var pins = guild["pins"]![pinned]?.AsArray() ?? [];
                body = new JsonObject
                {
                    ["has_more"] = false,
                    ["items"] = new JsonArray([.. pins
                        .OrderByDescending(x => x![0]!.GetValue<string>(), StringComparer.Ordinal)
                        .Select(x => (JsonNode)new JsonObject { ["pinned_at"] = x![0]!.DeepClone(), ["message"] = x[1]!.DeepClone() })]),
                };
            }
            else if (parts is ["channels", var channel, "messages"])
            {
                var limit = int.Parse(query["limit"]!, CultureInfo.InvariantCulture);
                var before = query["before"];
                var messages = guild["messages"]![channel]?.AsArray() ?? [];
                body = new JsonArray([.. messages
                    .Where(x => before is null || Compare(x!["id"]!.GetValue<string>(), before) < 0)
                    .OrderByDescending(x => x!["id"]!.GetValue<string>(), Comparer<string>.Create(Compare))
                    .Take(limit)
                    .Select(x => x!.DeepClone())]);
            }
            else
            {
                return new HttpResponseMessage(HttpStatusCode.NotFound)
                {
                    Content = new StringContent($$"""{"message":"Not expected: {{path}}","code":0}""", Encoding.UTF8, "application/json"),
                };
            }

            return new HttpResponseMessage(HttpStatusCode.OK) { Content = new StringContent(body.ToJsonString(), Encoding.UTF8, "application/json") };
        }
    }
}
