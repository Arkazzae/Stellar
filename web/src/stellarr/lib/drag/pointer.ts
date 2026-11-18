/**
 * One press on a drag handle, followed until it is released.
 *
 * A mouse or pen press has to travel `ACTIVATION_DISTANCE` before it counts as
 * a drag, which is what lets a whole row be both a click target and a handle.
 *
 * A finger cannot do the same: travelling is how it scrolls the rail. Rows
 * keep the default `touch-action`, so a swipe still scrolls, and a finger
 * lifts a row by holding still for `LONG_PRESS` instead. A finger that moves
 * before that is scrolling, and is left to the browser. Once a row is lifted,
 * the handle's owner has to cancel the `touchmove` events, or the browser
 * takes the gesture for a scroll after all.
 */

/** Pixels a press travels before it is a drag rather than a click. */
export const ACTIVATION_DISTANCE = 6;

/** Milliseconds a finger holds still on a handle before the row lifts. */
export const LONG_PRESS = 300;

export interface PointerDragEvents {
  /** The press became a drag: it travelled far enough, or a finger held still long enough. */
  onStart: () => void;
  /** Every move from the start on, in viewport pixels; the first is the height it started at. */
  onMove: (y: number) => void;
  /** Released after the start. */
  onDrop: () => void;
  /** Escape, the browser taking the gesture, the tab hidden or the window resized. */
  onCancel: () => void;
}

export interface PointerDrag {
  /** Stops following the press without a drop, as when the carried item disappears. */
  cancel: () => void;
}

const preventDefault = (event: Event) => event.preventDefault();

/**
 * Keeps the browser from scrolling under a finger that lifted a row: every
 * `touchmove` on `handle` is cancelled while `lifted()` says so. It has to be
 * in place before the finger lands, because that is when the browser decides
 * whether it may scroll without asking the page. Returns the removal.
 */
export function holdFingerOn(handle: HTMLElement, lifted: () => boolean): () => void {
  const hold = (event: TouchEvent) => {
    if (lifted()) event.preventDefault();
  };
  handle.addEventListener("touchmove", hold, { passive: false });
  return () => handle.removeEventListener("touchmove", hold);
}

/**
 * Eats the click the browser sends after a drag is released, so dropping a
 * channel does not also open it. One click at most, and only briefly: a click
 * that never comes must not take a later, real one with it.
 */
function swallowNextClick(view: Window) {
  const swallow = (event: MouseEvent) => {
    event.preventDefault();
    event.stopPropagation();
  };
  const armed = new AbortController();
  view.addEventListener("click", swallow, { capture: true, once: true, signal: armed.signal });
  setTimeout(() => armed.abort(), 50);
}

/**
 * Follows the press that `down` began on `handle`.
 *
 * The listeners sit on the document rather than on the handle, because a
 * handle can unmount mid-drag (a person leaving the call) and events aimed at
 * a detached node no longer bubble. Pointer capture is taken once the drag
 * starts, so the rows passing under the pointer do not light up as hovered.
 */
export function trackPointer(
  down: PointerEvent,
  handle: HTMLElement,
  events: PointerDragEvents,
): PointerDrag {
  const document = handle.ownerDocument;
  const view = document.defaultView ?? window;
  const { pointerId } = down;
  const finger = down.pointerType === "touch";
  let x = down.clientX;
  let y = down.clientY;
  let started = false;
  /** Where the drag started, to tell a held finger from one that moved. */
  let startX = x;
  let startY = y;
  const listening = new AbortController();
  const options = { signal: listening.signal };
  const ours = (event: PointerEvent) => event.pointerId === pointerId;
  const travelled = (fromX: number, fromY: number) =>
    Math.hypot(x - fromX, y - fromY) > ACTIVATION_DISTANCE;

  /** After a drag ends with the button still down, the release still owes a click. */
  const awaitRelease = () => {
    const waiting = new AbortController();
    const release = (event: PointerEvent) => {
      if (!ours(event)) return;
      waiting.abort();
      if (event.type === "pointerup") swallowNextClick(view);
    };
    document.addEventListener("pointerup", release, { signal: waiting.signal });
    document.addEventListener("pointercancel", release, { signal: waiting.signal });
  };

  const cancel = () => {
    if (listening.signal.aborted) return;
    listening.abort();
    if (!started) return;
    awaitRelease();
    events.onCancel();
  };

  const start = () => {
    started = true;
    startX = x;
    startY = y;
    if (handle.isConnected) handle.setPointerCapture(pointerId);
    document.getSelection()?.removeAllRanges();
    document.addEventListener("selectstart", preventDefault, options);
    // Captured before React sees it, so the row's own menu does not open
    // over a drag. A finger that has held still since the row lifted is a
    // long press asking for that menu, which wins over the drag.
    view.addEventListener(
      "contextmenu",
      (event) => {
        if (finger && !travelled(startX, startY)) {
          cancel();
          return;
        }
        event.preventDefault();
        event.stopPropagation();
      },
      { capture: true, signal: listening.signal },
    );
    events.onStart();
    events.onMove(y);
  };

  if (finger) {
    const held = view.setTimeout(() => {
      if (!listening.signal.aborted) start();
    }, LONG_PRESS);
    listening.signal.addEventListener("abort", () => view.clearTimeout(held));
  }

  document.addEventListener(
    "pointermove",
    (event) => {
      if (!ours(event)) return;
      x = event.clientX;
      y = event.clientY;
      if (started) {
        events.onMove(y);
        return;
      }
      if (!travelled(down.clientX, down.clientY)) return;
      // A finger on the move before the long press is a scroll.
      if (finger) listening.abort();
      else start();
    },
    options,
  );
  document.addEventListener(
    "pointerup",
    (event) => {
      if (!ours(event)) return;
      listening.abort();
      if (!started) return;
      swallowNextClick(view);
      events.onDrop();
    },
    options,
  );
  document.addEventListener(
    "pointercancel",
    (event) => {
      if (!ours(event)) return;
      listening.abort();
      if (started) events.onCancel();
    },
    options,
  );
  document.addEventListener(
    "keydown",
    (event) => {
      if (event.key === "Escape") cancel();
    },
    options,
  );
  document.addEventListener(
    "visibilitychange",
    () => {
      if (document.visibilityState === "hidden") cancel();
    },
    options,
  );
  view.addEventListener("resize", cancel, options);
  // An image or link inside the row would otherwise start the browser's own
  // drag and drop, which ends pointer events with a `pointercancel`.
  view.addEventListener("dragstart", preventDefault, options);

  return { cancel };
}
