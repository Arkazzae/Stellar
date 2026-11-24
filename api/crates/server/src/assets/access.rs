//! Who may read a stored file. All attachments share one store, so access
//! follows from what the attachment is used for.

use rusqlite::{Connection, params};

use crate::permissions::visibility;
use crate::values::Id;

/// Whether `user` may read the attachment: its uploader, or anybody who is
/// meant to see it where it is used.
///
/// A message's attachment, or the picture of one of its cards, is for whoever
/// sees a channel it is posted in; a file can be posted in several, and one is
/// enough.
pub fn can_read(db: &Connection, user: Id, attachment: Id) -> rusqlite::Result<bool> {
    let url = format!("/assets/{attachment}");
    let by_a_rule: bool = db.query_row(
        "SELECT EXISTS (
            SELECT 1 FROM \"ImageAttachments\" a WHERE a.\"Id\" = ?1 AND (
                a.\"UploadedById\" = ?2
                -- An avatar or banner is shown to everybody who can see the person.
                -- A wallpaper is not, on purpose: the uploader rule is the only one
                -- that lets anybody read it.
                OR EXISTS (SELECT 1 FROM \"Users\" u WHERE u.\"AvatarUrl\" = ?3 OR u.\"BannerUrl\" = ?3)
                -- A public server shows its icon in the directory.
                OR EXISTS (SELECT 1 FROM \"Servers\" s WHERE s.\"IconUrl\" = ?3 AND (s.\"IsPublic\" <> 0
                    OR EXISTS (SELECT 1 FROM \"ServerMembers\" m WHERE m.\"ServerId\" = s.\"Id\" AND m.\"UserId\" = ?2)))
                -- A direct message attachment belongs to the two people in the conversation.
                OR EXISTS (SELECT 1 FROM \"DirectMessageAttachments\" l
                    JOIN \"DirectMessages\" dm ON dm.\"Id\" = l.\"DirectMessageId\"
                    JOIN \"DirectConversations\" c ON c.\"Id\" = dm.\"ConversationId\"
                    WHERE l.\"AttachmentId\" = ?1 AND (c.\"UserAId\" = ?2 OR c.\"UserBId\" = ?2))
                -- A soundboard clip belongs to the members of its server.
                OR EXISTS (SELECT 1 FROM \"SoundboardSounds\" s
                    JOIN \"ServerMembers\" m ON m.\"ServerId\" = s.\"ServerId\"
                    WHERE s.\"AttachmentId\" = ?1 AND m.\"UserId\" = ?2)
                -- A join sound travels with its owner into rooms whose members never
                -- had access to the soundboard it came from: whoever shares a server
                -- or a direct conversation with the owner may read it, and no more.
                OR EXISTS (SELECT 1 FROM \"UserPreferences\" p
                    JOIN \"SoundboardSounds\" s ON s.\"Id\" = p.\"JoinSoundId\"
                    WHERE s.\"AttachmentId\" = ?1 AND (
                        EXISTS (SELECT 1 FROM \"ServerMembers\" o JOIN \"ServerMembers\" r ON r.\"ServerId\" = o.\"ServerId\"
                            WHERE o.\"UserId\" = p.\"UserId\" AND r.\"UserId\" = ?2)
                        OR EXISTS (SELECT 1 FROM \"DirectConversations\" c
                            WHERE (c.\"UserAId\" = p.\"UserId\" AND c.\"UserBId\" = ?2)
                               OR (c.\"UserBId\" = p.\"UserId\" AND c.\"UserAId\" = ?2)))))
        )",
        params![attachment, user, url],
        |row| row.get(0),
    )?;
    if by_a_rule {
        return Ok(true);
    }
    // The channels it is posted in, on servers the person is a member of: one
    // of them seen is enough.
    let mut statement = db.prepare(
        "SELECT DISTINCT c.\"Id\" FROM \"Channels\" c
         WHERE EXISTS (SELECT 1 FROM \"ServerMembers\" m WHERE m.\"ServerId\" = c.\"ServerId\" AND m.\"UserId\" = ?2)
           AND EXISTS (SELECT 1 FROM \"Messages\" msg WHERE msg.\"ChannelId\" = c.\"Id\" AND (
                EXISTS (SELECT 1 FROM \"MessageAttachments\" l WHERE l.\"MessageId\" = msg.\"Id\" AND l.\"AttachmentId\" = ?1)
                OR EXISTS (SELECT 1 FROM \"MessageEmbeds\" e WHERE e.\"MessageId\" = msg.\"Id\" AND e.\"ThumbnailAttachmentId\" = ?1)))",
    )?;
    let channels: Vec<Id> = statement
        .query_map(params![attachment, user], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for channel in channels {
        if visibility::views(db, user, channel)? {
            return Ok(true);
        }
    }
    Ok(false)
}
