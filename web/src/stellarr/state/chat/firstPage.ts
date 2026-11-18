import type { LoadedPage, ChatAction } from "./chatStore";
import type { PendingIds, RestoredWindows, SyncCursors } from "./shared";
import { floorOf } from "./timeline";

interface Held {
  pendingIds: PendingIds;
  syncCursors: SyncCursors;
  restored: RestoredWindows;
}

/**
 * What to make of the server's newest page for a channel, and where the
 * journal is drained from afterwards. `syncCursors` is set here.
 *
 * Without a window from the device this is the plain case: the page is
 * authoritative and the journal is drained from the page's own cursor.
 *
 * With one, the stored messages are kept below the page only when the page
 * shares a message with them. Then nothing can lie between the two, since a
 * message is created at the newest end and only deleted from the middle, and
 * the journal is drained from the cursor the stored messages are current as of,
 * which is what brings the edits and deletions made below the page. When the
 * page shares none, the channel has moved on by more than a page since the
 * device last kept it; keeping the stored messages would leave a stretch of
 * history missing under the page without a sign of it, so they are dropped and
 * the page stands alone. A page that reaches the start of the channel lists
 * every message it still has, so nothing older is kept then either.
 *
 * A cursor that is already held (an earlier page for this channel settled
 * first) stays: it was chosen with the messages that are held in mind.
 */
export function settleFirstPage(
  { pendingIds, syncCursors, restored }: Held,
  channelId: string,
  page: LoadedPage & { syncCursor: string },
): ChatAction {
  const stored = restored.current[channelId];
  delete restored.current[channelId];
  const base = { type: "first-page", channelId, page, pendingIds: new Set(pendingIds.current) } as const;

  if (syncCursors.current[channelId] !== undefined) {
    return { ...base, keepOlder: true, window: null };
  }

  const joins =
    stored !== undefined &&
    page.nextCursor !== null &&
    page.messages.some((message) => stored.ids.has(message.id));
  if (joins) {
    syncCursors.current[channelId] = stored.cursor;
    return { ...base, keepOlder: true, window: { cursor: stored.cursor, floor: stored.floor } };
  }
  syncCursors.current[channelId] = page.syncCursor;
  return {
    ...base,
    keepOlder: stored === undefined,
    window: { cursor: page.syncCursor, floor: floorOf(page) },
  };
}
