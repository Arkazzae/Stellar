import { useCallback, useLayoutEffect, useRef, useState } from "react";

import {
  anchoredScrollTop,
  centredScrollTop,
  distanceFromEnd,
  endScrollTop,
  isAtEnd,
  readingMarks,
  type ReadingMark,
} from "./scrollAnchor";
import { arrivedAtEnd, sharesAMessage, type MessageTail, type Row } from "./transcriptRows";

/**
 * How close to the end still counts as "reading the latest". Generous enough
 * to survive a rounding-off scrollTop on fractional device pixels, small
 * enough that someone who scrolled up a screen is left alone.
 */
const AT_BOTTOM_PX = 48;

/**
 * The longest a smooth scroll is trusted to still be travelling. Browsers that
 * do not report the end of a scroll would otherwise leave it in charge of the
 * scroller for good.
 */
const GLIDE_MS = 1500;

/** What the reader does to take the scroller back from a smooth scroll. */
const READER_INPUT = ["wheel", "touchstart", "pointerdown", "keydown"] as const;

/** A smooth scroll on its way to a place that may move while it travels. */
interface Glide {
  /** Where it is headed, measured afresh each time. */
  target(): number;
  /** The offset last handed to the browser. */
  aimed: number;
  until: number;
}

/** What the scroller knows between commits. */
interface Keeper {
  /** The rows of the last commit; null until the transcript has opened. */
  rows: readonly Row[] | null;
  /** Row id to its position among the rows, which is its position in the DOM. */
  index: Map<string, number>;
  tail: MessageTail | null;
  ownSend: number | undefined;
  /**
   * The reader is exactly at the end, so the end is followed as it grows.
   *
   * Set by opening, by following a message and by the reader scrolling to the
   * end; cleared only by the reader moving away. A followed row that grows
   * (media, embeds) therefore never reads as the reader having left, which
   * would flash the catch-up button for a frame.
   */
  pinned: boolean;
  /** The last "near the end" that was published, thresholded. */
  atBottom: boolean;
  /** Where the reader is, as rows; see `readingMarks`. */
  marks: ReadingMark[];
  /** The content height the marks were taken against. */
  scrollHeight: number;
  /** The offset last seen or written. */
  lastTop: number;
  glide: Glide | null;
}

/**
 * Scrollers whose room has said they are covered, by the call's stage. Asking
 * one for its height after a commit would lay the page out only to learn that
 * it has none, and behind a call commits come with every ring that lights.
 */
const coveredScrollers = new WeakSet<HTMLElement>();

/**
 * Behind a call the channel's transcript stays mounted with no box. Everything
 * measures zero then, and taking those numbers as the reader's place would
 * lose the place it has to come back to.
 */
function shown(scroller: HTMLElement): boolean {
  return !coveredScrollers.has(scroller) && scroller.clientHeight > 0;
}

function rowElement(keeper: Keeper, content: HTMLElement, id: string): Element | null {
  const index = keeper.index.get(id);
  return index === undefined ? null : content.children.item(index);
}

/** Takes down where the reader is now, for the next change to restore. */
function remember(keeper: Keeper, scroller: HTMLElement, content: HTMLElement): void {
  if (!shown(scroller)) return;
  keeper.lastTop = scroller.scrollTop;
  keeper.scrollHeight = scroller.scrollHeight;
  const rows = keeper.rows ?? [];
  const elements = content.children;
  const viewportTop = scroller.getBoundingClientRect().top;
  keeper.marks = readingMarks(
    Math.min(rows.length, elements.length),
    (index) => {
      const row = rows[index];
      const rect = elements.item(index)?.getBoundingClientRect();
      return {
        id: row?.id ?? "",
        anchor: row?.kind === "group",
        top: (rect?.top ?? 0) - viewportTop,
        bottom: (rect?.bottom ?? 0) - viewportTop,
      };
    },
    scroller.clientHeight,
  );
}

function goToEnd(keeper: Keeper, scroller: HTMLElement, content: HTMLElement): void {
  keeper.pinned = true;
  keeper.glide = null;
  if (shown(scroller)) {
    scroller.scrollTop = endScrollTop(scroller.scrollHeight, scroller.clientHeight);
  }
  remember(keeper, scroller, content);
}

/** Re-aims a smooth scroll whose destination moved under it. */
function aim(glide: Glide, scroller: HTMLElement): void {
  const top = glide.target();
  if (Math.abs(top - glide.aimed) < 1) return;
  glide.aimed = top;
  glide.until = performance.now() + GLIDE_MS;
  scroller.scrollTo({ top, behavior: "smooth" });
}

function glideTo(keeper: Keeper, scroller: HTMLElement, target: () => number): void {
  keeper.glide = null;
  if (!shown(scroller)) return;
  const top = target();
  if (Math.abs(top - scroller.scrollTop) <= 1) return;
  keeper.glide = { target, aimed: top, until: performance.now() + GLIDE_MS };
  scroller.scrollTo({ top, behavior: "smooth" });
}

/**
 * Keeps the reader where they were after something changed size or the rows
 * changed: at the end if they were pinned there, otherwise with the row they
 * were reading at the same height on screen.
 *
 * This is the only thing that compensates for content changing above the
 * viewport, so the scroller opts out of the browser's own scroll anchoring;
 * the two would otherwise correct the same change twice.
 */
function hold(keeper: Keeper, scroller: HTMLElement, content: HTMLElement): void {
  if (!shown(scroller)) return;
  if (keeper.glide !== null) {
    // Writing the offset would cancel the smooth scroll halfway, so the
    // scroll is pointed at where its destination is now instead.
    if (performance.now() < keeper.glide.until) {
      aim(keeper.glide, scroller);
      return;
    }
    keeper.glide = null;
  }
  if (keeper.pinned) {
    goToEnd(keeper, scroller, content);
    return;
  }
  const viewportTop = scroller.getBoundingClientRect().top;
  const anchored = anchoredScrollTop(scroller.scrollTop, keeper.marks, (id) => {
    const element = rowElement(keeper, content, id);
    return element === null ? undefined : element.getBoundingClientRect().top - viewportTop;
  });
  // Messages carried over but none of the rows near the reader did, as when
  // one long block absorbs a whole page. What changed is taken to have changed
  // above them, which keeps their distance from the end.
  const top = anchored ?? scroller.scrollTop + scroller.scrollHeight - keeper.scrollHeight;
  // Every write cancels a smooth or momentum scroll in flight, so nothing is
  // written when nothing moved.
  if (Math.abs(top - scroller.scrollTop) >= 0.5) scroller.scrollTop = top;
  remember(keeper, scroller, content);
}

interface Options {
  rows: readonly Row[];
  tail: MessageTail;
  /** Changes when the reader sends something of their own. */
  followOwnSend: number | undefined;
  /** The room shows something else over the transcript, which has no box then. */
  covered?: boolean;
}

/**
 * Keeps the scroller where the reader expects it: at the end when the
 * transcript opens, at the end for messages that arrive while they are at the
 * end, at the end for anything they send themselves, and on the row they are
 * reading when anything above it changes.
 */
export function useTranscriptScroll({ rows, tail, followOwnSend, covered = false }: Options) {
  const scrollerRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const keeper = useRef<Keeper>({
    rows: null,
    index: new Map(),
    tail: null,
    ownSend: undefined,
    pinned: true,
    atBottom: true,
    marks: [],
    scrollHeight: 0,
    lastTop: 0,
    glide: null,
  });
  const [atBottom, setAtBottom] = useState(true);

  /** Says whether the end is in reach, for the catch-up button and for following arrivals. */
  const publish = useCallback(() => {
    const scroller = scrollerRef.current;
    if (scroller === null || !shown(scroller)) return;
    const state = keeper.current;
    const bottom =
      state.pinned ||
      distanceFromEnd(scroller.scrollTop, scroller.scrollHeight, scroller.clientHeight) <=
        AT_BOTTOM_PX;
    // Asked after every commit that changed the rows and on every resize. The
    // same answer is not set again: right after a render React cannot tell it
    // is the same, and renders the transcript once more, in a commit of its
    // own, only to find nothing changed.
    if (bottom === state.atBottom) return;
    state.atBottom = bottom;
    setAtBottom(bottom);
  }, []);

  // Ahead of the effects that measure, so they know in the same commit.
  useLayoutEffect(() => {
    const scroller = scrollerRef.current;
    if (scroller === null || !covered) return;
    coveredScrollers.add(scroller);
    return () => {
      coveredScrollers.delete(scroller);
    };
  }, [covered]);

  useLayoutEffect(() => {
    const scroller = scrollerRef.current;
    const content = contentRef.current;
    if (scroller === null || content === null) return;
    const state = keeper.current;

    const onScroll = () => {
      if (!shown(scroller)) return;
      const top = scroller.scrollTop;
      // Every offset this hook writes raises a scroll event a frame later. It
      // says nothing about the reader, and by then the content may have grown
      // under them, which would read as having left the end.
      if (Math.abs(top - state.lastTop) < 0.5) return;
      state.pinned = isAtEnd(top, scroller.scrollHeight, scroller.clientHeight);
      remember(state, scroller, content);
      if (state.glide !== null && Math.abs(top - state.glide.target()) <= 1) {
        state.glide = null;
      }
      publish();
    };

    const endGlide = () => {
      state.glide = null;
    };

    // One observer for both boxes: the rows growing (media, embeds, edits,
    // reactions) and the viewport shrinking (a taller composer, a keyboard).
    const observer =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(() => {
            hold(state, scroller, content);
            publish();
          });
    observer?.observe(content);
    observer?.observe(scroller);
    scroller.addEventListener("scroll", onScroll, { passive: true });
    scroller.addEventListener("scrollend", endGlide);
    for (const type of READER_INPUT) {
      scroller.addEventListener(type, endGlide, { passive: true });
    }
    return () => {
      observer?.disconnect();
      scroller.removeEventListener("scroll", onScroll);
      scroller.removeEventListener("scrollend", endGlide);
      for (const type of READER_INPUT) scroller.removeEventListener(type, endGlide);
    };
  }, [publish]);

  /*
   * Runs after every commit that changed the rows, before it is painted, so a
   * prepended page or a followed message never shows in the wrong place for a
   * frame. Deciding between following the end and holding the reader's row in
   * one place is what keeps one from undoing the other.
   */
  useLayoutEffect(() => {
    const scroller = scrollerRef.current;
    const content = contentRef.current;
    if (scroller === null || content === null) return;
    const state = keeper.current;
    const previousRows = state.rows;
    const previousTail = state.tail;
    const previousSend = state.ownSend;
    state.rows = rows;
    state.index = new Map(rows.map((row, index) => [row.id, index]));
    state.tail = tail;
    state.ownSend = followOwnSend;

    const follow =
      // Opening: the end, before the first paint. A transcript too short to
      // scroll is already there, and CSS sits it at the bottom of the pane.
      previousRows === null ||
      previousTail === null ||
      // Following an arrival is conditional on already being at the end, which
      // is the right restraint for someone else's message: being yanked
      // mid-read is worse than missing a line. Your own message is not that.
      followOwnSend !== previousSend ||
      (arrivedAtEnd(previousTail, tail) && (state.atBottom || state.pinned)) ||
      // Another conversation under the same transcript, or a first page.
      !sharesAMessage(previousRows, rows);
    if (follow) goToEnd(state, scroller, content);
    else hold(state, scroller, content);
    // In the same commit, so a button that no longer applies is never painted.
    publish();
  }, [rows, tail, followOwnSend, publish]);

  /** Smoothly to the newest message, for the catch-up button. */
  const toEnd = useCallback(() => {
    const scroller = scrollerRef.current;
    if (scroller === null) return;
    glideTo(keeper.current, scroller, () =>
      endScrollTop(scroller.scrollHeight, scroller.clientHeight),
    );
  }, []);

  /** Smoothly to the row with this id, centred. A row that is not rendered is not scrolled to. */
  const reveal = useCallback((rowId: string) => {
    const scroller = scrollerRef.current;
    const content = contentRef.current;
    const state = keeper.current;
    if (scroller === null || content === null) return;
    if (rowElement(state, content, rowId) === null) return;
    glideTo(state, scroller, () => {
      const element = rowElement(state, content, rowId);
      if (element === null) return scroller.scrollTop;
      const rect = element.getBoundingClientRect();
      return centredScrollTop(
        scroller.scrollTop,
        rect.top - scroller.getBoundingClientRect().top,
        rect.height,
        scroller.clientHeight,
        scroller.scrollHeight,
      );
    });
  }, []);

  return { scrollerRef, contentRef, atBottom, toEnd, reveal };
}
