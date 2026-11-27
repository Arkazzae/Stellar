import type { ServerEvent } from "@/ports/events";

/**
 * What asks for an event's window: the rail's list, the composer's command
 * and a card's menu all open the same one, which one host draws.
 */
export type EventWindow =
  | { kind: "create"; announceOn?: string }
  | { kind: "edit"; event: ServerEvent }
  /** One event's card on its own, opened from the list. */
  | { kind: "view"; eventId: string };

/** A store of its own; the app uses `eventWindow`, and tests make fresh ones. */
export function createEventWindow() {
  const listeners = new Set<() => void>();
  let open: EventWindow | null = null;
  const set = (next: EventWindow | null) => {
    open = next;
    for (const listener of [...listeners]) listener();
  };
  return {
    get: () => open,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    open: (window: EventWindow) => set(window),
    close: () => set(null),
  };
}

export const eventWindow = createEventWindow();
