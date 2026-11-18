import {
  nearestSlot,
  shiftAt,
  shiftBreaks,
  slotsOf,
  stepIndex,
  type CarriedSpan,
  type MeasuredList,
  type Slot,
} from "./geometry";
import { layoutTop, measureBlocks, planShifts, writeShifts } from "./layout";
import type { Frame, Motion, Stage } from "./motion";

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

export interface SortConfig {
  lists: readonly SortList[];
  messages: SortMessages;
  /** Applies a drop. The lists have to be rendered in their new order when it returns. */
  commit: (move: SortMove) => void;
  /** Where focus goes after a keyboard drop into a closed list, whose rows cannot take it. */
  focusClosed?: (list: string) => HTMLElement | null;
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

/** Reads the lists `id` can go to, or null when one of them is not on the page. */
function measure(stage: Stage, model: readonly SortList[], id: string) {
  const { scroller, items, lists } = stage;
  const home = model.findIndex((list) => list.items.includes(id));
  const homeList = model[home];
  const node = items.get(id);
  if (homeList === undefined || homeList.closed || node === undefined) return null;
  const targets = model.flatMap((list, index) => (list.kind === homeList.kind ? [index] : []));
  const measured: MeasuredList[] = [];
  const wrappers: HTMLElement[] = [];
  for (const index of targets) {
    const list = model[index];
    const wrapper = list === undefined ? undefined : lists.get(list.id);
    if (list === undefined || wrapper === undefined) return null;
    wrappers.push(wrapper);
    const spans = [];
    for (const item of list.items) {
      const itemNode = items.get(item);
      if (itemNode === undefined) return null;
      spans.push({ top: layoutTop(itemNode, scroller), height: itemNode.offsetHeight });
    }
    const top = layoutTop(wrapper, scroller);
    measured.push({ items: spans, end: list.closed ? top : top + wrapper.offsetHeight, closed: list.closed });
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
    blocks: measureBlocks(scroller, wrappers, node),
  };
}

/**
 * Several vertical lists sorted as one: an item can be dropped anywhere in a
 * list of its own kind, its own list or another, collapsed ones included.
 *
 * Everything is measured once, from the layout without transforms, so
 * scrolling never calls for measuring again. The carried item follows the
 * pointer and the rest of the page moves aside with transforms only, block by
 * block, so nothing reflows under the pointer.
 *
 * `press` is where a pointer grabbed the item, in the viewport, and how far
 * the list was scrolled then; a keyboard drag has none.
 */
export function createSortMotion(
  stage: Stage,
  config: () => SortConfig,
  id: string,
  handle: HTMLElement,
  frame: Frame,
  press: { y: number; scrollTop: number } | null,
): Motion | null {
  const model = config().lists;
  const first = measure(stage, model, id);
  if (first === null) return null;
  let measured = first;
  let slot = measured.home;
  let shifts = new Map<HTMLElement, number>();
  /** The closed list flagged as the target, if any. */
  let over: HTMLElement | null = null;
  /** Pointer drags: how far below the item's top it was grabbed, and the latest pointer. */
  let y = press?.y ?? 0;
  let scrollTop = press?.scrollTop ?? 0;
  const grab = press === null ? 0 : y - frame.origin + scrollTop - measured.carried.top;
  /** How far the carried item is drawn from its place. */
  let drawn = 0;

  const listOf = (at: Slot): SortList | undefined => model[measured.targets[at.list] ?? -1];

  const placeOf = (at: number): { list: string; position: number; count: number } => {
    const target = measured.slots[at];
    const list = target === undefined ? undefined : listOf(target);
    if (target === undefined || list === undefined) return { list: "", position: 0, count: 0 };
    const count = list.items.length + (target.list === measured.carried.list ? 0 : 1);
    return { list: list.id, position: target.index + 1, count };
  };

  const draw = (top: number) => {
    const by = Math.min(measured.highest, Math.max(measured.lowest, top)) - measured.carried.top;
    if (by === drawn) return;
    drawn = by;
    measured.node.style.transform = `translateY(${by}px)`;
  };

  /** Moves the rest of the rail for the current slot, and flags a closed list taking the drop. */
  const show = () => {
    const target = measured.slots[slot];
    if (target === undefined) return;
    const { carried } = measured;
    shifts = writeShifts(
      shifts,
      planShifts(measured.blocks, measured.node, shiftBreaks(carried, target), (at) =>
        shiftAt(at, carried, target),
      ),
    );
    const list = listOf(target);
    const flagged = list?.closed ? (stage.lists.get(list.id) ?? null) : null;
    if (flagged === over) return;
    over?.removeAttribute("data-drag-over");
    flagged?.setAttribute("data-drag-over", "");
    over = flagged;
  };

  const moveTo = (next: number) => {
    if (next === slot) return;
    slot = next;
    show();
    stage.say(config().messages.moved(id, placeOf(next)));
  };

  /** Pointer drags: where the carried item's top is held, in content pixels. */
  const heldTop = () => y - frame.origin + scrollTop - grab;

  const follow = () => {
    const held = heldTop();
    const next = nearestSlot(measured.slots, held);
    moveTo(next);
    // Over a closed list, which opens no room, the item docks right below
    // the caption that lights up, rather than hiding it under the pointer.
    const target = measured.slots[next];
    draw(target?.room === 0 ? target.lands : held);
  };

  const clear = () => {
    for (const node of stage.scroller.querySelectorAll<HTMLElement>("[data-drag]")) {
      node.style.transform = "";
      delete node.dataset.drag;
      delete node.dataset.via;
    }
    over?.removeAttribute("data-drag-over");
    handle.removeAttribute("aria-pressed");
  };

  /**
   * The dropped item eases from where it was let go into its slot; the rest
   * of the rail is already there.
   */
  const settle = (letGo: number) => {
    const node = stage.items.get(id);
    if (node === undefined) return;
    const distance = letGo - layoutTop(node, stage.scroller);
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

  return {
    lift: () => {
      const { node } = measured;
      delete node.dataset.settling;
      node.style.transform = "";
      node.dataset.drag = "carried";
      node.dataset.via = press === null ? "keyboard" : "pointer";
      handle.setAttribute("aria-pressed", "true");
      stage.say(config().messages.lifted(id, placeOf(measured.home)));
    },
    follow: (pointer, scrolled) => {
      y = pointer;
      scrollTop = scrolled;
      follow();
    },
    step: (direction, scrolled) => {
      const next = stepIndex(slot, direction, measured.slots.length);
      const target = measured.slots[next];
      if (next === slot || target === undefined) return null;
      moveTo(next);
      draw(target.lands);
      const top = target.lands + frame.origin - scrolled;
      return { top, bottom: top + measured.carried.height };
    },
    scrolled: () => {},
    finish: (dropped, pointer) => {
      if (dropped && pointer !== null) {
        y = pointer.y;
        scrollTop = pointer.scrollTop;
        const held = heldTop();
        slot = nearestSlot(measured.slots, held);
        drawn = Math.min(measured.highest, Math.max(measured.lowest, held)) - measured.carried.top;
      }
      const at = dropped ? slot : measured.home;
      const target = measured.slots[at];
      const list = target === undefined ? undefined : listOf(target);
      const letGo = measured.carried.top + drawn;
      clear();
      if (target === undefined || list === undefined) return null;

      const { messages, commit, focusClosed } = config();
      stage.say(dropped ? messages.dropped(id, placeOf(at)) : messages.cancelled(id));
      if (at !== measured.home) {
        const order = list.items.filter((item) => item !== id);
        order.splice(target.index, 0, id);
        commit({ id, list: list.id, order });
      }
      if (!list.closed) settle(letGo);
      // A drop moved or re-rendered the handle; focus stays with the item, or
      // with what names a closed list it went into.
      const moved = stage.handles.get(id);
      return moved !== undefined && moved.closest("[inert]") === null
        ? moved
        : (focusClosed?.(list.id) ?? null);
    },
    committed: () => {
      // The lists changed, as when someone else reorders them: the drag ends.
      if (!sameLists(model, config().lists)) return false;
      // They only changed size, as when someone joins a call: measure again.
      const fresh = measure(stage, model, id);
      if (fresh === null) return false;
      const was = measured.slots[slot];
      measured = fresh;
      const same = measured.slots.findIndex(
        (each) => each.list === was?.list && each.index === was.index,
      );
      slot = same === -1 ? measured.home : same;
      drawn = Number.NaN;
      if (press === null) draw(measured.slots[slot]?.lands ?? measured.carried.top);
      else follow();
      show();
      return true;
    },
    clear,
  };
}
