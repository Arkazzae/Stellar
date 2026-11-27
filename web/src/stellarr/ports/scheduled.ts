import type { Attachment, Message } from "./chat";

/*
 * Messages the signed-in person wrote to go out later. Only their author ever
 * learns that they exist: the list, the counts and the live updates are the
 * author's own.
 */

/**
 * Where a scheduled message goes out. A thread is not a place of its own: a
 * message in a thread answers the thread's first message, as a reply does.
 */
export type ScheduledTarget =
  | { kind: "channel"; channelId: string; serverId?: string }
  | { kind: "conversation"; conversationId: string };

/** Why a scheduled message did not go out when it was due. */
export type ScheduledFailureCode = "no_access" | "target_gone" | "thread_gone" | "encrypted" | "other";

export const SCHEDULED_FAILURE_CODES: readonly ScheduledFailureCode[] = [
  "no_access",
  "target_gone",
  "thread_gone",
  "encrypted",
  "other",
];

export interface ScheduledMessage {
  id: string;
  target: ScheduledTarget;
  content: string;
  attachments: Attachment[];
  /** The message it answers, a thread's first message included; never in a conversation. */
  replyToMessageId?: string;
  /** ISO 8601, UTC: the time the author chose. */
  sendAt: string;
  createdAt: string;
  state: "pending" | "failed";
  failure?: { code: ScheduledFailureCode; at: string };
}

/** A message to schedule, with the files already uploaded for it. */
export interface ScheduledDraft {
  content: string;
  attachmentIds: string[];
  /** In a channel only. */
  replyToMessageId?: string;
  sendAt: Date;
}

/** Why scheduling, rescheduling or editing was refused, in words the person can act on. */
export type ScheduleRefusalReason =
  /** The time is too soon, already past or more than 30 days ahead. */
  | "time"
  /** Empty without files, or too long. */
  | "content"
  /** Too many files, files that cannot go there, or a voice message with company. */
  | "attachments"
  /** The author already has as many waiting as they may. */
  | "limit"
  | "encrypted"
  /** It did not go out, and so can no longer be edited, only sent again or dropped. */
  | "failed"
  /** Not allowed to post there now, or no longer there to post to. */
  | "forbidden"
  /** Sending now has to wait for the channel's slow mode. */
  | "slowMode"
  /** It was changed or removed at that moment; the list has been read again. */
  | "changed"
  | "other";

export class ScheduleRefusedError extends Error {
  readonly reason: ScheduleRefusalReason;

  constructor(reason: ScheduleRefusalReason, options?: { cause?: unknown }) {
    super(`Scheduling was refused: ${reason}`, options);
    this.name = "ScheduleRefusedError";
    this.reason = reason;
  }
}

export interface ScheduledPort {
  /**
   * Everything waiting or failed, soonest first; the same array until it
   * changes. Empty until the first `load` answers.
   */
  getSnapshot(): readonly ScheduledMessage[];
  /** Called on every change; the first subscriber starts the first load. */
  subscribe(listener: () => void): () => void;
  /** Reads the list again, as after a missed run of live updates. */
  load(): Promise<void>;
  schedule(target: ScheduledTarget, draft: ScheduledDraft): Promise<ScheduledMessage>;
  /** Pending ones only; the files and the answered message stay as they were. */
  edit(id: string, patch: { content?: string; sendAt?: Date }): Promise<ScheduledMessage>;
  /** Removes it, and the files nothing else uses with it. */
  remove(id: string): Promise<void>;
  /**
   * Posts it now, a failed one too, as a second try. The posted message has
   * the scheduled one's id and reaches the transcript through the usual live
   * events, as any other message does.
   */
  sendNow(id: string): Promise<Message>;
}
