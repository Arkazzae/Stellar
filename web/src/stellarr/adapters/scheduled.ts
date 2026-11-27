import {
  SCHEDULED_EVENTS,
  scheduleRefusalReason,
  type ScheduledApiClient,
} from "@/lib/api/calls/scheduled";
import type { RealtimeClient } from "@/lib/realtime/types";
import { ScheduleRefusedError, type ScheduledMessage, type ScheduledPort } from "@/ports/scheduled";

type Change = (list: readonly ScheduledMessage[]) => readonly ScheduledMessage[];

const upsert =
  (message: ScheduledMessage): Change =>
  (list) => [...list.filter((item) => item.id !== message.id), message];

const drop =
  (id: string): Change =>
  (list) => list.filter((item) => item.id !== id);

function soonestFirst(list: readonly ScheduledMessage[]): readonly ScheduledMessage[] {
  return [...list].sort((left, right) => Date.parse(left.sendAt) - Date.parse(right.sendAt));
}

/**
 * A refusal from any of the calls, in the reason the person can act on. One
 * that says the message changed at that moment reads the list again first,
 * so what the person sees next is what is there.
 */
async function refused<T>(work: () => Promise<T>, reload: () => Promise<void>): Promise<T> {
  try {
    return await work();
  } catch (cause) {
    const refusal = new ScheduledRefusal(cause);
    if (refusal.reason === "changed") await reload().catch(() => undefined);
    throw refusal;
  }
}

class ScheduledRefusal extends ScheduleRefusedError {
  constructor(cause: unknown) {
    super(scheduleRefusalReason(cause), { cause });
  }
}

/**
 * The author's scheduled messages, kept as one list in memory.
 *
 * The list is read once, when something first looks at it, and kept current
 * by the live events and by the answers to the author's own calls. An event
 * that arrives while the list is being read is applied again on top of what
 * the read returns, so the read cannot undo it. After a reconnect the list is
 * read again, since events may have been missed.
 */
export function createScheduledAdapter(
  api: ScheduledApiClient,
  realtime: RealtimeClient,
): ScheduledPort & { dispose(): void } {
  let items: readonly ScheduledMessage[] = [];
  const listeners = new Set<() => void>();
  let reading: Promise<void> | null = null;
  let changesWhileReading: Change[] | null = null;
  let asked = false;

  const publish = (next: readonly ScheduledMessage[]) => {
    items = soonestFirst(next);
    for (const listener of listeners) listener();
  };

  const change = (apply: Change) => {
    changesWhileReading?.push(apply);
    publish(apply(items));
  };

  const load = (): Promise<void> => {
    asked = true;
    reading ??= (async () => {
      changesWhileReading = [];
      try {
        const fresh = await api.listScheduled();
        publish(changesWhileReading.reduce<readonly ScheduledMessage[]>((list, apply) => apply(list), fresh));
      } finally {
        changesWhileReading = null;
        reading = null;
      }
    })();
    return reading;
  };

  let previous = realtime.getStatus();
  const stops = [
    realtime.on(SCHEDULED_EVENTS.changed, (scheduled) => change(upsert(scheduled))),
    realtime.on(SCHEDULED_EVENTS.removed, ({ id }) => change(drop(id))),
    realtime.subscribeStatus(() => {
      const next = realtime.getStatus();
      const recovered = next === "connected" && previous !== "connected";
      previous = next;
      if (recovered && asked) void load().catch(() => undefined);
    }),
  ];

  return {
    getSnapshot: () => items,
    subscribe(listener) {
      listeners.add(listener);
      if (!asked) void load().catch(() => undefined);
      return () => {
        listeners.delete(listener);
      };
    },
    load,
    schedule: (target, draft) =>
      refused(async () => {
        const created = await api.createScheduled(target, {
          content: draft.content,
          attachmentIds: draft.attachmentIds,
          ...(target.kind === "channel" && draft.replyToMessageId !== undefined
            ? { replyToMessageId: draft.replyToMessageId }
            : {}),
          // Always UTC with a Z: the server refuses an offset.
          sendAt: draft.sendAt.toISOString(),
        });
        change(upsert(created));
        return created;
      }, load),
    edit: (id, patch) =>
      refused(async () => {
        const edited = await api.updateScheduled(id, {
          ...(patch.content === undefined ? {} : { content: patch.content }),
          ...(patch.sendAt === undefined ? {} : { sendAt: patch.sendAt.toISOString() }),
        });
        change(upsert(edited));
        return edited;
      }, load),
    remove: (id) =>
      refused(async () => {
        await api.deleteScheduled(id);
        change(drop(id));
      }, load),
    sendNow: (id) =>
      refused(async () => {
        // The posted message arrives with the usual live events, under the
        // same id; only the queue entry is the list's to drop.
        const posted = await api.sendScheduled(id);
        change(drop(id));
        return posted;
      }, load),
    dispose() {
      for (const stop of stops) stop();
      listeners.clear();
    },
  };
}
