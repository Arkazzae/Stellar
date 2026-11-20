using Microsoft.AspNetCore.SignalR;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;
using Stellarr.Api.Realtime;

namespace Stellarr.Api.Endpoints;

/// <summary>Creating, ordering, configuring and removing the channels of a server.</summary>
public static class ChannelEndpoints
{
    private const int MaxNameLength = 60;
    private const int MaxTopicLength = 1024;
    private const int MaxSlowModeSeconds = 6 * 60 * 60;
    private const int MaxHideAfterInactivityHours = 365 * 24;
    private const int MaxUserLimit = 99;

    private static readonly string[] AudioProfiles =
        ["economy", "balanced", "clarity", "premium", "studio"];

    public static IEndpointRouteBuilder MapChannelEndpoints(this IEndpointRouteBuilder endpoints)
    {
        var channels = endpoints.MapGroup("/api/admin/channels")
            .RequireAuthorization()
            .AddEndpointFilter<ServerMembershipFilter>();
        channels.MapPost("", CreateAsync);
        channels.MapPut("/order", ReorderAsync);
        channels.MapPatch("/{channelId:guid}", UpdateAsync);
        channels.MapDelete("/{channelId:guid}", DeleteAsync);
        return endpoints;
    }

    private static async Task<IResult> CreateAsync(
        CreateChannelRequest request,
        HttpContext http,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        var name = Normalize(request.Name);
        ChannelKind? kind = request.Kind switch
        {
            "text" => ChannelKind.Text,
            "voice" => ChannelKind.Voice,
            _ => null,
        };
        if (!IsValidName(name) || kind is null)
        {
            return Invalid("channel", messages.Get(MessageKey.ChannelNameOrKindInvalid));
        }

        if (await db.Channels.AnyAsync(x => x.ServerId == serverId && x.Name == name, cancellationToken))
        {
            return Results.Conflict();
        }

        if (request.SectionId is { } sectionId && !await db.Sections.AnyAsync(
                x => x.Id == sectionId && x.ServerId == serverId, cancellationToken))
        {
            return Results.NotFound();
        }

        // Inside a section, that section's override decides who may add to it.
        var userId = http.User.RequireUserId();
        var allowed = request.SectionId is { } targetSection
            ? await ServerAccess.HasForSectionAsync(
                db, userId, serverId, targetSection, ServerPrivileges.ManageChannels, cancellationToken)
            : await ServerAccess.HasAsync(db, userId, serverId, ServerPrivileges.ManageChannels, cancellationToken);
        if (!allowed)
        {
            return Results.Forbid();
        }

        var lastPosition = await db.Channels.Where(x => x.ServerId == serverId)
            .MaxAsync(x => (int?)x.Position, cancellationToken);
        var channel = new Channel
        {
            Id = Guid.NewGuid(),
            ServerId = serverId,
            Name = name,
            Kind = kind.Value,
            Position = (lastPosition ?? -1) + 1,
            SectionId = request.SectionId,
        };
        db.Channels.Add(channel);
        var sectionName = request.SectionId is { } inSection
            ? await db.Sections.Where(x => x.Id == inSection).Select(x => x.Name).SingleAsync(cancellationToken)
            : null;
        await ServerAudit.StageAsync(db, serverId, userId, new AuditEvent(
            "channel.create",
            messages.Get(MessageKey.AuditChannelCreated, ("name", channel.Name)),
            "channel", channel.Id, channel.Name,
            ServerAudit.Only(
                new AuditChange("kind", To: request.Kind),
                sectionName is null ? null : new AuditChange("section", To: sectionName))), cancellationToken);
        if (!await db.TrySaveChangesAsync(cancellationToken))
        {
            return Results.Conflict();
        }

        return Results.Created($"/api/admin/channels/{channel.Id}", channel.ToDto());
    }

    private static async Task<IResult> UpdateAsync(
        Guid channelId,
        UpdateChannelRequest request,
        HttpContext http,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        var channel = await db.Channels.FirstOrDefaultAsync(
            x => x.Id == channelId && x.ServerId == serverId, cancellationToken);
        if (channel is null)
        {
            return Results.NotFound();
        }

        if (!await ServerAccess.HasAsync(
                db, http.User.RequireUserId(), serverId, channel.Id, ServerPrivileges.ManageChannels, cancellationToken))
        {
            return Results.Forbid();
        }

        var name = request.Name is null ? null : Normalize(request.Name);
        if (name is not null && !IsValidName(name))
        {
            return Invalid("name", messages.Get(MessageKey.ChannelNameInvalid));
        }

        if (name is not null && await db.Channels.AnyAsync(
                x => x.ServerId == serverId && x.Id != channel.Id && x.Name == name, cancellationToken))
        {
            return Results.Conflict();
        }

        if (!AreSettingsInRange(request))
        {
            return Invalid("channel", messages.Get(MessageKey.ChannelSettingInvalid));
        }

        var textOnly = request.Topic is not null || request.SlowModeSeconds is not null;
        var voiceOnly = request.AudioProfile is not null || request.UserLimit is not null;
        if (channel.Kind == ChannelKind.Text ? voiceOnly : textOnly)
        {
            return Invalid("channel", messages.Get(MessageKey.ChannelSettingWrongKind));
        }

        var before = (channel.Name, channel.Topic, channel.SlowModeSeconds, channel.HideAfterInactivityHours,
            channel.AudioProfile, channel.UserLimit);
        channel.Name = name ?? channel.Name;
        channel.Topic = request.Topic?.Trim() ?? channel.Topic;
        channel.SlowModeSeconds = request.SlowModeSeconds ?? channel.SlowModeSeconds;
        channel.HideAfterInactivityHours = request.HideAfterInactivityHours ?? channel.HideAfterInactivityHours;
        channel.AudioProfile = request.AudioProfile ?? channel.AudioProfile;
        channel.UserLimit = request.UserLimit ?? channel.UserLimit;
        var changes = ServerAudit.Only(
            ServerAudit.Changed("name", before.Name, channel.Name),
            ServerAudit.Changed("topic", before.Topic ?? "", channel.Topic ?? ""),
            ServerAudit.Changed("slowMode", Number(before.SlowModeSeconds), Number(channel.SlowModeSeconds)),
            ServerAudit.Changed("hideAfter", Number(before.HideAfterInactivityHours), Number(channel.HideAfterInactivityHours)),
            ServerAudit.Changed("audioProfile", before.AudioProfile, channel.AudioProfile),
            ServerAudit.Changed("userLimit", Number(before.UserLimit), Number(channel.UserLimit)));
        if (changes.Count > 0)
        {
            await ServerAudit.StageAsync(db, serverId, http.User.RequireUserId(), new AuditEvent(
                "channel.update",
                messages.Get(MessageKey.AuditChannelUpdated, ("name", channel.Name)),
                "channel", channel.Id, channel.Name, changes), cancellationToken);
        }

        return await db.TrySaveChangesAsync(cancellationToken) ? Results.Ok(channel.ToDto()) : Results.Conflict();
    }

    /// <summary>
    /// Sets the order of every channel and, with a move, the section of one of
    /// them: one transaction and one broadcast, so nobody sees a channel moved
    /// to another group but not yet placed in it.
    /// </summary>
    private static async Task<IResult> ReorderAsync(
        ReorderChannelsRequest request,
        HttpContext http,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        var serverId = http.RequireActiveServerId();
        var userId = http.User.RequireUserId();
        if (!await ServerAccess.HasAsync(db, userId, serverId, ServerPrivileges.ManageChannels, cancellationToken))
        {
            return Results.Forbid();
        }

        var channels = await db.Channels.Where(x => x.ServerId == serverId).ToListAsync(cancellationToken);
        var movedSection = false;
        var order = request.ChannelIds.Select((id, position) => (id, position)).ToList();
        if (order.Select(x => x.id).Distinct().Count() != order.Count ||
            !order.Select(x => x.id).ToHashSet().SetEquals(channels.Select(x => x.Id)))
        {
            return Invalid("channelIds", messages.Get(MessageKey.ChannelOrderIncomplete));
        }

        if (request.Move is { } move)
        {
            // The order names every channel of the server, so a moved channel
            // missing from it belongs to another server.
            var moved = channels.FirstOrDefault(x => x.Id == move.ChannelId);
            if (moved is null || (move.SectionId is { } sectionId && !await db.Sections.AnyAsync(
                    x => x.Id == sectionId && x.ServerId == serverId, cancellationToken)))
            {
                return Results.NotFound();
            }

            // The same rules as assigning a section on its own: a section can
            // change who may see or edit the channel.
            var decision = await ServerAuthorization.MoveChannelAsync(
                db, userId, serverId, moved.Id, move.SectionId, cancellationToken);
            if (!decision.Allowed)
            {
                return Results.Forbid();
            }
            if (decision.ChangesPermissions && await InstanceServerFreshSecondStepFilter.RefusalAsync(http) is { } refusal)
            {
                return refusal;
            }

            if (moved.SectionId != move.SectionId)
            {
                var names = await db.Sections.Where(x => x.ServerId == serverId)
                    .ToDictionaryAsync(x => x.Id, x => x.Name, cancellationToken);
                string? NameOf(Guid? id) => id is { } known && names.TryGetValue(known, out var found) ? found : null;
                var to = NameOf(move.SectionId);
                await ServerAudit.StageAsync(db, serverId, userId, new AuditEvent(
                    "channel.move",
                    to is null
                        ? messages.Get(MessageKey.AuditChannelMovedOut, ("name", moved.Name))
                        : messages.Get(MessageKey.AuditChannelMoved, ("name", moved.Name), ("section", to)),
                    "channel", moved.Id, moved.Name,
                    [new AuditChange("section", NameOf(moved.SectionId), to)]), cancellationToken);
                movedSection = true;
            }

            moved.SectionId = move.SectionId;
        }

        var positions = order.ToDictionary(x => x.id, x => x.position);
        var reordered = channels.Any(x => x.Position != positions[x.Id]);
        foreach (var channel in channels)
        {
            channel.Position = positions[channel.Id];
        }

        if (reordered && !movedSection)
        {
            await ServerAudit.StageAsync(db, serverId, userId, new AuditEvent(
                "channel.reorder", messages.Get(MessageKey.AuditChannelsReordered)), cancellationToken);
        }

        await db.SaveChangesAsync(cancellationToken);
        await transaction.CommitAsync(cancellationToken);

        var layout = new ChannelLayoutDto(
            serverId.ToString(),
            channels.OrderBy(x => x.Position)
                .Select(x => new ChannelPlacementDto(x.Id.ToString(), x.SectionId?.ToString()))
                .ToArray());
        await hub.Clients.Group(AppHub.ServerGroup(serverId)).SendAsync("ChannelLayoutChanged", layout, cancellationToken);
        return Results.NoContent();
    }

    private static async Task<IResult> DeleteAsync(
        Guid channelId,
        HttpContext http,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        var channel = await db.Channels.AsNoTracking().FirstOrDefaultAsync(
            x => x.Id == channelId && x.ServerId == serverId, cancellationToken);
        if (channel is null)
        {
            return Results.NotFound();
        }

        if (!await ServerAccess.HasAsync(
                db, http.User.RequireUserId(), serverId, channel.Id, ServerPrivileges.ManageChannels, cancellationToken))
        {
            return Results.Forbid();
        }

        // Counting and deleting are one statement. SQLite runs two of them one
        // after the other, so the second sees what the first removed and two
        // requests cannot between them take the last channel of a kind.
        var deleted = await db.Channels
            .Where(x => x.Id == channel.Id &&
                db.Channels.Count(other => other.ServerId == serverId && other.Kind == channel.Kind) > 1)
            .ExecuteDeleteAsync(cancellationToken);
        if (deleted == 0)
        {
            return Results.Conflict();
        }

        await ServerAudit.StageAsync(db, serverId, http.User.RequireUserId(), new AuditEvent(
            "channel.delete",
            messages.Get(MessageKey.AuditChannelDeleted, ("name", channel.Name)),
            "channel", channel.Id, channel.Name), cancellationToken);
        await db.SaveChangesAsync(cancellationToken);
        return Results.NoContent();
    }

    private static string? Number(int? value) => value?.ToString(System.Globalization.CultureInfo.InvariantCulture);

    internal static string Normalize(string? name) => name?.Trim().ToLowerInvariant() ?? "";

    internal static bool IsValidName(string name) =>
        name.Length is >= 1 and <= MaxNameLength && name.All(c => char.IsLetterOrDigit(c) || c == '-');

    private static bool AreSettingsInRange(UpdateChannelRequest request) =>
        request.Topic?.Trim().Length is null or <= MaxTopicLength &&
        request.SlowModeSeconds is null or >= 0 and <= MaxSlowModeSeconds &&
        request.HideAfterInactivityHours is null or >= 0 and <= MaxHideAfterInactivityHours &&
        request.UserLimit is null or >= 0 and <= MaxUserLimit &&
        (request.AudioProfile is null || AudioProfiles.Contains(request.AudioProfile, StringComparer.Ordinal));

    private static IResult Invalid(string field, string message) =>
        Results.ValidationProblem(new Dictionary<string, string[]> { [field] = [message] });
}
