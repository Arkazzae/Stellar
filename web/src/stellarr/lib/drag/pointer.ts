/**
 * One press on a drag handle, followed until it is released.
 *
 * The press has to travel `ACTIVATION_DISTANCE` before it counts as a drag,
 * which is what lets a whole row be both a click target and a handle. Mouse,
 * touch and pen all arrive as pointer events. Rows keep the default
 * `touch-action`, so a finger swiping along the rail scrolls it: the browser
 * takes the gesture and reports `pointercancel`, which ends the drag.
 */

/** Pixels a press travels before it is a drag rather than a click. */
export const ACTIVATION_DISTANCE = 6;

export interface PointerDragEvents {
  /** The press went past the activation distance. */
  onStart: () => void;
  /** Every move from the start on, in viewport pixels. */
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
  const originX = down.clientX;
  const originY = down.clientY;
  let started = false;
  const listening = new AbortController();
  const options = { signal: listening.signal };
  const ours = (event: PointerEvent) => event.pointerId === pointerId;

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

  document.addEventListener(
    "pointermove",
    (event) => {
      if (!ours(event)) return;
      if (!started) {
        const travelled = Math.hypot(event.clientX - originX, event.clientY - originY);
        if (travelled <= ACTIVATION_DISTANCE) return;
        started = true;
        if (handle.isConnected) handle.setPointerCapture(pointerId);
        document.getSelection()?.removeAllRanges();
        document.addEventListener("selectstart", preventDefault, options);
        view.addEventListener("contextmenu", preventDefault, options);
        events.onStart();
      }
      events.onMove(event.clientY);
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
