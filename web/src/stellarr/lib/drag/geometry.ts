/**
 * The arithmetic behind the rail's two drags, kept apart from the DOM so it
 * can be checked without a layout engine.
 *
 * Both drags are locked to the vertical axis, so every measure here is a
 * vertical extent in viewport pixels.
 */

export interface Span {
  top: number;
  height: number;
}

export interface Bounds {
  top: number;
  bottom: number;
}

/** `items` with the entry at `from` taken out and put back at `to`. */
export function moveItem<T>(items: readonly T[], from: number, to: number): T[] {
  const next = [...items];
  const [moved] = next.splice(from, 1);
  if (moved !== undefined) next.splice(to, 0, moved);
  return next;
}

/** The index of the span whose centre is nearest `centre`; the first wins a tie. */
export function closestIndex(spans: readonly Span[], centre: number): number {
  let best = -1;
  let bestDistance = Infinity;
  spans.forEach((span, index) => {
    const distance = Math.abs(span.top + span.height / 2 - centre);
    if (distance < bestDistance) {
      best = index;
      bestDistance = distance;
    }
  });
  return best;
}

/**
 * How far each item has to move for the one at `from` to sit at `to`.
 *
 * Items keep their own heights, so a channel with its threads open moves as
 * one block, and each position keeps the gap that followed it. The entry at
 * `from` is where the carried item will land; the others are the shifts that
 * make room for it. `spans` are in list order.
 */
export function sortOffsets(spans: readonly Span[], from: number, to: number): number[] {
  const offsets = spans.map(() => 0);
  const first = spans[0];
  if (first === undefined) return offsets;

  let top = first.top;
  moveItem(
    spans.map((_, index) => index),
    from,
    to,
  ).forEach((index, position) => {
    const span = spans[index];
    const here = spans[position];
    const next = spans[position + 1];
    if (span === undefined || here === undefined) return;
    offsets[index] = top - span.top;
    const gap = next === undefined ? 0 : next.top - (here.top + here.height);
    top += span.height + gap;
  });
  return offsets;
}

/** `shift` limited so that `span`, moved by it, stays inside `bounds`. */
export function clampShift(span: Span, shift: number, bounds: Bounds): number {
  const up = bounds.top - span.top;
  const down = bounds.bottom - (span.top + span.height);
  return Math.min(down, Math.max(up, shift));
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
