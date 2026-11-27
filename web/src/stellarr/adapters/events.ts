import { EVENT_EVENTS, type EventApiClient } from "@/lib/api/calls/events";
import { applySignupChange, isComing, mergeEvent, soonestFirst } from "@/lib/events/eventStore";
import type { RealtimeClient } from "@/lib/realtime/types";
import type { EventsPort, ServerEvent } from "@/ports";

const NONE: readonly ServerEvent[] = [];

/**
 * `EventsPort` over HTTP and the hub. Every copy of an event heard, from a
 * list, a message, a live change or an answer, is kept by id, so the card,
 * the list and the rail show the same one. The list is read for the server
 * the client is on, which every `/api/` call names.
 */
export function createEventsAdapter(api: EventApiClient, realtime: RealtimeClient): EventsPort & { dispose(): void } {
  const events = new Map<string, ServerEvent>();
  const listeners = new Set<() => void>();
  let lists = new Map<string, readonly ServerEvent[]>();
  /** The server whose list was read last: the one to read again after a reconnect. */
  let current: string | null = null;

  const changed = () => {
    lists = new Map();
    for (const listener of [...listeners]) listener();
  };
  const take = (incoming: ServerEvent): boolean => {
    const known = events.get(incoming.id);
    const next = mergeEvent(known, incoming);
    if (next === known) return false;
    events.set(incoming.id, next);
    return true;
  };

  const load = async (serverId: string) => {
    current = serverId;
    const read = await api.listEvents();
    const answered = new Set(read.map((event) => event.id));
    // A coming one the answer leaves out was cancelled, ended or hidden meanwhile.
    for (const [id, event] of events) {
      if (event.serverId === serverId && isComing(event) && !answered.has(id)) events.delete(id);
    }
    for (const event of read) take(event);
    changed();
  };

  let previous = realtime.getStatus();
  const stops = [
    realtime.on(EVENT_EVENTS.changed, (event) => {
      if (take(event)) changed();
    }),
    realtime.on(EVENT_EVENTS.signup, (change) => {
      const known = events.get(change.eventId);
      const next = applySignupChange(known, change);
      if (next === undefined || next === known) return;
      events.set(change.eventId, next);
      changed();
    }),
    realtime.subscribeStatus(() => {
      const next = realtime.getStatus();
      const recovered = next === "connected" && previous !== "connected";
      previous = next;
      // Changes may have been missed while away.
      if (recovered && current !== null) void load(current).catch(() => undefined);
    }),
  ];

  const answered = (event: ServerEvent) => {
    if (take(event)) changed();
    return event;
  };

  return {
    list(serverId) {
      const cached = lists.get(serverId);
      if (cached !== undefined) return cached;
      const coming = [...events.values()].filter((event) => event.serverId === serverId && isComing(event));
      const list = coming.length === 0 ? NONE : coming.sort(soonestFirst);
      lists.set(serverId, list);
      return list;
    },
    get: (eventId) => events.get(eventId),
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    load,
    remember(event) {
      if (take(event)) changed();
    },
    create: async (draft) => answered(await api.createEvent(draft)),
    edit: async (event, draft) => answered(await api.editEvent(event, draft)),
    async cancel(eventId) {
      await api.cancelEvent(eventId);
      const known = events.get(eventId);
      // Cancelled at once here; the change everyone gets says the same.
      if (known !== undefined && known.state !== "cancelled") {
        events.set(eventId, { ...known, state: "cancelled" });
        changed();
      }
    },
    signUp: async (event) => void answered(await api.signUp(event.id, event.occurrenceId)),
    signOut: async (event) => void answered(await api.signOut(event.id, event.occurrenceId)),
    people: (event, cursor) => api.people(event.id, event.occurrenceId, cursor),
    dispose() {
      for (const stop of stops) stop();
    },
  };
}
