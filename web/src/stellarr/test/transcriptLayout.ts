import { act, fireEvent, screen } from "@testing-library/react";
import { onTestFinished } from "vitest";

/** The height of a row nobody gave a height. */
export const ROW_PX = 100;

/** A ResizeObserver whose notifications the test sends by hand. */
class ObserverStub {
  static all = new Set<ObserverStub>();

  constructor(private readonly callback: ResizeObserverCallback) {}

  observe() {
    ObserverStub.all.add(this);
  }

  unobserve() {}

  disconnect() {
    ObserverStub.all.delete(this);
  }

  static notify() {
    for (const observer of [...ObserverStub.all]) {
      observer.callback([], observer as unknown as ResizeObserver);
    }
  }
}

/** What jsdom leaves at zero and the layout below answers instead. */
const MEASURED = ["clientHeight", "scrollHeight", "scrollTop", "getBoundingClientRect"] as const;

/**
 * Layout for the transcript in jsdom, which computes none.
 *
 * The scroller (the `role="log"` element) is `viewport` px tall. Its rows stack
 * in document order under the intro, each as tall as `heights` says or
 * `ROW_PX`; a transcript shorter than the viewport sits at the bottom, as the
 * CSS puts it. Rectangles are measured from the viewport's top edge at 0, and
 * `scrollTop` is clamped to what the content allows, as a browser does. A
 * viewport of 0 is a hidden transcript: everything measures zero and the
 * scroll offset is lost.
 *
 * Installed for the test that calls it, before rendering, and taken down when
 * that test finishes.
 */
export function fakeTranscriptLayout({ viewport = 600, intro = 0 } = {}) {
  const layout = { viewport, intro, heights: new Map<string, number>() };
  const offsets = new WeakMap<Element, number>();

  const isScroller = (element: Element) => element.classList.contains("transcript");
  const rowsOf = (scroller: Element) =>
    [...(scroller.querySelector(".transcript-inner")?.children ?? [])] as HTMLElement[];
  const heightOf = (row: HTMLElement) => layout.heights.get(row.dataset.rowId ?? "") ?? ROW_PX;
  const contentHeight = (scroller: Element) =>
    rowsOf(scroller).reduce((total, row) => total + heightOf(row), layout.intro);
  const scrollHeight = (scroller: Element) => Math.max(layout.viewport, contentHeight(scroller));
  const clamp = (scroller: Element, top: number) =>
    Math.min(Math.max(0, top), scrollHeight(scroller) - layout.viewport);
  const hidden = () => layout.viewport === 0;
  const offsetOf = (scroller: Element) => {
    if (hidden()) offsets.delete(scroller);
    return clamp(scroller, offsets.get(scroller) ?? 0);
  };

  // jsdom keeps these on `Element.prototype`; own properties on the
  // `HTMLElement` one shadow them until they are deleted again.
  Object.defineProperties(HTMLElement.prototype, {
    clientHeight: {
      configurable: true,
      get(this: HTMLElement) {
        return isScroller(this) ? layout.viewport : 0;
      },
    },
    scrollHeight: {
      configurable: true,
      get(this: HTMLElement) {
        return isScroller(this) && !hidden() ? scrollHeight(this) : 0;
      },
    },
    scrollTop: {
      configurable: true,
      get(this: HTMLElement) {
        return isScroller(this) ? offsetOf(this) : 0;
      },
      set(this: HTMLElement, value: number) {
        if (isScroller(this) && !hidden()) offsets.set(this, clamp(this, value));
      },
    },
    getBoundingClientRect: {
      configurable: true,
      value(this: HTMLElement) {
        const scroller = this.closest(".transcript");
        if (scroller === null || hidden()) return new DOMRect();
        if (this === scroller) return new DOMRect(0, 0, 0, layout.viewport);
        const rows = rowsOf(scroller);
        const index = rows.indexOf(this);
        if (index === -1) return new DOMRect();
        const free = Math.max(0, layout.viewport - contentHeight(scroller));
        let top = layout.intro + free - offsetOf(scroller);
        for (const row of rows.slice(0, index)) top += heightOf(row);
        return new DOMRect(0, top, 0, heightOf(this));
      },
    },
  });
  const realObserver = globalThis.ResizeObserver;
  globalThis.ResizeObserver = ObserverStub as unknown as typeof ResizeObserver;
  onTestFinished(() => {
    for (const name of MEASURED) {
      delete (HTMLElement.prototype as unknown as Record<string, unknown>)[name];
    }
    ObserverStub.all.clear();
    globalThis.ResizeObserver = realObserver;
  });

  const scroller = () => screen.getByRole("log");

  return {
    /** Changes what the next measurement sees; nothing is notified. */
    set(change: { viewport?: number; intro?: number; heights?: Record<string, number> }) {
      if (change.viewport !== undefined) layout.viewport = change.viewport;
      if (change.intro !== undefined) layout.intro = change.intro;
      for (const [id, height] of Object.entries(change.heights ?? {})) {
        layout.heights.set(id, height);
      }
    },
    scroller,
    /** The reader scrolls: the offset moves and the browser reports it. */
    scrollTo(top: number) {
      const element = scroller();
      act(() => {
        element.scrollTop = top;
        fireEvent.scroll(element);
      });
    },
    /** The browser reports that a box changed size. */
    resized() {
      act(() => ObserverStub.notify());
    },
    /** The furthest the scroller can go now. */
    end() {
      const element = scroller();
      return element.scrollHeight - element.clientHeight;
    },
    /** Where a row starts, from the viewport's top edge. */
    topOf(rowId: string) {
      const row = scroller().querySelector(`[data-row-id="${rowId}"]`);
      if (row === null) throw new Error(`no row ${rowId}`);
      return row.getBoundingClientRect().top;
    },
  };
}
