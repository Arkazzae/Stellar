import { edgeScrollSpeed } from "./geometry";

/**
 * The nearest ancestor that scrolls vertically.
 *
 * `overflow: hidden` does not count: the rail's collapsible groups clip with
 * it, but only the rail body scrolls.
 */
export function scrollParent(element: Element): HTMLElement | null {
  const view = element.ownerDocument.defaultView;
  if (view === null) return null;
  for (let node = element.parentElement; node !== null; node = node.parentElement) {
    const { overflowY } = view.getComputedStyle(node);
    if (overflowY === "auto" || overflowY === "scroll" || overflowY === "overlay") {
      return node;
    }
  }
  return null;
}

export interface EdgeScroller {
  /** The pointer's latest height; scrolling starts once it nears an edge. */
  follow: (y: number) => void;
  stop: () => void;
}

/**
 * Scrolls `container` while a dragged pointer is held near its top or bottom
 * edge, so a channel can be carried past the part of the rail on screen. The
 * pointer may not move at all while this runs, so whoever measures against the
 * scrolled content listens for the container's own `scroll` events.
 */
export function edgeScroller(container: HTMLElement | null): EdgeScroller {
  const view = container?.ownerDocument.defaultView;
  if (container === null || view === null || view === undefined) {
    return { follow: () => {}, stop: () => {} };
  }

  let y = 0;
  let frame = 0;
  const tick = () => {
    frame = 0;
    const { top, bottom } = container.getBoundingClientRect();
    const speed = edgeScrollSpeed(y, { top, bottom });
    if (speed === 0) return;
    const before = container.scrollTop;
    // Whole pixels: a fraction can round back to where it started.
    container.scrollTop = before + Math.sign(speed) * Math.max(1, Math.round(Math.abs(speed)));
    // At the end of the list there is nothing left to scroll; the next move
    // starts the loop again if the pointer heads back.
    if (container.scrollTop !== before) frame = view.requestAnimationFrame(tick);
  };

  return {
    follow: (next) => {
      y = next;
      if (frame === 0) frame = view.requestAnimationFrame(tick);
    },
    stop: () => {
      if (frame !== 0) view.cancelAnimationFrame(frame);
      frame = 0;
    },
  };
}
