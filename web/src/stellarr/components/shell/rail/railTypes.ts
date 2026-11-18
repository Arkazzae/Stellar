import type { MouseEventHandler, RefCallback } from "react";

import type { ChannelThreadVM } from "@/components/view-models";
import type { SortHandleProps } from "@/lib/drag/useListSort";

/** What one rail list takes from the rail's sort. */
export interface RailSortRefs {
  /** The list's own element, which also marks where an empty list is. */
  list: RefCallback<HTMLElement>;
  /** The element of a row that moves, with whatever it carries. */
  item: (channelId: string) => RefCallback<HTMLElement>;
  /** Absent when the reader may not sort: the rows are plain buttons then. */
  handle?: (channelId: string) => SortHandleProps;
}

/** What the context menu needs to know about the row it was raised on. */
export interface ChannelMenuTarget {
  id: string;
  kind: "text" | "voice";
  name: string;
  unread: number;
  sectionId?: string;
}

/** Builds the right-click handler of one channel row. */
export type ChannelMenuRaiser = (
  target: ChannelMenuTarget,
) => MouseEventHandler<HTMLElement>;

/** The thread dropdowns under the text channels of every group. */
export interface RailThreads {
  byChannel: Readonly<Record<string, ChannelThreadVM[]>>;
  /** The thread the room is showing, if any. */
  activeId: string | null;
  /** The thread being fetched after a click, if any. */
  openingId: string | null;
  /** Channels whose dropdown the reader opened by hand. */
  expanded: ReadonlySet<string>;
  onToggle: (channelId: string) => void;
  onOpen?: (thread: ChannelThreadVM) => void;
  /** Hands the rail a thread's button, and takes it back with `null`. */
  register: (threadId: string, button: HTMLButtonElement | null) => void;
}
