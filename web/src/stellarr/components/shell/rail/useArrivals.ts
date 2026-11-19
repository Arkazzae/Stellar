import { useCallback, useLayoutEffect, useRef } from "react";

import type { VoiceChannelVM } from "@/components/view-models";

/** How soon after leaving a seat a seat elsewhere is the same person moving. */
const MOVE_GAP_MS = 1500;

/**
 * Tells someone sitting down from the same person moving between channels.
 *
 * A move reaches the rail as two room frames, the room left and the room
 * reached, a moment apart and in either order. So a seat is a move when its
 * person sat in another channel at the last commit or left a seat a moment
 * ago, and an arrival otherwise. The answer is read when a row mounts, from
 * the seats of the commit before.
 */
export function useArrivals(
  voiceChannels: readonly VoiceChannelVM[],
): (personId: string) => boolean {
  const seats = useRef(new Map<string, string>());
  const leftAt = useRef(new Map<string, number>());

  useLayoutEffect(() => {
    const now = Date.now();
    const next = new Map<string, string>();
    for (const channel of voiceChannels) {
      for (const person of channel.occupants) next.set(person.id, channel.id);
    }
    for (const personId of seats.current.keys()) {
      if (!next.has(personId)) leftAt.current.set(personId, now);
    }
    for (const personId of next.keys()) leftAt.current.delete(personId);
    seats.current = next;
  }, [voiceChannels]);

  return useCallback((personId) => {
    if (seats.current.has(personId)) return false;
    const left = leftAt.current.get(personId);
    return left === undefined || Date.now() - left > MOVE_GAP_MS;
  }, []);
}
