import type { StickerName } from "@/components/primitives/MascotArt";

/**
 * The sticker each empty or first-time moment of messages and Friends shows,
 * kept in one table so the choice can be read and changed in one place.
 *
 * A sticker marks such a moment in the main area or a dialog, one at a time
 * on a screen. The side columns (the rail, Active now) say they are empty in
 * a faint line instead, and rows, headers, the start of a conversation
 * (which shows the person), calls and the lines about a conversation carry
 * none.
 */
export const MESSAGES_STICKERS = {
  /** A Friends search that matched nobody. */
  noMatch: "33-finding-message",
  /** Friends, online: there are friends, and none of them is around. */
  nobodyOnline: "51-quiet-night",
  /** Friends, before the first friend, on the online tab as on the full list. */
  noFriends: "05-invite-friends",
  /** Friends, pending: no request either way. */
  noRequests: "10-reading",
  /** Friends, blocked: nobody is. */
  nobodyBlocked: "55-moderation",
  /** The form that adds a friend. */
  addFriend: "38-high-five",
  /** The first PIN for encrypted conversations. */
  pinSetup: "41-avatar-online",
  /** The safety code two people compare. */
  safetyCode: "41-avatar-online",
} as const satisfies Record<string, StickerName>;
