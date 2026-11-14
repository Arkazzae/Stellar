import type { MessageGroupVM } from "./MessageGroup";

export interface TranscriptSectionVM {
  /** Day heading, already localised — `formatDay` produces these. */
  day: string;
  groups: MessageGroupVM[];
}

export type Row =
  | { kind: "day"; id: string; day: string }
  | { kind: "unread"; id: string }
  | { kind: "group"; id: string; group: MessageGroupVM };

/**
 * Flattens the day/group tree into the flat list the transcript renders.
 *
 * Separators are rows of their own rather than wrappers around a day, so every
 * row is one box in document order: the reader's place is kept by finding the
 * row at the top of the viewport, and that search halves a flat list.
 */
export function toRows(
  sections: readonly TranscriptSectionVM[],
  firstUnreadGroupId: string | null,
): Row[] {
  const rows: Row[] = [];
  for (const { day, groups } of sections) {
    rows.push({ kind: "day", id: `day-${day}`, day });
    for (const group of groups) {
      // The line belongs between two blocks, and the reader's place is kept by
      // whole rows, so it cannot be a class on the block that follows it.
      if (group.id === firstUnreadGroupId) {
        rows.push({ kind: "unread", id: `unread-${group.id}` });
      }
      rows.push({ kind: "group", id: group.id, group });
    }
  }
  return rows;
}

/** What the end of the transcript looks like, to tell an arrival from a reload. */
export interface MessageTail {
  count: number;
  lastMessageId: string | undefined;
}

/**
 * A message landed at the end.
 *
 * Both have to change. Swapping an optimistic id for the server's keeps the
 * count, so it cannot count as a second arrival; an older page raises the count
 * but leaves the newest message where it was.
 */
export function arrivedAtEnd(previous: MessageTail, next: MessageTail): boolean {
  return next.count > previous.count && next.lastMessageId !== previous.lastMessageId;
}

/**
 * Whether any message from the old rows is still in the new ones.
 *
 * Not the case when a direct message switches to another person under the
 * same transcript, or when the first page lands in an empty one: there is no
 * place to keep, and the transcript opens at its end as it would on mounting.
 * Messages rather than rows, because a merged block changes its row's id while
 * every message in it stays.
 */
export function sharesAMessage(previous: readonly Row[], next: readonly Row[]): boolean {
  const before = new Set<string>();
  for (const row of previous) {
    if (row.kind === "group") for (const body of row.group.bodies) before.add(body.id);
  }
  if (before.size === 0) return false;
  return next.some(
    (row) => row.kind === "group" && row.group.bodies.some((body) => before.has(body.id)),
  );
}

/** The oldest message block that is loaded; older history is asked for from here. */
export function oldestGroupId(rows: readonly Row[]): string | null {
  return rows.find((row) => row.kind === "group")?.id ?? null;
}

export function messageTail(rows: readonly Row[]): MessageTail {
  let count = 0;
  let lastMessageId: string | undefined;
  for (const row of rows) {
    if (row.kind !== "group") continue;
    count += row.group.bodies.length;
    lastMessageId = row.group.bodies.at(-1)?.id ?? lastMessageId;
  }
  return { count, lastMessageId };
}

/** The row holding the message (or the block) with this id, or -1. */
export function indexOfMessage(rows: readonly Row[], messageId: string): number {
  return rows.findIndex(
    (row) =>
      row.kind === "group" &&
      (row.id === messageId ||
        row.group.bodies.some((body) => body.id === messageId)),
  );
}
