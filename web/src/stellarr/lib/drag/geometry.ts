/**
 * The arithmetic behind the rail's drags, kept apart from the DOM so it can be
 * checked without a layout engine.
 *
 * Every drag is locked to the vertical axis, so every measure here is a
 * vertical extent. Sorting works in content pixels, measured from the top of
 * the scrolling list, which scrolling does not change; the voice drop zones
 * work in viewport pixels.
 */

export interface Span {
  top: number;
  height: number;
}

export interface Bounds {
  top: number;
  bottom: number;
}

/** One list of a sort as it was measured. */
export interface MeasuredList {
  /** Its items in order, the carried one included where it sits. */
  items: readonly Span[];
  /** Where an item put after the last one would start. */
  end: number;
  /**
   * A collapsed list shows nothing it holds: it takes a drop at its end,
   * unseen, and opens no room for it.
   */
  closed: boolean;
}

/** The item being sorted: which list it is in, at which index, and where. */
export interface CarriedSpan extends Span {
  list: number;
  index: number;
}

/** Somewhere the carried item can be dropped. */
export interface Slot {
  list: number;
  /** Its index in that list once dropped, counted without the carried item. */
  index: number;
  /** Where in the measured layout it goes in: the rest of the rail from here moves down. */
  at: number;
  /** How far the rest of the rail moves down for it: the item's height, or 0 in a closed list. */
  room: number;
  /** Where the carried item's top lands; for a closed list, right below its caption. */
  lands: number;
}

/**
 * Every slot of `lists`, in rail order.
 *
 * Taking the carried item out lifts everything below it by its height, so a
 * slot further down lands that much higher than where it goes in. An open
 * list has a slot before each item and one at its end; a closed one has just
 * the end, which the item reaches when it is held right below the caption
 * that names the list, where the caption stays in sight.
 */
export function slotsOf(lists: readonly MeasuredList[], carried: CarriedSpan): Slot[] {
  const lands = (at: number) => (at > carried.top ? at - carried.height : at);
  return lists.flatMap((list, index) => {
    const items =
      index === carried.list ? list.items.filter((_, item) => item !== carried.index) : list.items;
    if (list.closed) {
      return [{ list: index, index: items.length, at: list.end, room: 0, lands: lands(list.end) }];
    }
    const at = [...items.map((item) => item.top), list.end];
    return at.map((y, slot) => ({
      list: index,
      index: slot,
      at: y,
      room: carried.height,
      lands: lands(y),
    }));
  });
}

/** The slot whose landing place is nearest `top`; the first wins a tie. */
export function nearestSlot(slots: readonly Slot[], top: number): number {
  let best = -1;
  let bestDistance = Infinity;
  slots.forEach((slot, index) => {
    const distance = Math.abs(slot.lands - top);
    if (distance < bestDistance) {
      best = index;
      bestDistance = distance;
    }
  });
  return best;
}

/**
 * How far whatever starts at `y` in the measured layout moves while the
 * carried item is over `slot`: up by the item's height once below where it
 * was taken from, and down by the room the slot opens once below where it
 * goes in. Between the two the rail closes up behind the item and opens in
 * front of it; outside them both cancel out.
 */
export function shiftAt(y: number, carried: Span, slot: Slot): number {
  return (y >= carried.top + carried.height ? -carried.height : 0) + (y >= slot.at ? slot.room : 0);
}

/** The places where `shiftAt` changes, which a block moved as one must not straddle. */
export function shiftBreaks(carried: Span, slot: Slot): number[] {
  return [carried.top + carried.height, slot.at];
}

/** One keyboard step from `index` among `count` targets, stopping at either end. */
export function stepIndex(index: number, step: number, count: number): number {
  return Math.min(count - 1, Math.max(0, index + step));
}

/** The index of the span that `y` falls in, or -1 when it is in none. */
export function spanAt(spans: readonly Span[], y: number): number {
  return spans.findIndex((span) => y >= span.top && y < span.top + span.height);
}

/** How far to scroll `view` so that all of `target` is inside it; 0 when it already is. */
export function revealDelta(target: Bounds, view: Bounds): number {
  if (target.top < view.top) return target.top - view.top;
  if (target.bottom > view.bottom) return target.bottom - view.bottom;
  return 0;
}

/** The band along each edge of a scrolling list where a drag scrolls it. */
const EDGE_SHARE = 0.2;
const EDGE_MAX = 72;
/** Pixels per frame at the very edge; the speed falls off linearly across the band. */
const EDGE_SPEED = 14;

/**
 * How many pixels to scroll a list this frame while the pointer is at `y`:
 * negative is up, 0 outside the edge bands. Past an edge is full speed.
 */
export function edgeScrollSpeed(y: number, view: Bounds): number {
  const band = Math.min(EDGE_MAX, (view.bottom - view.top) * EDGE_SHARE);
  if (band <= 0) return 0;
  if (y < view.top + band) return -EDGE_SPEED * Math.min(1, (view.top + band - y) / band);
  if (y > view.bottom - band) return EDGE_SPEED * Math.min(1, (y - (view.bottom - band)) / band);
  return 0;
}
