import type { ServerEvent } from "@/ports";

/** Saturday 28 November 2026, 20:00 in Warsaw: the board's game night. */
export const GAME_NIGHT_START = "2026-11-28T19:00:00Z";

/** What the reader's own list knows of the board's voice channels. */
export const BOARD_CHANNELS = {
  "vc-gaming": { name: "Gaming" },
  "vc-meet": { name: "Spotkania" },
};

/** The board's game night on Gaming, with ten seats; override what a case is about. */
export function serverEvent(extra: Partial<ServerEvent> = {}): ServerEvent {
  return {
    id: "ev-1",
    serverId: "srv-1",
    name: "Wieczór planszówek",
    description: "Przynieście swoje gry.",
    place: { kind: "voice", channelId: "vc-gaming" },
    timeZone: "Europe/Warsaw",
    repeat: "none",
    seats: 10,
    createdBy: "u-kenji",
    occurrenceId: "occ-1",
    startsAt: GAME_NIGHT_START,
    endsAt: "2026-11-28T21:00:00Z",
    state: "upcoming",
    signedUp: 7,
    waiting: 0,
    faces: [],
    mine: "none",
    revision: 1,
    ...extra,
  };
}
