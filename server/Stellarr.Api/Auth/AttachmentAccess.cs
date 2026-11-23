using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;

namespace Stellarr.Api.Auth;

/// <summary>
/// Decides who may read an object from the attachment store. All attachments
/// share one store, so access follows from what the attachment is used for.
/// </summary>
public static class AttachmentAccess
{
    /// <summary>
    /// Whether <paramref name="userId"/> may read the attachment: its uploader,
    /// or anybody who is meant to see it where it is used. Every rule is part
    /// of one query, so a read costs one round trip however many there are.
    /// </summary>
    /// <remarks>
    /// A message's attachment, or the picture of one of its cards, is for
    /// whoever sees its channel. Where nobody is denied view, that is every
    /// member, and the query decides it. Only when the query says no, and the
    /// attachment is in a channel where somebody is denied view, is the
    /// evaluator asked about those channels: one of them seen is enough, since
    /// a file can be posted in several.
    /// </remarks>
    public static async Task<bool> CanReadAsync(
        AppDbContext db,
        Guid userId,
        Guid attachmentId,
        CancellationToken cancellationToken = default)
    {
        if (await CanReadByAnyRuleAsync(db, userId, attachmentId, cancellationToken))
        {
            return true;
        }

        var closed = await db.Channels.AsNoTracking()
            .Where(channel =>
                db.ServerMembers.Any(member => member.ServerId == channel.ServerId && member.UserId == userId) &&
                db.Messages.Any(message =>
                    message.ChannelId == channel.Id &&
                    (db.MessageAttachments.Any(link => link.MessageId == message.Id && link.AttachmentId == attachmentId) ||
                     db.MessageEmbeds.Any(embed => embed.MessageId == message.Id && embed.ThumbnailAttachmentId == attachmentId))))
            .Select(channel => channel.Id)
            .ToListAsync(cancellationToken);
        foreach (var channelId in closed)
        {
            if (await ChannelVisibility.ViewsAsync(db, userId, channelId, cancellationToken))
            {
                return true;
            }
        }

        return false;
    }

    private static Task<bool> CanReadByAnyRuleAsync(
        AppDbContext db,
        Guid userId,
        Guid attachmentId,
        CancellationToken cancellationToken)
    {
        var assetUrl = $"/assets/{attachmentId}";
        var open = ChannelVisibility.OpenChannels(db);
        return db.ImageAttachments.AsNoTracking().AnyAsync(
            attachment =>
                attachment.Id == attachmentId &&
                (attachment.UploadedById == userId ||

                 // An avatar or banner is shown to everybody who can see the person.
                 db.Users.Any(user => user.AvatarUrl == assetUrl || user.BannerUrl == assetUrl) ||

                 // A public server shows its icon in the directory.
                 db.Servers.Any(server =>
                     server.IconUrl == assetUrl &&
                     (server.IsPublic ||
                      db.ServerMembers.Any(member => member.ServerId == server.Id && member.UserId == userId))) ||

                 // Message attachments and link thumbnails belong to whoever
                 // sees the channel: every member, where nobody is denied view.
                 db.MessageAttachments.Any(link =>
                     link.AttachmentId == attachmentId &&
                     db.Messages.Any(message =>
                         message.Id == link.MessageId &&
                         open.Any(channel =>
                             channel.Id == message.ChannelId &&
                             db.ServerMembers.Any(member =>
                                 member.ServerId == channel.ServerId && member.UserId == userId)))) ||
                 db.MessageEmbeds.Any(embed =>
                     embed.ThumbnailAttachmentId == attachmentId &&
                     db.Messages.Any(message =>
                         message.Id == embed.MessageId &&
                         open.Any(channel =>
                             channel.Id == message.ChannelId &&
                             db.ServerMembers.Any(member =>
                                 member.ServerId == channel.ServerId && member.UserId == userId)))) ||

                 // A direct message attachment belongs to the two people in the conversation.
                 db.DirectMessageAttachments.Any(link =>
                     link.AttachmentId == attachmentId &&
                     db.DirectMessages.Any(message =>
                         message.Id == link.DirectMessageId &&
                         db.DirectConversations.Any(conversation =>
                             conversation.Id == message.ConversationId &&
                             (conversation.UserAId == userId || conversation.UserBId == userId)))) ||

                 // A soundboard clip belongs to the members of its server.
                 db.SoundboardSounds.Any(sound =>
                     sound.AttachmentId == attachmentId &&
                     db.ServerMembers.Any(member =>
                         member.ServerId == sound.ServerId && member.UserId == userId)) ||

                 // A join sound travels with its owner into rooms whose members
                 // never had access to the soundboard it came from. They have to
                 // fetch the clip they are being played, and no more: only somebody
                 // who already shares a server or a direct conversation with the
                 // owner can read it.
                 db.UserPreferences.Any(preference =>
                     preference.JoinSoundId != null &&
                     db.SoundboardSounds.Any(sound =>
                         sound.Id == preference.JoinSoundId && sound.AttachmentId == attachmentId) &&
                     (db.ServerMembers.Any(owner =>
                          owner.UserId == preference.UserId &&
                          db.ServerMembers.Any(reader =>
                              reader.ServerId == owner.ServerId && reader.UserId == userId)) ||
                      db.DirectConversations.Any(conversation =>
                          (conversation.UserAId == preference.UserId && conversation.UserBId == userId) ||
                          (conversation.UserBId == preference.UserId && conversation.UserAId == userId))))),
            cancellationToken);
    }
}
