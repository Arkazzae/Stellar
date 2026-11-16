// The notification uses the transcript's own parse, so a name that is not
// highlighted there never rings or pops up here: code, links and escaped
// characters are the same in both.
import { mentionsUsername } from "@/lib/messages/messageBody";

export interface MessageNotificationContext {
  authorId: string;
  channelId: string;
  content?: string;
  selfId: string | null;
  selfUsername?: string | null;
  openChannelId: string | null;
  soundEnabled: boolean;
  mentionsOnly?: boolean;
  mutedChannelIds: readonly string[];
}

export interface MentionNotificationContext {
  authorId: string;
  channelId: string;
  content: string;
  selfId: string | null;
  selfUsername: string | null;
  /** The account's switch; the device's permission is checked where it is shown. */
  enabled: boolean;
  mutedChannelIds: readonly string[];
  /** Whether the person is somewhere else: the page hidden or the window unfocused. */
  away: boolean;
}

/**
 * Whether an incoming message should pop up over other apps.
 *
 * Only a mention does, and only while the person is away from Stellarr: in
 * front of the app the message and its sound are already there. A muted
 * channel stays quiet in this as in sound, mention or not; the mention still
 * reaches the inbox, which the server fills on its own.
 */
export function shouldShowMentionNotification({
  authorId,
  channelId,
  content,
  selfId,
  selfUsername,
  enabled,
  mutedChannelIds,
  away,
}: MentionNotificationContext): boolean {
  if (!enabled || !away || selfUsername === null) return false;
  if (authorId === selfId || mutedChannelIds.includes(channelId)) return false;
  return mentionsUsername(content, selfUsername);
}

/** The first words of a message, on one line, for a notification's body. */
export function notificationPreview(content: string, limit = 140): string {
  const line = content.replace(/\s+/g, " ").trim();
  return line.length <= limit ? line : `${line.slice(0, limit - 1).trimEnd()}…`;
}

/** Whether an incoming channel message should call attention with a sound. */
export function shouldPlayMessageNotificationSound({
  authorId,
  channelId,
  content = "",
  selfId,
  selfUsername = null,
  openChannelId,
  soundEnabled,
  mentionsOnly = false,
  mutedChannelIds,
}: MessageNotificationContext): boolean {
  if (!soundEnabled || authorId === selfId || mutedChannelIds.includes(channelId)) {
    return false;
  }
  if (selfUsername !== null && mentionsUsername(content, selfUsername)) return true;
  return !mentionsOnly && channelId !== openChannelId;
}
