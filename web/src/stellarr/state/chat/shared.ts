import type { Dispatch, RefObject } from "react";

import type { ChatAction } from "./chatStore";
import type { Mark } from "./timeline";

export type ChatDispatch = Dispatch<ChatAction>;

/** Ids of optimistic messages the server has not answered yet. */
export type PendingIds = RefObject<Set<string>>;

/** The newest journal cursor seen for each channel. */
export type SyncCursors = RefObject<Record<string, string>>;

/** How many messages one history request asks for. */
export const PAGE_SIZE = 50;

/**
 * A channel's window as the device kept it, until the server's first page has
 * settled what to make of it. `ids` are the messages that were painted from it,
 * `floor` the oldest of them.
 */
export interface RestoredWindow {
  ids: ReadonlySet<string>;
  cursor: string;
  floor: Mark;
}

export type RestoredWindows = RefObject<Record<string, RestoredWindow>>;
