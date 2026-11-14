import { fireEvent } from "@testing-library/react";
import { vi } from "vitest";

/** Where the rail's lists begin, and the heights the style sheets give their rows. */
export const TEXT_TOP = 100;
export const VOICE_TOP = 400;
export const ROW = 32;
export const THREAD_ROW = 44;
export const OCCUPANT_ROW = 30;

function box(top: number, height: number, left = 8, width = 232): DOMRect {
  return {
    x: left,
    y: top,
    top,
    left,
    width,
    height,
    bottom: top + height,
    right: left + width,
    toJSON: () => ({}),
  } as DOMRect;
}

function translateY(element: HTMLElement): number {
  return Number(/translateY\((-?[\d.]+)px\)/.exec(element.style.transform)?.[1] ?? 0);
}

function textNodeHeight(node: Element): number {
  const threads = node.querySelector(".channel-threads:not(.closed)");
  return ROW + (threads?.children.length ?? 0) * THREAD_ROW;
}

function zoneHeight(zone: Element): number {
  return ROW + zone.querySelectorAll(".voice-occupants > li").length * OCCUPANT_ROW;
}

/** The top of `element` when its siblings matching `selector` are stacked from `start`. */
function stackedTop(element: Element, selector: string, start: number, height: (node: Element) => number) {
  let top = start;
  for (const sibling of element.ownerDocument.querySelectorAll(selector)) {
    if (sibling === element) break;
    top += height(sibling);
  }
  return top;
}

/**
 * jsdom lays nothing out, so this stands in for the rail's layout: text
 * channels stacked from `TEXT_TOP` in document order, each as tall as its row
 * and open threads, voice channels from `VOICE_TOP` with their occupants, and
 * a transform moving a box the way it would on screen. It reads the document
 * on every call, so a reorder or an opened thread list is laid out again.
 */
export function layOutRail() {
  return vi
    .spyOn(HTMLElement.prototype, "getBoundingClientRect")
    .mockImplementation(function (this: HTMLElement) {
      if (this.classList.contains("text-channel-node")) {
        const top = stackedTop(this, ".text-channel-node", TEXT_TOP, textNodeHeight);
        return box(top + translateY(this), textNodeHeight(this));
      }
      if (this.classList.contains("voice-channel-drop")) {
        return box(stackedTop(this, ".voice-channel-drop", VOICE_TOP, zoneHeight), zoneHeight(this));
      }
      const zone = this.closest(".voice-channel-drop");
      if (this.matches(".voice-occupants > li") && zone !== null) {
        const rows = [...zone.querySelectorAll(".voice-occupants > li")];
        const top =
          stackedTop(zone, ".voice-channel-drop", VOICE_TOP, zoneHeight) +
          ROW +
          rows.indexOf(this) * OCCUPANT_ROW;
        return box(top, OCCUPANT_ROW, 36, 200);
      }
      return box(0, 0, 0, 0);
    });
}

/** jsdom has no pointer capture; a drag asks for it once it starts. */
export function stubPointerCapture() {
  const capture = vi.fn();
  HTMLElement.prototype.setPointerCapture = capture;
  return capture;
}

/**
 * A mouse drag in steps: press on `handle` at `fromY`, then move the pointer
 * through each height in `path`. jsdom's `PointerEvent` defaults `isPrimary`
 * to false, which the drag rejects, so it is set here.
 */
export function pressAt(handle: Element, fromY: number) {
  fireEvent.pointerDown(handle, {
    pointerId: 1,
    isPrimary: true,
    button: 0,
    clientX: 40,
    clientY: fromY,
  });
}

export function moveTo(...path: number[]) {
  for (const y of path) {
    fireEvent.pointerMove(document, { pointerId: 1, isPrimary: true, clientX: 40, clientY: y });
  }
}

export function release(y: number) {
  fireEvent.pointerUp(document, { pointerId: 1, isPrimary: true, button: 0, clientX: 40, clientY: y });
}

export function translateOf(element: Element | null): string {
  return element instanceof HTMLElement ? element.style.transform : "";
}
