import type { ReactNode, RefObject } from "react";
import { flushSync } from "react-dom";
import { createRoot, type Root } from "react-dom/client";

/** Where the toolbar folds into the action sheet; the same width the style sheet uses. */
export const PHONE_QUERY = "(max-width: 767px)";

export function onPhone(): boolean {
  return window.matchMedia(PHONE_QUERY).matches;
}

/**
 * The longest a toolbar nobody wants any more stays for its fade. The end of
 * the fade normally takes it out sooner; this is for one with nothing left to
 * fade, such as a toolbar whose open picker kept it after the pointer left.
 */
const FADE_LIMIT_MS = 400;

/** Toolbars kept drawn for the next message the pointer reaches. */
const SPARE_LIMIT = 4;

interface Drawn {
  element: HTMLDivElement;
  root: Root;
}

const spares: Drawn[] = [];

function take(): Drawn {
  const spare = spares.pop();
  if (spare !== undefined) return spare;
  const element = document.createElement("div");
  element.className = "msg-toolbar";
  element.setAttribute("role", "toolbar");
  return { element, root: createRoot(element) };
}

function giveBack(drawn: Drawn): void {
  drawn.element.remove();
  if (spares.length < SPARE_LIMIT) {
    spares.push(drawn);
    return;
  }
  // Unmounting a root is not allowed while another one is rendering, which is
  // when a message that leaves the transcript takes its toolbar with it.
  queueMicrotask(() => drawn.root.unmount());
}

/** The message the pointer is over. There is one pointer, so one at a time. */
let pointedAt: ToolbarPresence | null = null;

/** Messages whose toolbar hangs in their corner now. */
const hanging = new Set<ToolbarPresence>();

let watchingWidth = false;

/**
 * Whether a message's hover toolbar is in the document, kept outside React
 * state.
 *
 * The toolbar is mostly icons, and one in every message made it most of the
 * transcript's DOM, so it is there only while it is used: while the pointer or
 * focus is on its message, while its picker or sheet is open, and while it
 * fades out. It comes and goes with the pointer many times a second, so that
 * must not be a state change in the transcript: React walks every row beside
 * the one that changed, which made each crossing dearer the longer the
 * history. The toolbar is drawn by a small root of its own and hung in the
 * message's corner, and a few of them are kept drawn, so the next message the
 * pointer reaches takes one that is ready.
 */
export class ToolbarPresence {
  hovered = false;
  focused = false;
  /** The picker or the sheet is open. */
  engaged = false;
  /** The corner the toolbar hangs in, and the control that stands for it there. */
  corner: HTMLElement | null = null;
  standIn: HTMLElement | null = null;
  label = "";
  content: ReactNode = null;
  private drawn: Drawn | null = null;
  private stopFade: (() => void) | null = null;

  get toolbar(): HTMLDivElement | null {
    return this.drawn?.element ?? null;
  }

  /**
   * Hangs the toolbar while anything uses it, and lets it fade out once
   * nothing does. `now` draws it before returning, for an event that has to
   * find its buttons; inside React's own commit it cannot.
   */
  update(now = true): void {
    if (this.hovered || this.focused || this.engaged) this.hang(now);
    else this.fade();
  }

  /** Draws the message's current actions into a toolbar that already hangs. */
  refresh(): void {
    if (this.drawn === null) return;
    this.drawn.element.setAttribute("aria-label", this.label);
    this.drawn.root.render(this.content);
  }

  /**
   * After the message has drawn again. A focused node can leave the document
   * without a blur, as the editor's field does when an edit is saved.
   */
  settle(): void {
    if (this.focused && this.corner?.parentElement?.contains(document.activeElement) !== true) {
      this.focused = false;
    }
    this.refresh();
    this.update(false);
  }

  dispose(): void {
    if (pointedAt === this) pointedAt = null;
    this.hovered = this.focused = this.engaged = false;
    this.takeDown();
  }

  private hang(now: boolean): void {
    this.stopFade?.();
    if (this.drawn !== null || this.corner === null) return;
    const drawn = take();
    this.drawn = drawn;
    drawn.element.setAttribute("aria-label", this.label);
    if (now) flushSync(() => drawn.root.render(this.content));
    else drawn.root.render(this.content);
    this.corner.insertBefore(drawn.element, this.standIn);
    hanging.add(this);
    this.placeStandIn();
    watchWidth();
  }

  /**
   * The toolbar's buttons take the stand-in's place in the tab order while it
   * hangs. On a phone they are not shown, and the stand-in keeps its place.
   */
  placeStandIn(): void {
    if (this.drawn !== null && !onPhone()) this.standIn?.setAttribute("tabindex", "-1");
    else this.standIn?.removeAttribute("tabindex");
  }

  private fade(): void {
    const drawn = this.drawn;
    if (drawn === null || this.stopFade !== null) return;
    const { element } = drawn;
    const done = () => {
      this.stopFade?.();
      // Focus inside it means it is still in use, whatever the pointer did.
      if (element.contains(document.activeElement)) {
        this.focused = true;
        return;
      }
      this.takeDown();
    };
    const faded = (event: TransitionEvent) => {
      if (event.target === element && event.propertyName === "opacity") done();
    };
    element.addEventListener("transitionend", faded);
    const limit = window.setTimeout(done, FADE_LIMIT_MS);
    this.stopFade = () => {
      element.removeEventListener("transitionend", faded);
      window.clearTimeout(limit);
      this.stopFade = null;
    };
  }

  private takeDown(): void {
    this.stopFade?.();
    const drawn = this.drawn;
    if (drawn === null) return;
    this.drawn = null;
    hanging.delete(this);
    this.placeStandIn();
    giveBack(drawn);
  }
}

/**
 * The pointer is on a message. The one it was on before lets go, which also
 * covers a pointer whose message went away under it without a mouseout.
 */
export function pointerEntered(presence: ToolbarPresence): void {
  if (pointedAt !== presence) {
    if (pointedAt !== null) pointerLeft(pointedAt);
    pointedAt = presence;
  }
  if (presence.hovered) return;
  presence.hovered = true;
  presence.update();
}

export function pointerLeft(presence: ToolbarPresence): void {
  if (pointedAt === presence) pointedAt = null;
  if (!presence.hovered) return;
  presence.hovered = false;
  presence.update();
}

/** Crossing the phone width moves the tab stop between the toolbar and its stand-in. */
function watchWidth(): void {
  if (watchingWidth) return;
  watchingWidth = true;
  window.matchMedia(PHONE_QUERY).addEventListener("change", () => {
    for (const presence of hanging) presence.placeStandIn();
  });
}

/** The presence a ref holds, made the first time it is asked for, never during render. */
export function presenceIn(ref: RefObject<ToolbarPresence | null>): ToolbarPresence {
  ref.current ??= new ToolbarPresence();
  return ref.current;
}
