import type { Bounds } from "./geometry";

/** The rail's elements as their components registered them, shared by every drag. */
export interface Stage {
  /** The scrolling element, positioned, which everything is measured from. */
  scroller: HTMLElement;
  items: ReadonlyMap<string, HTMLElement>;
  lists: ReadonlyMap<string, HTMLElement>;
  handles: ReadonlyMap<string, HTMLElement>;
  /** Hands a sentence to the live region. */
  say: (text: string) => void;
}

/** Where the list sits on the screen when a drag begins; a resize ends the drag. */
export interface Frame {
  /** Where the list's content starts in the viewport when it is not scrolled. */
  origin: number;
  /** The part of the viewport the list shows. */
  view: Bounds;
}

/**
 * One kind of drag, measured when it began: how it answers the pointer and
 * the keys, and how it ends. Nothing here reads the page after `lift`.
 */
export interface Motion {
  /** Writes the lifted state and says what was picked up. */
  lift: () => void;
  /** Pointer drags: the pointer is at `y` in the viewport and the list scrolled to `scrollTop`. */
  follow: (y: number, scrollTop: number) => void;
  /** Keyboard drags: one step; returns the part of the viewport to bring into view. */
  step: (direction: 1 | -1, scrollTop: number) => Bounds | null;
  /** Keyboard drags: the list scrolled under what is carried. */
  scrolled: (scrollTop: number) => void;
  /**
   * Puts the page back, says how it ended and, when `dropped`, applies the
   * drop. A pointer drag passes where the pointer was let go, which can come
   * before the frame that would have drawn it. Returns where keyboard focus
   * belongs afterwards.
   */
  finish: (dropped: boolean, pointer: { y: number; scrollTop: number } | null) => HTMLElement | null;
  /** After the owner rendered again: false when the drag can no longer go on. */
  committed: () => boolean;
  /** Puts the page back with no drop and nothing said, as when the owner unmounts. */
  clear: () => void;
}
