import type { VoiceSeatSource, VoiceSeatVM } from "@/components/view-models";
import type { VoicePeer } from "@/lib/voice/types";

/**
 * The ring and the microphone of everyone in the call this client is in, by
 * person.
 *
 * A row listens to its own person only, so a ring that flips renders that row
 * and nothing else. A seat keeps its object while it says the same thing, and
 * a person outside the call has none.
 */
export class VoiceSeats implements VoiceSeatSource {
  #seats: ReadonlyMap<string, VoiceSeatVM> = new Map();
  readonly #listeners = new Map<string, Set<() => void>>();

  get = (userId: string): VoiceSeatVM | null => this.#seats.get(userId) ?? null;

  subscribe = (userId: string, listener: () => void): (() => void) => {
    let listeners = this.#listeners.get(userId);
    if (listeners === undefined) {
      listeners = new Set();
      this.#listeners.set(userId, listeners);
    }
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
      if (listeners.size === 0 && this.#listeners.get(userId) === listeners) {
        this.#listeners.delete(userId);
      }
    };
  };

  /** Takes the people of a new snapshot and tells only those whose seat changed. */
  take(peers: readonly VoicePeer[]): void {
    const previous = this.#seats;
    const next = new Map<string, VoiceSeatVM>();
    for (const { userId, speaking, muted } of peers) {
      const seat = previous.get(userId);
      next.set(
        userId,
        seat !== undefined && seat.speaking === speaking && seat.muted === muted
          ? seat
          : Object.freeze({ speaking, muted }),
      );
    }
    this.#seats = next;

    const changed = new Set<string>();
    for (const [userId, seat] of next) if (previous.get(userId) !== seat) changed.add(userId);
    for (const userId of previous.keys()) if (!next.has(userId)) changed.add(userId);
    for (const userId of changed) {
      for (const listener of [...(this.#listeners.get(userId) ?? [])]) listener();
    }
  }
}
