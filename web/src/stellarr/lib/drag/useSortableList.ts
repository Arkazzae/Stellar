import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent,
} from "react";

import {
  clampShift,
  closestIndex,
  moveItem,
  sortOffsets,
  stepIndex,
  type Span,
} from "./geometry";
import { trackPointer } from "./pointer";
import {
  carryKey,
  closeSession,
  followScroll,
  openSession,
  restoreFocus,
  reveal,
  scrolledBy,
  watchPage,
  type DragHandleProps,
  type DragSession,
} from "./session";

/** What a screen reader hears at each step; positions count from 1. */
export interface SortMessages {
  lifted: (id: string, position: number, count: number) => string;
  moved: (id: string, position: number, count: number) => string;
  dropped: (id: string, position: number, count: number) => string;
  cancelled: (id: string) => string;
}

export interface SortableListOptions {
  /** The items in display order. */
  ids: readonly string[];
  /** The complete new order, after a drop that changed it. */
  onReorder: (order: string[]) => void;
  messages: SortMessages;
  /** Read out in place of the handle's own role. */
  roleDescription: string;
}

/** What each item's outermost element takes: the part of it that moves. */
export interface SortableItemProps {
  ref: (node: HTMLElement | null) => void;
  className?: string;
  style?: CSSProperties;
}

interface Carry {
  id: string;
  via: "pointer" | "keyboard";
  /** The list as measured when the drag began; any change to it ends the drag. */
  ids: readonly string[];
  spans: readonly Span[];
  from: number;
  to: number;
  /** Pointer drags only: how far the carried item is drawn from its own slot. */
  shift: number;
}

function sameOrder(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((id, index) => id === b[index]);
}

/**
 * A vertical list sorted by dragging, with the pointer or the keyboard.
 *
 * Every item is measured when a drag begins. The carried one follows the
 * pointer, held inside the list; the others move aside with transforms only,
 * so nothing reflows under the pointer, and the target is the slot whose
 * centre is nearest the carried item's. The keyboard steps one slot at a time
 * with the arrows. Items may differ in height.
 */
export function useSortableList({
  ids,
  onReorder,
  messages,
  roleDescription,
}: SortableListOptions) {
  const instructionsId = useId();
  const [carry, setCarry] = useState<Carry | null>(null);
  const [announcement, setAnnouncement] = useState("");
  // Mirrors `carry` for the pointer listeners, which outlive the render that
  // created them.
  const latest = useRef<Carry | null>(null);
  const session = useRef<DragSession | null>(null);
  const nodes = useRef(new Map<string, HTMLElement>());
  const options = useRef({ ids, onReorder, messages });
  /** Where the carried item was drawn when it was let go, for the settle. */
  const settle = useRef<{ id: string; top: number } | null>(null);
  /** A Space that picked up or dropped; its keyup must not click the button. */
  const spaceTaken = useRef(false);
  /** The handle a keyboard drag ended on, which keeps focus after the drop. */
  const refocus = useRef<HTMLElement | null>(null);

  // A channel deleted, added or moved by someone else leaves the measured
  // rows describing a list that no longer exists, so the drag ends.
  if (carry !== null && !sameOrder(carry.ids, ids)) {
    setCarry(null);
    setAnnouncement(messages.cancelled(carry.id));
  }

  useLayoutEffect(() => {
    options.current = { ids, onReorder, messages };
  });

  useLayoutEffect(() => {
    if (carry !== null) return;
    latest.current = null;
    closeSession(session);
    restoreFocus(refocus.current);
    refocus.current = null;

    // The list has re-rendered in its new order. The dropped item starts from
    // where it was let go and eases into its slot; the rest are already there.
    const landed = settle.current;
    settle.current = null;
    const node = landed === null ? undefined : nodes.current.get(landed.id);
    if (landed === null || node === undefined) return;
    const distance = landed.top - node.getBoundingClientRect().top;
    if (Math.abs(distance) < 1) return;
    node.style.transform = `translateY(${distance}px)`;
    node.getBoundingClientRect();
    node.dataset.settling = "";
    node.style.transform = "";
    // The row's own colour transitions bubble up here too, and end sooner.
    const settled = new AbortController();
    node.addEventListener(
      "transitionend",
      (event) => {
        if (event.target !== node || event.propertyName !== "transform") return;
        delete node.dataset.settling;
        settled.abort();
      },
      { signal: settled.signal },
    );
  }, [carry]);

  useEffect(() => () => closeSession(session), []);

  const update = (next: Carry) => {
    latest.current = next;
    setCarry(next);
  };

  /** Measures every item, or null when one of them is not on the page. */
  const measure = (id: string, via: Carry["via"]): Carry | null => {
    const order = options.current.ids;
    const from = order.indexOf(id);
    if (from === -1) return null;
    const spans: Span[] = [];
    for (const each of order) {
      const node = nodes.current.get(each);
      if (node === undefined) return null;
      // An item still easing in from the last drop is measured at rest.
      node.style.transform = "";
      delete node.dataset.settling;
      const { top, height } = node.getBoundingClientRect();
      spans.push({ top, height });
    }
    return { id, via, ids: order, spans, from, to: from, shift: 0 };
  };

  const lift = (next: Carry) => {
    update(next);
    setAnnouncement(
      options.current.messages.lifted(next.id, next.from + 1, next.ids.length),
    );
  };

  const moveTo = (current: Carry, to: number, shift: number) => {
    if (to === current.to && shift === current.shift) return;
    update({ ...current, to, shift });
    if (to !== current.to) {
      setAnnouncement(
        options.current.messages.moved(current.id, to + 1, current.ids.length),
      );
    }
  };

  const finish = (dropped: boolean) => {
    const current = latest.current;
    if (current === null) return;
    const node = nodes.current.get(current.id);
    settle.current =
      node === undefined ? null : { id: current.id, top: node.getBoundingClientRect().top };
    latest.current = null;
    setCarry(null);
    closeSession(session);

    const { messages: say, onReorder: reorder } = options.current;
    if (!dropped) {
      setAnnouncement(say.cancelled(current.id));
      return;
    }
    setAnnouncement(say.dropped(current.id, current.to + 1, current.ids.length));
    if (current.to !== current.from) reorder(moveItem(current.ids, current.from, current.to));
  };

  /** Pointer drags: where the carried item is drawn, and the slot it is over. */
  const follow = (active: DragSession) => {
    const current = latest.current;
    const span = current?.spans[current.from];
    const first = current?.spans[0];
    const last = current?.spans[current.spans.length - 1];
    if (current?.via !== "pointer" || !span || !first || !last) return;
    const travel = active.y - active.originY + scrolledBy(active);
    const shift = clampShift(span, travel, {
      top: first.top,
      bottom: last.top + last.height,
    });
    // Clamp the drawing only: a tall item cannot reach a short edge row's
    // original centre while its whole box is held inside the list.
    moveTo(current, closestIndex(current.spans, span.top + travel + span.height / 2), shift);
  };

  const pressPointer = (id: string, event: PointerEvent<HTMLElement>) => {
    if (!event.isPrimary || event.button !== 0 || latest.current !== null) return;
    // A press that was released without becoming a drag left its session here.
    closeSession(session);
    const handle = event.currentTarget;
    const active = openSession(handle, event.clientY);
    session.current = active;
    active.pointer = trackPointer(event.nativeEvent, handle, {
      onStart: () => {
        if (session.current !== active) return;
        const measured = measure(id, "pointer");
        if (measured === null) {
          closeSession(session);
          return;
        }
        active.scrollTop = active.scroller?.scrollTop ?? 0;
        followScroll(active, () => follow(active));
        lift(measured);
      },
      onMove: (y) => {
        if (session.current !== active) return;
        active.y = y;
        follow(active);
        active.edge.follow(y);
      },
      onDrop: () => {
        if (session.current === active) finish(true);
      },
      onCancel: () => {
        if (session.current === active) finish(false);
      },
    });
  };

  /** Keyboard drags: scrolls the list so the carried item's new slot is in view. */
  const revealSlot = (current: Carry, to: number) => {
    const active = session.current;
    const span = current.spans[current.from];
    if (active === null || span === undefined) return;
    const offset = sortOffsets(current.spans, current.from, to)[current.from] ?? 0;
    const top = span.top + offset - scrolledBy(active);
    reveal(active, { top, bottom: top + span.height });
  };

  const keyDown = (id: string, event: KeyboardEvent<HTMLElement>) => {
    const current = latest.current;
    if (current === null) {
      if (event.key !== " " || event.repeat || event.target !== event.currentTarget) return;
      event.preventDefault();
      spaceTaken.current = true;
      closeSession(session);
      const measured = measure(id, "keyboard");
      if (measured === null) return;
      const active = openSession(event.currentTarget, 0);
      session.current = active;
      watchPage(active, event.currentTarget, () => {
        if (session.current === active) finish(false);
      });
      lift(measured);
      return;
    }
    if (current.via !== "keyboard" || current.id !== id) return;
    const key = carryKey(event);
    if (key === null) return;
    event.preventDefault();
    if (key === "drop" || key === "cancel") {
      if (event.key === " ") spaceTaken.current = true;
      refocus.current = event.currentTarget;
      finish(key === "drop");
      return;
    }
    const to = stepIndex(current.to, key === "down" ? 1 : -1, current.ids.length);
    if (to === current.to) return;
    revealSlot(current, to);
    moveTo(current, to, 0);
  };

  const keyUp = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key !== " " || !spaceTaken.current) return;
    spaceTaken.current = false;
    event.preventDefault();
  };

  const offsets = carry === null ? null : sortOffsets(carry.spans, carry.from, carry.to);

  return {
    /** The id being carried, if any. */
    carrying: carry?.id ?? null,
    announcement,
    instructionsId,
    item: (id: string): SortableItemProps => {
      const ref = (node: HTMLElement | null) => {
        if (node === null) nodes.current.delete(id);
        else nodes.current.set(id, node);
      };
      if (carry === null || offsets === null) return { ref };
      const carried = id === carry.id;
      const y =
        carried && carry.via === "pointer"
          ? carry.shift
          : (offsets[carry.ids.indexOf(id)] ?? 0);
      // The pointer places the carried item itself; everything else, and the
      // carried item under the keyboard, slides to its place.
      const className = !carried
        ? "is-sorting"
        : carry.via === "pointer"
          ? "is-dragging"
          : "is-dragging is-sorting";
      return {
        ref,
        className,
        style: y === 0 ? undefined : { transform: `translateY(${y}px)` },
      };
    },
    handle: (id: string): DragHandleProps => ({
      onPointerDown: (event) => pressPointer(id, event),
      onKeyDown: (event) => keyDown(id, event),
      onKeyUp: keyUp,
      onBlur: () => {
        // Keys go to the focused handle, so a carry it can no longer hear ends.
        const current = latest.current;
        if (current?.via === "keyboard" && current.id === id) finish(false);
      },
      "aria-describedby": instructionsId,
      "aria-roledescription": roleDescription,
      "aria-pressed": carry?.id === id ? true : undefined,
    }),
  };
}
