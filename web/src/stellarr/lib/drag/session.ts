import type { KeyboardEvent, PointerEvent, RefObject } from "react";

import { edgeScroller, scrollParent, type EdgeScroller } from "./autoScroll";
import { revealDelta, type Bounds } from "./geometry";
import type { PointerDrag } from "./pointer";

/** What a drag handle spreads onto its element, beside its own label and role. */
export interface DragHandleProps {
  onPointerDown: (event: PointerEvent<HTMLElement>) => void;
  onKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
  onKeyUp: (event: KeyboardEvent<HTMLElement>) => void;
  onBlur: () => void;
  "aria-describedby": string;
  "aria-roledescription": string;
  "aria-pressed": true | undefined;
}

/** What a key does to a drag that the keyboard is carrying. */
export type CarryKey = "drop" | "cancel" | "up" | "down";

/**
 * The keys of a keyboard drag: the arrows move, Escape cancels, and Space,
 * Enter or Tab drop. A held Space or Enter repeats, and a repeat must not drop
 * what the first press picked up.
 */
export function carryKey(event: KeyboardEvent): CarryKey | null {
  switch (event.key) {
    case "ArrowUp":
      return "up";
    case "ArrowDown":
      return "down";
    case "Escape":
      return "cancel";
    case " ":
    case "Enter":
    case "Tab":
      return event.repeat ? null : "drop";
    default:
      return null;
  }
}

/**
 * Everything a drag holds outside React: the press being followed, the edge
 * scroller and the listeners, all released together.
 */
export interface DragSession {
  pointer: PointerDrag | null;
  edge: EdgeScroller;
  /** The list's scrolling ancestor and its offset when the drag began. */
  scroller: HTMLElement | null;
  scrollTop: number;
  /** Pointer drags: the height of the press and of the latest move. */
  originY: number;
  y: number;
  listening: AbortController;
}

export function openSession(element: Element, originY: number): DragSession {
  const scroller = scrollParent(element);
  return {
    pointer: null,
    edge: edgeScroller(null),
    scroller,
    scrollTop: scroller?.scrollTop ?? 0,
    originY,
    y: originY,
    listening: new AbortController(),
  };
}

/** How far the list has scrolled since the drag began. */
export function scrolledBy(session: DragSession): number {
  return session.scroller === null ? 0 : session.scroller.scrollTop - session.scrollTop;
}

/** Scrolls the list just far enough for `target`, in viewport pixels, to be in view. */
export function reveal(session: DragSession | null, target: Bounds) {
  const scroller = session?.scroller;
  if (!scroller) return;
  const { top, bottom } = scroller.getBoundingClientRect();
  scroller.scrollTop += revealDelta(target, { top, bottom });
}

/**
 * Puts focus back on the handle a keyboard drag ended on. A drop can move or
 * re-render the handle, and an engine that drops focus from a moved node
 * would leave the user on the page body, where the shell does not let Tab
 * walk back in.
 */
export function restoreFocus(handle: HTMLElement | null) {
  if (handle === null || !handle.isConnected) return;
  if (handle.ownerDocument.activeElement !== handle) handle.focus({ preventScroll: true });
}

/** Releases a session; safe to call when there is none. */
export function closeSession(ref: RefObject<DragSession | null>) {
  const session = ref.current;
  if (session === null) return;
  ref.current = null;
  session.listening.abort();
  session.edge.stop();
  session.pointer?.cancel();
}

/**
 * Starts scrolling the list near its edges and re-measuring as it scrolls,
 * for a pointer drag that has just begun.
 */
export function followScroll(session: DragSession, onScroll: () => void) {
  session.edge = edgeScroller(session.scroller);
  session.scroller?.addEventListener("scroll", onScroll, {
    signal: session.listening.signal,
  });
}

/**
 * A keyboard drag ends when the page it measured changes under it: the window
 * resized or the tab hidden. A pointer drag gets the same from `trackPointer`.
 */
export function watchPage(session: DragSession, element: Element, onCancel: () => void) {
  const document = element.ownerDocument;
  const options = { signal: session.listening.signal };
  document.defaultView?.addEventListener("resize", onCancel, options);
  document.addEventListener(
    "visibilitychange",
    () => {
      if (document.visibilityState === "hidden") onCancel();
    },
    options,
  );
}
