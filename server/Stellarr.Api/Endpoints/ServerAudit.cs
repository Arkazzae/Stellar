using System.Text.Json;
using System.Text.Json.Serialization;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Endpoints;

/// <summary>
/// One field an audited action changed. Values are what a reader's client
/// words in its own language: names as they were at the time, and fixed
/// tokens such as permission names or <c>public</c> and <c>invite-only</c>.
/// </summary>
public sealed record AuditChange(
    string Field,
    string? From = null,
    string? To = null,
    IReadOnlyList<string>? Added = null,
    IReadOnlyList<string>? Removed = null,
    string? FromColor = null,
    string? ToColor = null);

/// <summary>
/// What happened, for the audit log. <paramref name="Sentence"/> is the line
/// in the server's language without the actor's name, for readers that know
/// only the sentence; the rest lets a client say it in its own words.
/// </summary>
public sealed record AuditEvent(
    string Action,
    string Sentence,
    string? TargetType = null,
    Guid? TargetId = null,
    string? TargetName = null,
    IReadOnlyList<AuditChange>? Changes = null);

/// <summary>The one writer of a server's audit log.</summary>
internal static class ServerAudit
{
    private const int MaxSummary = 500;
    private const int MaxTargetName = 200;

    internal static readonly JsonSerializerOptions Json = new(JsonSerializerDefaults.Web)
    {
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    };

    /// <summary>Stages a line of the audit log; the caller saves it with the change it records.</summary>
    internal static async Task StageAsync(
        AppDbContext db,
        Guid serverId,
        Guid actorId,
        AuditEvent audit,
        CancellationToken cancellationToken)
    {
        var name = await db.Users.AsNoTracking()
            .Where(x => x.Id == actorId)
            .Select(x => x.DisplayName)
            .SingleAsync(cancellationToken);
        db.ServerAuditEntries.Add(new ServerAuditEntry
        {
            Id = Guid.NewGuid(),
            ServerId = serverId,
            ActorId = actorId,
            Summary = Clip($"{name} {audit.Sentence}", MaxSummary)!,
            CreatedAt = DateTime.UtcNow,
            Action = audit.Action,
            TargetType = audit.TargetType,
            TargetId = audit.TargetId,
            TargetName = Clip(audit.TargetName, MaxTargetName),
            ChangesJson = audit.Changes is { Count: > 0 } ? JsonSerializer.Serialize(audit.Changes, Json) : null,
        });
    }

    /// <summary>The changes of an entry, or none when it has none or they cannot be read.</summary>
    internal static IReadOnlyList<AuditChange> ReadChanges(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return [];
        }

        try
        {
            return JsonSerializer.Deserialize<AuditChange[]>(json, Json) ?? [];
        }
        catch (JsonException)
        {
            // A damaged line must not make the log unreadable.
            return [];
        }
    }

    /// <summary>A change of one value, or nothing when it stayed the same.</summary>
    internal static AuditChange? Changed(string field, string? from, string? to) =>
        string.Equals(from, to, StringComparison.Ordinal) ? null : new AuditChange(field, from, to);

    /// <summary>The changes that happened, without the fields that did not change.</summary>
    internal static IReadOnlyList<AuditChange> Only(params AuditChange?[] changes) =>
        changes.OfType<AuditChange>().ToArray();

    private static string? Clip(string? text, int max) =>
        text is null || text.Length <= max ? text : text[..(max - 1)] + "…";
}
