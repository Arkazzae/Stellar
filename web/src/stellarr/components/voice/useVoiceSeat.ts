import { createContext, useCallback, useContext, useSyncExternalStore } from "react";

import type { VoiceSeatSource, VoiceSeatVM } from "@/components/view-models";

/**
 * The seats of the call this client is in. The voice provider hands out one
 * source for its whole life, so nothing renders because the source changed.
 */
export const VoiceSeatContext = createContext<VoiceSeatSource | null>(null);

const NOTHING = () => undefined;

/**
 * One person's ring and microphone in this client's call, or null for anyone
 * outside it. The caller renders when that person's seat changes and at no
 * other time. Without a source there is no call, so there is no seat.
 */
export function useVoiceSeat(userId: string): VoiceSeatVM | null {
  const seats = useContext(VoiceSeatContext);
  const subscribe = useCallback(
    (listener: () => void) => (seats === null ? NOTHING : seats.subscribe(userId, listener)),
    [seats, userId],
  );
  const read = () => seats?.get(userId) ?? null;
  return useSyncExternalStore(subscribe, read, read);
}
