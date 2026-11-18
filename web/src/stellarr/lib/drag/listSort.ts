import type { KeyboardEvent, PointerEvent } from "react";

import { createAnnouncer } from "./announcer";
import {
  edgeScrollSpeed,
  nearestSlot,
  revealDelta,
  shiftAt,
  shiftBreaks,
  slotsOf,
  stepIndex,
  type Bounds,
  type CarriedSpan,
  type MeasuredList,
  type Slot,
} from "./geometry";
import { layoutTop, measureBlocks, planShifts, writeShifts, type Block } from "./layout";
import { holdFingerOn, trackPointer, type PointerDrag } from "./pointer";
import { carryKey, restoreFocus, watchPage } from "./session";

/** One list of a sort, in display order. */
export interface SortList {
  id: string;
  /** Items drop only into lists of their own kind. */
  kind: string;
  items: readonly string[];
  /** Collapsed: a drop lands at its end, unseen, and nothing in it can be picked up. */
  closed: boolean;
}

/** Where an item is or would be, for what a screen reader hears; positions count from 1. */
export interface SortPlace {
  list: string;
  position: number;
  count: number;
}

export interface SortMessages {
  lifted: (id: string, place: SortPlace) => string;
  moved: (id: string, place: SortPlace) => string;
  dropped: (id: string, place: SortPlace) => string;
  cancelled: (id: string) => string;
}

/** A drop that changed something: the list the item landed in, in its new order. */
export interface SortMove {
  id: string;
  list: string;
  order: string[];
}

export interface ListSortConfig {
  lists: readonly SortList[];
  messages: SortMessages;
  /** Applies a drop. The lists have to be rendered in their new order when it returns. */
  commit: (move: SortMove) => void;
  /** Where focus goes after a keyboard drop into a closed list, whose rows cannot take it. */
  focusClosed?: (list: string) => HTMLElement | null;
}

type Ref = (node: HTMLElement | null) => void | (() => void);

interface Drag {
  id: string;
  via: "pointer" | "keyboard";
  finger: boolean;
  /** The lists when the drag began; any other set of lists ends it. */
  model: readonly SortList[];
  /** The lists of the item's kind, as indices into `model`, in order. */
  targets: number[];
  node: HTMLElement;
  handle: HTMLElement;
  carried: CarriedSpan;
  blocks: Block[];
  slots: Slot[];
  /** The highest and lowest the carried item is drawn, so it stays among the slots. */
  lowest: number;
  highest: number;
  home: number;
  slot: number;
  shifts: Map<HTMLElement, number>;
  /** The closed list flagged as the target, if any. */
  over: HTMLElement | null;
  /**
   * Where the list's content starts in the viewport when it is not scrolled,
   * and the part of the viewport the list shows.
   */
  origin: number;
  view: Bounds;
  /** Pointer drags: how far below the item's top it was grabbed, and the pointer's latest height. */
  grab: number;
  y: number;
  /**
   * The list's scroll offset, kept here rather than read in each frame: by
   * then hovering has usually invalidated the styles, and reading would
   * recompute them. Its own edge scrolling sets it, a scroll event from
   * anything else reads it.
   */
  scrollTop: number;
  maxScroll: number;
  /** The next scroll event is the drag's own edge scrolling. */
  ownScroll: boolean;
  /** How far the carried item is drawn from its place. */
  drawn: number;
  frame: number;
  pointer: PointerDrag | null;
  listening: AbortController;
}

const sameLists = (a: readonly SortList[], b: readonly SortList[]) =>
  a.length === b.length &&
  a.every((list, index) => {
    const other = b[index];
    return (
      other !== undefined &&
      other.id === list.id &&
      other.kind === list.kind &&
      other.closed === list.closed &&
      other.items.length === list.items.length &&
      other.items.every((id, item) => id === list.items[item])
    );
  });

/**
 * Several vertical lists sorted as one, with the pointer or the keyboard: an
 * item can be dropped anywhere in a list of its own kind, its own list or
 * another, collapsed ones included.
 *
 * Everything is measured once when a drag begins, from the layout without
 * transforms, so scrolling never calls for measuring again. The carried item
 * follows the pointer and the rest of the page moves aside with transforms
 * only, block by block, so nothing reflows under the pointer. All of it is
 * written straight to the DOM, once a frame at most: React renders nothing
 * while an item is carried, and the live region re-renders once per new
 * slot.
 *
 * The scrolling element is the container, and has to be positioned.
 */
export function createListSort() {
  const announcer = createAnnouncer();
  let config: ListSortConfig | null = null;
  let scroller: HTMLElement | null = null;
  const items = new Map<string, HTMLElement>();
  const lists = new Map<string, HTMLElement>();
  const handles = new Map<string, HTMLElement>();
  const refs = new Map<string, Ref>();
  let drag: Drag | null = null;
  /** A press not yet a drag, which a new press replaces. */
  let pressing: PointerDrag | null = null;
  /** A Space that picked up or dropped; its keyup must not click the button. */
  let spaceTaken = false;

  /** A ref callback per element and id, the same every render, so React does not detach and attach it. */
  const cached = (key: string, make: () => Ref): Ref => {
    let ref = refs.get(key);
    if (ref === undefined) {
      ref = make();
      refs.set(key, ref);
    }
    return ref;
  };
  const registry = (map: Map<string, HTMLElement>, id: string) => (node: HTMLElement | null) => {
    if (node === null) map.delete(id);
    else map.set(id, node);
  };

  /** Reads the lists the item can go to, or null when one of them is not on the page. */
  const measure = (id: string, model: readonly SortList[]) => {
    const home = model.findIndex((list) => list.items.includes(id));
    const homeList = model[home];
    const node = items.get(id);
    if (scroller === null || homeList === undefined || homeList.closed || node === undefined) {
      return null;
    }
    const targets = model.flatMap((list, index) => (list.kind === homeList.kind ? [index] : []));
    const measured: MeasuredList[] = [];
    const nodes: HTMLElement[] = [];
    for (const index of targets) {
      const list = model[index];
      const wrapper = list === undefined ? undefined : lists.get(list.id);
      if (list === undefined || wrapper === undefined) return null;
      nodes.push(wrapper);
      const spans = [];
      for (const item of list.items) {
        const itemNode = items.get(item);
        if (itemNode === undefined) return null;
        spans.push({ top: layoutTop(itemNode, scroller), height: itemNode.offsetHeight });
      }
      const top = layoutTop(wrapper, scroller);
      measured.push({
        items: spans,
        end: list.closed ? top : top + wrapper.offsetHeight,
        closed: list.closed,
      });
    }
    const carried: CarriedSpan = {
      list: targets.indexOf(home),
      index: homeList.items.indexOf(id),
      top: layoutTop(node, scroller),
      height: node.offsetHeight,
    };
    const slots = slotsOf(measured, carried);
    const lands = slots.map((slot) => slot.lands);
    return {
      node,
      targets,
      carried,
      slots,
      lowest: Math.min(...lands),
      highest: Math.max(...lands),
      home: slots.findIndex((slot) => slot.list === carried.list && slot.index === carried.index),
      blocks: measureBlocks(scroller, nodes, node),
    };
  };

  const listOf = (current: Drag, slot: Slot): SortList | undefined =>
    current.model[current.targets[slot.list] ?? -1];

  const placeOf = (current: Drag, at: number): SortPlace => {
    const slot = current.slots[at];
    const list = slot === undefined ? undefined : listOf(current, slot);
    if (slot === undefined || list === undefined) return { list: "", position: 0, count: 0 };
    const count = list.items.length + (slot.list === current.carried.list ? 0 : 1);
    return { list: list.id, position: slot.index + 1, count };
  };

  const draw = (current: Drag, top: number) => {
    const by = Math.min(current.highest, Math.max(current.lowest, top)) - current.carried.top;
    if (by === current.drawn) return;
    current.drawn = by;
    current.node.style.transform = `translateY(${by}px)`;
  };

  /** Moves the rest of the rail for the current slot, and flags a closed list taking the drop. */
  const show = (current: Drag) => {
    const slot = current.slots[current.slot];
    if (slot === undefined) return;
    current.shifts = writeShifts(
      current.shifts,
      planShifts(current.blocks, current.node, shiftBreaks(current.carried, slot), (y) =>
        shiftAt(y, current.carried, slot),
      ),
    );
    const list = listOf(current, slot);
    const over = list?.closed ? (lists.get(list.id) ?? null) : null;
    if (over === current.over) return;
    current.over?.removeAttribute("data-drag-over");
    over?.setAttribute("data-drag-over", "");
    current.over = over;
  };

  const moveTo = (current: Drag, slot: number) => {
    if (slot === current.slot || config === null) return;
    current.slot = slot;
    show(current);
    announcer.say(config.messages.moved(current.id, placeOf(current, slot)));
  };

  /** Pointer drags: where the carried item's top is held, in content pixels. */
  const heldTop = (current: Drag, scrollTop: number) =>
    current.y - current.origin + scrollTop - current.grab;

  const follow = (current: Drag, scrollTop: number) => {
    const held = heldTop(current, scrollTop);
    const slot = nearestSlot(current.slots, held);
    moveTo(current, slot);
    // Over a closed list, which opens no room, the item docks right below
    // the caption that lights up, rather than hiding it under the pointer.
    const target = current.slots[slot];
    draw(current, target?.room === 0 ? target.lands : held);
  };

  const frameOf = (current: Drag) => current.node.ownerDocument.defaultView ?? window;

  /**
   * One frame of a pointer drag: the carried item and, on a new slot, the
   * rest of the rail. Nothing is read from the page, so nothing waits for a
   * layout.
   */
  const tick = (current: Drag) => {
    current.frame = 0;
    const list = scroller;
    if (drag !== current || list === null) return;
    follow(current, current.scrollTop);
    const speed = edgeScrollSpeed(current.y, current.view);
    const step = Math.sign(speed) * Math.max(1, Math.round(Math.abs(speed)));
    const scrollTop = Math.min(current.maxScroll, Math.max(0, current.scrollTop + step));
    // At the end of the list there is nothing left to scroll; the next move
    // starts the loop again if the pointer heads back.
    if (speed === 0 || scrollTop === current.scrollTop) return;
    // Scrolling has to lay the page out first. Once this frame is drawn there
    // is nothing left to lay out, and the next frame draws the scroll and the
    // rows over it together.
    frameOf(current).setTimeout(() => {
      if (drag !== current) return;
      current.scrollTop = scrollTop;
      current.ownScroll = true;
      list.scrollTop = scrollTop;
      request(current);
    }, 0);
  };

  const request = (current: Drag) => {
    if (current.frame === 0) current.frame = frameOf(current).requestAnimationFrame(() => tick(current));
  };

  /** Starts a drag; `pressedAt` is the viewport height a pointer grabbed the item at. */
  const begin = (
    id: string,
    via: Drag["via"],
    handle: HTMLElement,
    finger: boolean,
    pressedAt = 0,
  ): Drag | null => {
    if (config === null || scroller === null) return null;
    const measured = measure(id, config.lists);
    if (measured === null) return null;
    const box = scroller.getBoundingClientRect();
    const origin = box.top + scroller.clientTop;
    const scrollTop = scroller.scrollTop;
    const current: Drag = {
      id,
      via,
      finger,
      model: config.lists,
      handle,
      ...measured,
      slot: measured.home,
      shifts: new Map(),
      over: null,
      origin,
      view: { top: box.top, bottom: box.bottom },
      grab: pressedAt - origin + scrollTop - measured.carried.top,
      y: pressedAt,
      scrollTop,
      maxScroll: scroller.scrollHeight - scroller.clientHeight,
      ownScroll: false,
      drawn: 0,
      frame: 0,
      pointer: null,
      listening: new AbortController(),
    };
    drag = current;
    // Everything is read; from here on the drag only writes.
    delete measured.node.dataset.settling;
    measured.node.style.transform = "";
    measured.node.dataset.drag = "carried";
    measured.node.dataset.via = via;
    handle.setAttribute("aria-pressed", "true");
    scroller.dataset.sorting = "";
    announcer.say(config.messages.lifted(id, placeOf(current, current.home)));
    return current;
  };

  const stop = (current: Drag) => {
    if (current.frame !== 0) frameOf(current).cancelAnimationFrame(current.frame);
    current.frame = 0;
    current.listening.abort();
    current.pointer?.cancel();
  };

  const clear = (current: Drag) => {
    for (const node of scroller?.querySelectorAll<HTMLElement>("[data-drag]") ?? []) {
      node.style.transform = "";
      delete node.dataset.drag;
      delete node.dataset.via;
    }
    current.over?.removeAttribute("data-drag-over");
    current.handle.removeAttribute("aria-pressed");
    if (scroller !== null) delete scroller.dataset.sorting;
  };

  /**
   * The dropped item eases from where it was let go into its slot; the rest
   * of the rail is already there.
   */
  const settle = (id: string, letGo: number) => {
    const node = items.get(id);
    if (node === undefined || scroller === null) return;
    const distance = letGo - layoutTop(node, scroller);
    if (Math.abs(distance) < 1) return;
    node.style.transform = `translateY(${distance}px)`;
    // The start of the ease has to be drawn before it can ease.
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
  };

  const finish = (dropped: boolean) => {
    const current = drag;
    if (current === null || config === null) return;
    drag = null;
    stop(current);
    // A release can come before the frame that would have drawn the last move.
    if (dropped && current.via === "pointer") {
      const held = heldTop(current, current.scrollTop);
      current.slot = nearestSlot(current.slots, held);
      current.drawn =
        Math.min(current.highest, Math.max(current.lowest, held)) - current.carried.top;
    }
    const at = dropped ? current.slot : current.home;
    const slot = current.slots[at];
    const list = slot === undefined ? undefined : listOf(current, slot);
    const letGo = current.carried.top + current.drawn;
    clear(current);
    if (slot === undefined || list === undefined) return;

    const { messages, commit, focusClosed } = config;
    announcer.say(
      dropped ? messages.dropped(current.id, placeOf(current, at)) : messages.cancelled(current.id),
    );
    if (at !== current.home) {
      const order = list.items.filter((item) => item !== current.id);
      order.splice(slot.index, 0, current.id);
      commit({ id: current.id, list: list.id, order });
    }
    if (!list.closed) settle(current.id, letGo);
    if (current.via !== "keyboard") return;
    // A keyboard drop moved or re-rendered the handle; focus stays with the
    // item, or with what names a closed list it went into.
    const handle = handles.get(current.id);
    restoreFocus(
      handle !== undefined && handle.closest("[inert]") === null
        ? handle
        : (focusClosed?.(list.id) ?? null),
    );
  };

  /** The lists changed size under a drag, as when someone joins a call: measure again. */
  const remeasure = (current: Drag) => {
    const fresh = measure(current.id, current.model);
    if (fresh === null || scroller === null) {
      finish(false);
      return;
    }
    const was = current.slots[current.slot];
    Object.assign(current, fresh);
    const same = current.slots.findIndex(
      (slot) => slot.list === was?.list && slot.index === was.index,
    );
    current.slot = same === -1 ? current.home : same;
    current.drawn = Number.NaN;
    if (current.via === "pointer") follow(current, current.scrollTop);
    else draw(current, current.slots[current.slot]?.lands ?? current.carried.top);
    show(current);
  };

  const pressPointer = (id: string, event: PointerEvent<HTMLElement>) => {
    if (!event.isPrimary || event.button !== 0 || drag !== null) return;
    pressing?.cancel();
    const handle = event.currentTarget;
    const pressedAt = event.clientY;
    const finger = event.pointerType === "touch";
    let current: Drag | null = null;
    const press = trackPointer(event.nativeEvent, handle, {
      onStart: () => {
        pressing = null;
        current = begin(id, "pointer", handle, finger, pressedAt);
        if (current === null) {
          press.cancel();
          return;
        }
        const started = current;
        started.pointer = press;
        // Edge scrolling and the wheel move the list under a still pointer.
        const list = scroller;
        list?.addEventListener(
          "scroll",
          () => {
            if (started.ownScroll) started.ownScroll = false;
            else started.scrollTop = list.scrollTop;
            request(started);
          },
          { signal: started.listening.signal },
        );
      },
      onMove: (y) => {
        if (current === null || drag !== current) return;
        current.y = y;
        request(current);
      },
      onDrop: () => {
        if (current !== null && drag === current) finish(true);
      },
      onCancel: () => {
        if (current !== null && drag === current) finish(false);
      },
    });
    pressing = press;
  };

  /** Keyboard drags: scrolls just far enough for the item's new place to be in view. */
  const reveal = (current: Drag, scrollTop: number) => {
    const slot = current.slots[current.slot];
    if (slot === undefined || scroller === null) return;
    const top = slot.lands + current.origin - scrollTop;
    const delta = revealDelta({ top, bottom: top + current.carried.height }, current.view);
    if (delta !== 0) scroller.scrollTop = scrollTop + delta;
  };

  const keyDown = (id: string, event: KeyboardEvent<HTMLElement>) => {
    const current = drag;
    if (current === null) {
      if (event.key !== " " || event.repeat || event.target !== event.currentTarget) return;
      event.preventDefault();
      spaceTaken = true;
      pressing?.cancel();
      const handle = event.currentTarget;
      const started = begin(id, "keyboard", handle, false);
      if (started === null) return;
      watchPage(started.listening.signal, handle, () => {
        if (drag === started) finish(false);
      });
      return;
    }
    if (current.via !== "keyboard" || current.id !== id) return;
    const key = carryKey(event);
    if (key === null) return;
    event.preventDefault();
    if (key === "drop" || key === "cancel") {
      if (event.key === " ") spaceTaken = true;
      finish(key === "drop");
      return;
    }
    const next = stepIndex(current.slot, key === "down" ? 1 : -1, current.slots.length);
    if (next === current.slot || scroller === null) return;
    const scrollTop = scroller.scrollTop;
    moveTo(current, next);
    reveal(current, scrollTop);
    draw(current, current.slots[next]?.lands ?? current.carried.top);
  };

  return {
    announcer,
    configure: (next: ListSortConfig) => {
      config = next;
    },
    /**
     * After every render of the lists' owner. A drag whose lists changed,
     * as when someone else reorders them, ends; one whose lists only
     * changed size is measured again.
     */
    committed: () => {
      const current = drag;
      if (current === null || config === null) return;
      if (sameLists(current.model, config.lists)) remeasure(current);
      else finish(false);
    },
    dispose: () => {
      const current = drag;
      drag = null;
      pressing?.cancel();
      if (current === null) return;
      stop(current);
      clear(current);
    },
    container: (node: HTMLElement | null) => {
      scroller = node;
    },
    list: (id: string) => cached(`list:${id}`, () => registry(lists, id)),
    item: (id: string) => cached(`item:${id}`, () => registry(items, id)),
    handleRef: (id: string) =>
      cached(`handle:${id}`, () => (node: HTMLElement | null) => {
        if (node === null) return;
        handles.set(id, node);
        const release = holdFingerOn(node, () => drag?.finger === true);
        return () => {
          release();
          if (handles.get(id) === node) handles.delete(id);
        };
      }),
    pressPointer,
    keyDown,
    keyUp: (event: KeyboardEvent<HTMLElement>) => {
      if (event.key !== " " || !spaceTaken) return;
      spaceTaken = false;
      event.preventDefault();
    },
    /** Keys go to the focused handle, so a carry it can no longer hear ends. */
    blur: (id: string) => {
      if (drag?.via === "keyboard" && drag.id === id) finish(false);
    },
  };
}

export type ListSort = ReturnType<typeof createListSort>;
