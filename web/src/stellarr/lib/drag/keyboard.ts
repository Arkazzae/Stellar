import type { KeyboardEvent } from "react";

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
 * Puts focus back on the handle a keyboard drag ended on. A drop can move or
 * re-render the handle, and an engine that drops focus from a moved node
 * would leave the user on the page body, where the shell does not let Tab
 * walk back in.
 */
export function restoreFocus(handle: HTMLElement | null) {
  if (handle === null || !handle.isConnected) return;
  if (handle.ownerDocument.activeElement !== handle) handle.focus({ preventScroll: true });
}

/**
 * A keyboard drag ends when the page it measured changes under it: the window
 * resized or the tab hidden. A pointer drag gets the same from `trackPointer`.
 */
export function watchPage(signal: AbortSignal, element: Element, onCancel: () => void) {
  const document = element.ownerDocument;
  const options = { signal };
  document.defaultView?.addEventListener("resize", onCancel, options);
  document.addEventListener(
    "visibilitychange",
    () => {
      if (document.visibilityState === "hidden") onCancel();
    },
    options,
  );
}
