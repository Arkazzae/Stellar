using Microsoft.AspNetCore.SignalR;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;
using Stellarr.Api.Realtime;

namespace Stellarr.Api.Endpoints;

/// <summary>
/// The collapsible groups of the channel rail. A section is a label over the
/// server-wide channel order; it owns no order of its channels, and a channel
/// is filed under one together with that order (<see cref="ChannelEndpoints"/>).
/// </summary>
public static class SectionEndpoints
{
    private const int MaxNameLength = 60;

    public static IEndpointRouteBuilder MapSectionEndpoints(this IEndpointRouteBuilder endpoints)
    {
        var sections = endpoints.MapGroup("/api/admin/sections")
            .RequireAuthorization()
            .AddEndpointFilter<ServerMembershipFilter>();
        sections.MapPost("", CreateAsync);
        sections.MapPut("/order", ReorderAsync);
        sections.MapPatch("/{sectionId:guid}", RenameAsync);
        sections.MapDelete("/{sectionId:guid}", DeleteAsync);
        return endpoints;
    }

    private static async Task<IResult> CreateAsync(
        CreateSectionRequest request,
        HttpContext http,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        if (!await ServerAccess.HasAsync(
                db, http.User.RequireUserId(), serverId, ServerPrivileges.ManageChannels, cancellationToken))
        {
            return Results.Forbid();
        }

        var name = request.Name?.Trim() ?? "";
        if (!IsValidName(name))
        {
            return InvalidName(messages);
        }

        var lastPosition = await db.Sections.Where(x => x.ServerId == serverId)
            .MaxAsync(x => (int?)x.Position, cancellationToken);
        var section = new Section
        {
            Id = Guid.NewGuid(),
            ServerId = serverId,
            Name = name,
            Position = (lastPosition ?? -1) + 1,
        };
        db.Sections.Add(section);
        await ServerAudit.StageAsync(db, serverId, http.User.RequireUserId(), new AuditEvent(
            "section.create",
            messages.Get(MessageKey.AuditSectionCreated, ("name", section.Name)),
            "section", section.Id, section.Name), cancellationToken);
        await db.SaveChangesAsync(cancellationToken);
        return Results.Created($"/api/admin/sections/{section.Id}", section.ToDto());
    }

    private static async Task<IResult> RenameAsync(
        Guid sectionId,
        UpdateSectionRequest request,
        HttpContext http,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        var section = await db.Sections.FirstOrDefaultAsync(
            x => x.Id == sectionId && x.ServerId == serverId, cancellationToken);
        if (section is null)
        {
            return Results.NotFound();
        }

        if (!await ServerAccess.HasForSectionAsync(
                db, http.User.RequireUserId(), serverId, section.Id, ServerPrivileges.ManageChannels, cancellationToken))
        {
            return Results.Forbid();
        }

        var name = request.Name?.Trim();
        if (name is not null && !IsValidName(name))
        {
            return InvalidName(messages);
        }

        var before = section.Name;
        section.Name = name ?? section.Name;
        if (before != section.Name)
        {
            await ServerAudit.StageAsync(db, serverId, http.User.RequireUserId(), new AuditEvent(
                "section.update",
                messages.Get(MessageKey.AuditSectionRenamed, ("name", section.Name)),
                "section", section.Id, section.Name,
                [new AuditChange("name", before, section.Name)]), cancellationToken);
        }

        await db.SaveChangesAsync(cancellationToken);
        return Results.Ok(section.ToDto());
    }

    private static async Task<IResult> ReorderAsync(
        ReorderSectionsRequest request,
        HttpContext http,
        AppDbContext db,
        IHubContext<AppHub> hub,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        if (!await ServerAccess.HasAsync(
                db, http.User.RequireUserId(), serverId, ServerPrivileges.ManageChannels, cancellationToken))
        {
            return Results.Forbid();
        }

        var sections = await db.Sections.Where(x => x.ServerId == serverId).ToListAsync(cancellationToken);
        var order = request.SectionIds.Select((id, position) => (id, position)).ToList();
        if (order.Select(x => x.id).Distinct().Count() != order.Count ||
            !order.Select(x => x.id).ToHashSet().SetEquals(sections.Select(x => x.Id)))
        {
            return Results.ValidationProblem(new Dictionary<string, string[]>
            {
                ["sectionIds"] = [messages.Get(MessageKey.SectionOrderIncomplete)],
            });
        }

        var positions = order.ToDictionary(x => x.id, x => x.position);
        var reordered = sections.Any(x => x.Position != positions[x.Id]);
        foreach (var section in sections)
        {
            section.Position = positions[section.Id];
        }

        if (reordered)
        {
            await ServerAudit.StageAsync(db, serverId, http.User.RequireUserId(), new AuditEvent(
                "section.reorder", messages.Get(MessageKey.AuditSectionsReordered)), cancellationToken);
        }

        await db.SaveChangesAsync(cancellationToken);
        await hub.Clients.Group(AppHub.ServerGroup(serverId)).SendAsync(
            "SectionOrderChanged",
            new SectionOrderDto(serverId.ToString(), order.Select(x => x.id.ToString()).ToArray()),
            cancellationToken);
        return Results.NoContent();
    }

    private static async Task<IResult> DeleteAsync(
        Guid sectionId,
        HttpContext http,
        AppDbContext db,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var serverId = http.RequireActiveServerId();
        var section = await db.Sections.AsNoTracking().FirstOrDefaultAsync(
            x => x.Id == sectionId && x.ServerId == serverId, cancellationToken);
        if (section is null)
        {
            return Results.NotFound();
        }

        if (!await ServerAccess.HasForSectionAsync(
                db, http.User.RequireUserId(), serverId, section.Id, ServerPrivileges.ManageChannels, cancellationToken))
        {
            return Results.Forbid();
        }

        // Channels are not tied to a section by a foreign key, so the emptiness
        // check and the delete are one statement: a channel moved in meanwhile
        // can not be left pointing at a section that is gone.
        var deleted = await db.Sections
            .Where(x => x.Id == section.Id && !db.Channels.Any(channel => channel.SectionId == x.Id))
            .ExecuteDeleteAsync(cancellationToken);
        if (deleted == 0)
        {
            return Results.Conflict();
        }

        await ServerAudit.StageAsync(db, serverId, http.User.RequireUserId(), new AuditEvent(
            "section.delete",
            messages.Get(MessageKey.AuditSectionDeleted, ("name", section.Name)),
            "section", section.Id, section.Name), cancellationToken);
        await db.SaveChangesAsync(cancellationToken);
        return Results.NoContent();
    }

    private static bool IsValidName(string name) => name.Length is >= 1 and <= MaxNameLength;

    private static IResult InvalidName(Messages messages) =>
        Results.ValidationProblem(new Dictionary<string, string[]>
        {
            ["name"] = [messages.Get(MessageKey.SectionNameInvalid)],
        });
}
