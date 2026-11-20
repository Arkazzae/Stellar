using System.Globalization;
using Microsoft.AspNetCore.SignalR;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;
using Stellarr.Api.Realtime;
using Stellarr.Api.Storage;

namespace Stellarr.Api.Endpoints;

/// <summary>
/// The sound catalogue of a server, its order and the two rules that decide
/// who may play it. A sound is an uploaded audio attachment plus a trim, which
/// never alters the stored file.
/// </summary>
public static class SoundboardEndpoints
{
    private const double MinClipSeconds = 0.15;
    private const double MaxClipSeconds = 15;

    // Trims come from a browser that measured the clip in floating point.
    private const double TrimTolerance = 0.01;

    public static IEndpointRouteBuilder MapSoundboardEndpoints(this IEndpointRouteBuilder endpoints)
    {
        endpoints.MapGroup("/api/soundboard")
            .RequireAuthorization()
            .AddEndpointFilter<ServerMembershipFilter>()
            .MapGet("", GetAsync);

        var admin = endpoints.MapGroup("/api/admin/soundboard")
            .RequireAuthorization()
            .AddEndpointFilter<ServerMembershipFilter>();
        admin.MapPost("/sounds", CreateAsync);
        admin.MapPatch("/sounds/{soundId:guid}", UpdateAsync);
        admin.MapDelete("/sounds/{soundId:guid}", DeleteAsync);
        admin.MapPut("/order", ReorderAsync);
        admin.MapPatch("/settings", UpdateSettingsAsync);
        return endpoints;
    }

    private static async Task<IResult> GetAsync(
        HttpContext http,
        AppDbContext db,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        var server = await db.Servers.AsNoTracking().SingleAsync(x => x.Id == serverId, cancellationToken);
        var sounds = await db.SoundboardSounds.AsNoTracking()
            .Include(x => x.Attachment)
            .Where(x => x.ServerId == serverId)
            .OrderBy(x => x.Position)
            .ThenBy(x => x.CreatedAt)
            .ToListAsync(cancellationToken);
        var canManage = await CanManageAsync(http, db, serverId, cancellationToken);
        var canPlay = await SoundboardAccess.CanPlayAsync(
            db, http.User.RequireUserId(), serverId, server.SoundboardEveryoneCanPlay, cancellationToken);
        return Results.Ok(new SoundboardSnapshotDto(
            sounds.Select(x => x.ToDto()).ToArray(),
            Settings(server),
            canManage,
            canPlay));
    }

    private static async Task<IResult> CreateAsync(
        CreateSoundboardSoundRequest request,
        HttpContext http,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        if (!await CanManageAsync(http, db, serverId, cancellationToken))
        {
            return Results.Forbid();
        }

        var userId = http.User.RequireUserId();
        var attachment = await db.ImageAttachments.FirstOrDefaultAsync(
            x => x.Id == request.AttachmentId && x.UploadedById == userId, cancellationToken);
        if (attachment is not { DurationSeconds: { } duration } ||
            !attachment.ContentType.StartsWith("audio/", StringComparison.OrdinalIgnoreCase))
        {
            return Results.ValidationProblem(new Dictionary<string, string[]>
            {
                ["attachmentId"] = [messages.Get(MessageKey.SoundAttachmentInvalid)],
            });
        }

        if (await db.SoundboardSounds.AnyAsync(x => x.AttachmentId == attachment.Id, cancellationToken))
        {
            return Results.Conflict();
        }

        if (!TryValidate(
                request.Name, request.Glyph, request.TrimStartSeconds, request.TrimEndSeconds, duration,
                out var name, out var glyph))
        {
            return InvalidSound(messages);
        }

        var lastPosition = await db.SoundboardSounds.Where(x => x.ServerId == serverId)
            .MaxAsync(x => (int?)x.Position, cancellationToken);
        var sound = new SoundboardSound
        {
            Id = Guid.NewGuid(),
            ServerId = serverId,
            AttachmentId = attachment.Id,
            CreatedById = userId,
            Name = name,
            Glyph = glyph,
            TrimStartSeconds = Math.Round(request.TrimStartSeconds, 3),
            TrimEndSeconds = Math.Round(request.TrimEndSeconds, 3),
            Position = (lastPosition ?? -1) + 1,
            CreatedAt = DateTime.UtcNow,
            Attachment = attachment,
        };
        db.SoundboardSounds.Add(sound);
        await ServerAudit.StageAsync(db, serverId, userId, new AuditEvent(
            "sound.create",
            messages.Get(MessageKey.AuditSoundAdded, ("name", sound.Name)),
            "sound", sound.Id, sound.Name), cancellationToken);
        // Two requests may pick the same attachment between the check and here.
        if (!await db.TrySaveChangesAsync(cancellationToken))
        {
            return Results.Conflict();
        }

        await NotifyChangedAsync(hub, serverId, cancellationToken);
        return Results.Created($"/api/admin/soundboard/sounds/{sound.Id}", sound.ToDto());
    }

    private static async Task<IResult> UpdateAsync(
        Guid soundId,
        UpdateSoundboardSoundRequest request,
        HttpContext http,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        if (!await CanManageAsync(http, db, serverId, cancellationToken))
        {
            return Results.Forbid();
        }

        var sound = await db.SoundboardSounds.Include(x => x.Attachment)
            .FirstOrDefaultAsync(x => x.Id == soundId && x.ServerId == serverId, cancellationToken);
        if (sound is null)
        {
            return Results.NotFound();
        }

        var start = request.TrimStartSeconds ?? sound.TrimStartSeconds;
        var end = request.TrimEndSeconds ?? sound.TrimEndSeconds;
        if (!TryValidate(
                request.Name ?? sound.Name,
                request.Glyph ?? sound.Glyph,
                start,
                end,
                sound.Attachment.DurationSeconds ?? sound.TrimEndSeconds,
                out var name,
                out var glyph))
        {
            return InvalidSound(messages);
        }

        var before = (sound.Name, sound.Glyph, sound.TrimStartSeconds, sound.TrimEndSeconds);
        sound.Name = name;
        sound.Glyph = glyph;
        sound.TrimStartSeconds = Math.Round(start, 3);
        sound.TrimEndSeconds = Math.Round(end, 3);
        var changes = ServerAudit.Only(
            ServerAudit.Changed("name", before.Name, sound.Name),
            ServerAudit.Changed("glyph", before.Glyph, sound.Glyph),
            before.TrimStartSeconds == sound.TrimStartSeconds && before.TrimEndSeconds == sound.TrimEndSeconds
                ? null
                : new AuditChange("trim", Seconds(before.TrimStartSeconds, before.TrimEndSeconds), Seconds(sound.TrimStartSeconds, sound.TrimEndSeconds)));
        if (changes.Count > 0)
        {
            await ServerAudit.StageAsync(db, serverId, http.User.RequireUserId(), new AuditEvent(
                "sound.update",
                messages.Get(MessageKey.AuditSoundUpdated, ("name", sound.Name)),
                "sound", sound.Id, sound.Name, changes), cancellationToken);
        }

        await db.SaveChangesAsync(cancellationToken);
        await NotifyChangedAsync(hub, serverId, cancellationToken);
        return Results.Ok(sound.ToDto());
    }

    private static async Task<IResult> DeleteAsync(
        Guid soundId,
        HttpContext http,
        AppDbContext db,
        MediaStorage mediaStorage,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        if (!await CanManageAsync(http, db, serverId, cancellationToken))
        {
            return Results.Forbid();
        }

        var sound = await db.SoundboardSounds.FirstOrDefaultAsync(
            x => x.Id == soundId && x.ServerId == serverId, cancellationToken);
        if (sound is null)
        {
            return Results.NoContent();
        }

        // The audio is collected below, so a join sound that names this clip
        // has to go with it: left behind, it would be a setting that can only
        // ever mean silence.
        await using (var transaction = await db.Database.BeginTransactionAsync(cancellationToken))
        {
            db.SoundboardSounds.Remove(sound);
            await ServerAudit.StageAsync(db, serverId, http.User.RequireUserId(), new AuditEvent(
                "sound.delete",
                messages.Get(MessageKey.AuditSoundDeleted, ("name", sound.Name)),
                "sound", sound.Id, sound.Name), cancellationToken);
            await db.SaveChangesAsync(cancellationToken);
            await db.UserPreferences.Where(x => x.JoinSoundId == soundId)
                .ExecuteUpdateAsync(setters => setters.SetProperty(x => x.JoinSoundId, (Guid?)null), cancellationToken);
            await transaction.CommitAsync(cancellationToken);
        }

        await mediaStorage.QueueAttachmentIfUnreferencedAsync(
            $"/assets/{sound.AttachmentId}", uploadedById: null, DateTime.UtcNow, cancellationToken);
        await NotifyChangedAsync(hub, serverId, cancellationToken);
        return Results.NoContent();
    }

    private static async Task<IResult> ReorderAsync(
        ReorderSoundboardSoundsRequest request,
        HttpContext http,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        if (!await CanManageAsync(http, db, serverId, cancellationToken))
        {
            return Results.Forbid();
        }

        var sounds = await db.SoundboardSounds.Where(x => x.ServerId == serverId).ToListAsync(cancellationToken);
        var order = request.SoundIds ?? [];
        if (order.Distinct().Count() != order.Count ||
            !order.ToHashSet().SetEquals(sounds.Select(x => x.Id)))
        {
            return Results.ValidationProblem(new Dictionary<string, string[]>
            {
                ["soundIds"] = [messages.Get(MessageKey.SoundOrderIncomplete)],
            });
        }

        var positions = order.Select((id, position) => (id, position)).ToDictionary(x => x.id, x => x.position);
        foreach (var sound in sounds)
        {
            sound.Position = positions[sound.Id];
        }

        await db.SaveChangesAsync(cancellationToken);
        await NotifyChangedAsync(hub, serverId, cancellationToken);
        return Results.NoContent();
    }

    private static async Task<IResult> UpdateSettingsAsync(
        UpdateSoundboardSettingsRequest request,
        HttpContext http,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        if (!await CanManageAsync(http, db, serverId, cancellationToken))
        {
            return Results.Forbid();
        }

        var server = await db.Servers.SingleAsync(x => x.Id == serverId, cancellationToken);
        var before = (server.SoundboardEveryoneCanPlay, server.SoundboardRateLimited);
        server.SoundboardEveryoneCanPlay = request.EveryoneCanPlay ?? server.SoundboardEveryoneCanPlay;
        server.SoundboardRateLimited = request.RateLimited ?? server.SoundboardRateLimited;
        var changes = ServerAudit.Only(
            ServerAudit.Changed("everyoneCanPlay", OnOff(before.SoundboardEveryoneCanPlay), OnOff(server.SoundboardEveryoneCanPlay)),
            ServerAudit.Changed("rateLimited", OnOff(before.SoundboardRateLimited), OnOff(server.SoundboardRateLimited)));
        if (changes.Count > 0)
        {
            await ServerAudit.StageAsync(db, serverId, http.User.RequireUserId(), new AuditEvent(
                "soundboard.settings", messages.Get(MessageKey.AuditSoundboardSettings), Changes: changes), cancellationToken);
        }

        await db.SaveChangesAsync(cancellationToken);
        await NotifyChangedAsync(hub, serverId, cancellationToken);
        return Results.Ok(Settings(server));
    }

    private static string OnOff(bool value) => value ? "on" : "off";

    private static string Seconds(double start, double end) =>
        string.Create(System.Globalization.CultureInfo.InvariantCulture, $"{start:0.##}–{end:0.##}");

    private static Task<bool> CanManageAsync(
        HttpContext http,
        AppDbContext db,
        Guid serverId,
        CancellationToken cancellationToken) =>
        ServerAccess.HasAsync(db, http.User.RequireUserId(), serverId, ServerPrivileges.ManageServer, cancellationToken);

    private static SoundboardSettingsDto Settings(StellarrServer server) =>
        new(server.SoundboardEveryoneCanPlay, server.SoundboardRateLimited);

    private static bool TryValidate(
        string? rawName,
        string? rawGlyph,
        double start,
        double end,
        double duration,
        out string name,
        out string glyph)
    {
        name = rawName?.Trim() ?? "";
        glyph = rawGlyph?.Trim() ?? "";
        return name.Length is >= 1 and <= 32 &&
            glyph.Length is >= 1 and <= 16 &&
            double.IsFinite(start) && double.IsFinite(end) &&
            start >= 0 && end <= duration + TrimTolerance &&
            end - start >= MinClipSeconds && end - start <= MaxClipSeconds + TrimTolerance;
    }

    private static IResult InvalidSound(Messages messages) => Results.ValidationProblem(new Dictionary<string, string[]>
    {
        ["sound"] =
        [
            messages.Get(
                MessageKey.SoundInvalid,
                ("min", MinClipSeconds.ToString("0.##", CultureInfo.InvariantCulture)),
                ("max", MaxClipSeconds.ToString("0", CultureInfo.InvariantCulture))),
        ],
    });

    private static Task NotifyChangedAsync(IHubContext<AppHub> hub, Guid serverId, CancellationToken cancellationToken) =>
        hub.Clients.Group(AppHub.ServerGroup(serverId))
            .SendAsync("SoundboardChanged", new { serverId = serverId.ToString() }, cancellationToken);
}
