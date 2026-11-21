import type { Message, MessageSyncChange } from "@/ports";

export const NO_MESSAGES: Message[] = [];

/** Oldest first; the id breaks a tie so two messages never swap on a re-sort. */
export function byCreatedAt(
  a: Pick<Message, "id" | "createdAt">,
  b: Pick<Message, "id" | "createdAt">,
): number {
  return (
    new Date(a.createdAt).getTime() - new Date(b.createdAt).getTime() ||
    a.id.localeCompare(b.id)
  );
}

/**
 * Adds `message` to a channel's bucket.
 *
 * The sender appends the POST answer *and* receives `MessageCreated`, so the id
 * is the identity: the second arrival replaces the first in place rather than
 * doubling it. Only a genuinely new message is placed by time.
 */
export function mergeMessage(
  bucket: Message[] | undefined,
  message: Message,
): Message[] {
  const existing = bucket ?? NO_MESSAGES;
  const at = existing.findIndex((item) => item.id === message.id);
  if (at !== -1) {
    const next = [...existing];
    next[at] = message;
    return next;
  }
  return [...existing, message].sort(byCreatedAt);
}

export function mergeAll(
  bucket: Message[] | undefined,
  messages: readonly Message[],
): Message[] {
  return messages.reduce(mergeMessage, bucket ?? NO_MESSAGES);
}

/** A message that marks a place in a channel's history; the ends of a window. */
export type Mark = Pick<Message, "id" | "createdAt">;

/**
 * Where the contiguous run that a page starts ends below: its oldest message,
 * or null when the page reaches the beginning of the channel, so that nothing
 * lies below it at all.
 */
export function floorOf(page: {
  messages: readonly Message[];
  nextCursor: string | null;
}): Mark | null {
  if (page.nextCursor === null) return null;
  return page.messages.reduce<Mark | null>(
    (oldest, message) =>
      oldest === null || byCreatedAt(message, oldest) < 0 ? message : oldest,
    null,
  );
}

/** The lower of two floors; null, the beginning of the channel, is the lowest. */
export function lowerFloor(a: Mark | null, b: Mark | null): Mark | null {
  if (a === null || b === null) return null;
  return byCreatedAt(a, b) <= 0 ? a : b;
}

/**
 * Replaces the newest server-owned window while retaining older pages and the
 * optimistic sends that have not reached the server yet.
 *
 * `keepOlder` is false when what lies below the page is not known to join it:
 * messages restored from the device, with the channel moved on by more than one
 * page since they were kept. Keeping them would put a gap in the history that
 * nothing shows, so only the page and the unsent messages remain.
 */
export function reconcileLatest(
  current: Message[] | undefined,
  latest: readonly Message[],
  pendingIds: ReadonlySet<string>,
  keepOlder = true,
): Message[] {
  const serverIds = new Set(latest.map((message) => message.id));
  const oldest = latest.length === 0
    ? Number.NEGATIVE_INFINITY
    : Math.min(...latest.map((message) => Date.parse(message.createdAt)));
  const retained = (current ?? NO_MESSAGES).filter(
    (message) =>
      pendingIds.has(message.id) ||
      serverIds.has(message.id) ||
      (keepOlder && Date.parse(message.createdAt) < oldest),
  );
  return mergeAll(retained, latest);
}

/**
 * The window after the server compacted the journal this client was resuming:
 * nothing already held can be trusted, so only the unsent messages survive
 * underneath the fresh page.
 */
export function resetWindow(
  current: Message[] | undefined,
  fresh: readonly Message[],
  pendingIds: ReadonlySet<string>,
): Message[] {
  const unsent = (current ?? NO_MESSAGES).filter((message) =>
    pendingIds.has(message.id));
  return mergeAll(unsent, fresh);
}

/**
 * Replays a journal page, in order, over a bucket.
 *
 * A change to a message that is not loaded and lies below `floor` is left out:
 * it would stand alone under a stretch of history that is not loaded.
 */
export function applySyncChanges(
  bucket: Message[] | undefined,
  changes: readonly MessageSyncChange[],
  floor: Mark | null = null,
): Message[] {
  let next = bucket ?? NO_MESSAGES;
  for (const change of changes) {
    if (change.kind === "delete" || change.message === undefined) {
      next = next.filter((message) => message.id !== change.messageId);
    } else if (
      floor === null ||
      byCreatedAt(change.message, floor) >= 0 ||
      next.some((message) => message.id === change.messageId)
    ) {
      next = mergeMessage(next, change.message);
    }
  }
  return next;
}

/** The same bucket, untouched, when the message is not in it. */
export function removeMessage(bucket: Message[], messageId: string): Message[] {
  const next = bucket.filter((message) => message.id !== messageId);
  return next.length === bucket.length ? bucket : next;
}

/**
 * Rewrites a message that is already loaded; the same bucket when it is not.
 * A message nobody has loaded needs no patch, since opening the channel reads
 * it with the change already in.
 */
export function updateLoaded(
  bucket: Message[],
  messageId: string,
  change: (message: Message) => Message,
): Message[] {
  const at = bucket.findIndex((message) => message.id === messageId);
  const existing = bucket[at];
  if (existing === undefined) return bucket;
  const changed = change(existing);
  if (changed === existing) return bucket;
  const next = [...bucket];
  next[at] = changed;
  return next;
}

/**
 * The server's copy takes the placeholder's place in one update, so the row
 * carries the real id and timestamp without blinking twice.
 */
export function settleOptimistic(
  bucket: Message[] | undefined,
  placeholderId: string,
  message: Message,
): Message[] {
  const without = (bucket ?? NO_MESSAGES).filter(
    (item) => item.id !== placeholderId,
  );
  return mergeMessage(without, message);
}
