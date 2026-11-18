import { act, fireEvent } from "@testing-library/react";
import { vi } from "vitest";

/** The heights the style sheets give the rail's fixed parts, in px. */
export const ROW = 32;
export const THREAD_ROW = 44;
export const OCCUPANT_ROW = 30;
export const CAPTION = 22;
export const SECTION_CAPTION = 35;
export const EMPTY_NOTE = 26;

function heightOf(element: Element): number {
  if (element.matches(".collapsible.closed, .channel-threads.closed, [hidden], .sr-only")) return 0;
  if (element.matches(".channel")) return ROW;
  if (element.matches(".rail-section-cap")) return SECTION_CAPTION;
  if (element.matches(".rail-cap")) return CAPTION;
  if (element.matches(".voice-occupants > li")) return OCCUPANT_ROW;
  if (element.matches(".channel-threads > li")) return THREAD_ROW;
  if (element.matches(".rail-section-empty")) return EMPTY_NOTE;
  return [...element.children].reduce((sum, child) => sum + heightOf(child), 0);
}

/**
 * Where `element` starts in the rail's content, every element stacked right
 * below the siblings before it: what `offsetTop` reads from the positioned
 * `.rail-scroll`, before any transform and however far it has scrolled.
 */
export function railTop(element: Element): number {
  const parent = element.parentElement;
  if (parent === null || element.matches(".rail-scroll")) return 0;
  let top = railTop(parent);
  for (const sibling of parent.children) {
    if (sibling === element) break;
    top += heightOf(sibling);
  }
  return top;
}

function translateY(element: Element): number {
  if (!(element instanceof HTMLElement)) return 0;
  return Number(/translateY\((-?[\d.]+)px\)/.exec(element.style.transform)?.[1] ?? 0);
}

/**
 * jsdom lays nothing out, so this stands in for the rail's layout: every
 * element of the rail stacked in document order from the top of the
 * unscrolled `.rail-scroll`, rows, captions, threads and people at the
 * heights the style sheets give them, and a collapsed group or thread list
 * at none. It reads the document on every call, so a reorder or a group
 * closing is laid out again. Viewport boxes add the element's own transform,
 * the way a carried copy is drawn.
 */
export function layOutRail() {
  vi.spyOn(HTMLElement.prototype, "offsetTop", "get").mockImplementation(function (this: HTMLElement) {
    return railTop(this);
  });
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (
    this: HTMLElement,
  ) {
    return heightOf(this);
  });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    const inRail = this.closest(".rail-scroll") !== null;
    const top = inRail ? railTop(this) + translateY(this) : 0;
    const height = this.matches(".rail-scroll") ? 600 : inRail ? heightOf(this) : 0;
    return { x: 8, y: top, top, left: 8, width: 232, height, bottom: top + height, right: 240, toJSON: () => ({}) } as DOMRect;
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

/** Lets the frame a pointer drag draws in run. */
export async function nextFrame() {
  await act(() => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));
}

export function translateOf(element: Element | null): string {
  return element instanceof HTMLElement ? element.style.transform : "";
}
