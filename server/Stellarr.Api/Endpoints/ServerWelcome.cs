using System.Text;
using Microsoft.AspNetCore.SignalR;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;
using Stellarr.Api.Realtime;

namespace Stellarr.Api.Endpoints;

/// <summary>The system notice that greets a person who has just joined a server.</summary>
internal static class ServerWelcome
{
    /// <summary>
    /// Stages the notice in the first text channel the newcomer will see; the
    /// caller saves it together with the membership. A server without such a
    /// channel gets no notice, which is valid for a voice-only server.
    /// </summary>
    /// <remarks>
    /// The membership is not saved yet, so who sees what is asked of the base
    /// role alone, as a membership starts; the owner of a new server sees
    /// every channel, and its first one is open anyway.
    /// </remarks>
    internal static async Task<Message?> CreateAsync(
        AppDbContext db,
        Guid serverId,
        User user,
        DateTime joinedAt,
        Messages messages,
        CancellationToken cancellationToken)
    {
        if (await FirstTextChannelIdAsync(db, serverId, cancellationToken) is not { } channelId)
        {
            return null;
        }

        // The notice is markup, so every punctuation character of the name is
        // escaped; the transcript shows it as the character that was typed and
        // it cannot open a span, a link, a mention or an emoji of its own.
        var name = EscapeAsciiPunctuation(user.DisplayName);
        var message = new Message
        {
            Id = Guid.NewGuid(),
            ChannelId = channelId,
            AuthorId = user.Id,
            Author = user,
            IsSystem = true,
            Content = messages.Get(MessageKey.ServerWelcomeNotice, ("name", name)),
            CreatedAt = joinedAt,
        };
        db.Messages.Add(message);
        return message;
    }

    /// <summary>Tells the members who are online and see the channel, once the notice has been saved.</summary>
    internal static Task PublishAsync(
        IHubContext<AppHub> hub,
        AppDbContext db,
        Guid serverId,
        Message? message,
        Messages messages,
        CancellationToken cancellationToken) =>
        message is null
            ? Task.CompletedTask
            : ChannelAudience.SendAsync(
                hub.Clients, db, serverId, message.ChannelId, "MessageCreated", message.ToDto(messages), cancellationToken);

    /// <summary>
    /// The name with every ASCII punctuation character after a backslash, the
    /// one escape the message grammar reads anywhere; everything else, Unicode
    /// included, needs none.
    /// </summary>
    private static string EscapeAsciiPunctuation(string value)
    {
        StringBuilder? escaped = null;
        for (var index = 0; index < value.Length; index++)
        {
            var current = value[index];
            if (!IsAsciiPunctuation(current))
            {
                escaped?.Append(current);
                continue;
            }

            escaped ??= new StringBuilder(value.Length + 8).Append(value, 0, index);
            escaped.Append('\\').Append(current);
        }

        return escaped?.ToString() ?? value;
    }

    private static bool IsAsciiPunctuation(char value) =>
        value is (>= '!' and <= '/') or (>= ':' and <= '@') or (>= '[' and <= '`') or (>= '{' and <= '~');

    private static async Task<Guid?> FirstTextChannelIdAsync(
        AppDbContext db,
        Guid serverId,
        CancellationToken cancellationToken)
    {
        // A server created in the same unit of work has its channels staged, not saved.
        var staged = db.Channels.Local
            .Where(x => x.ServerId == serverId && x.Kind == ChannelKind.Text)
            .OrderBy(x => x.Position)
            .FirstOrDefault();
        if (staged is not null)
        {
            return staged.Id;
        }

        var channels = await db.Channels.AsNoTracking()
            .Where(x => x.ServerId == serverId && x.Kind == ChannelKind.Text)
            .OrderBy(x => x.Position)
            .Select(x => new { x.Id, x.SectionId })
            .ToListAsync(cancellationToken);
        if (channels.Count == 0 || await ChannelVisibility.ServerOpenAsync(db, serverId, cancellationToken))
        {
            return channels.FirstOrDefault()?.Id;
        }

        var snapshot = await PermissionSnapshot.LoadAsync(
            db, serverId, [],
            new PermissionTargets(
                channels.Select(x => x.SectionId).OfType<Guid>().Distinct().ToArray(), channels.Select(x => x.Id).ToArray()),
            cancellationToken);
        return channels
            .FirstOrDefault(x => snapshot.EvaluateNewcomer(x.SectionId, x.Id).HasFlag(ServerPrivileges.ViewChannel))
            ?.Id;
    }
}
