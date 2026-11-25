/**
 * Where the reader is, in terms that survive the rows changing around them.
 *
 * A scroll offset stops meaning anything as soon as something above it
 * changes height: an older page is prepended, an image above the viewport
 * finishes loading, a reaction is added to a message the reader has already
 * passed. What the reader is looking at is a row, so the place is kept as rows
 * and their distance from the viewport's top edge, and put back by moving the
 * scroller until the first of them that still exists is where it was.
 *
 * Putting it back is absolute, not a delta, so doing it twice for the same
 * change moves nothing the second time.
 */

/** A row's box, measured from the viewport's top edge. */
export interface RowBox {
  id: string;
  /**
   * Only message blocks hold the reader's place. Older messages can land in
   * the same day and leave its separator exactly where it was while the
   * reading position moved, and the unread line comes and goes.
   */
  anchor: boolean;
  top: number;
  bottom: number;
}

/** A row in or near view, and how far below the viewport's top edge it starts. */
export interface ReadingMark {
  id: string;
  offset: number;
}

/** The index of the first row that extends below the top of the viewport. */
function firstVisible(
  count: number,
  boxAt: (index: number) => RowBox,
  previous: number | undefined,
): number {
  let low = 0;
  let high = count;
  // A wheel or touch scroll usually moves only a few rows. Check near the
  // last mark first and use binary search for longer jumps.
  if (previous !== undefined && previous < count) {
    if (boxAt(previous).bottom <= 0) {
      low = previous + 1;
      for (let step = 0; low < count && step < 4; step += 1, low += 1) {
        if (boxAt(low).bottom > 0) return low;
      }
    } else {
      high = previous;
      for (let step = 0; high > 0 && step < 4; step += 1, high -= 1) {
        if (boxAt(high - 1).bottom <= 0) return high;
      }
    }
  }
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (boxAt(middle).bottom <= 0) low = middle + 1;
    else high = middle;
  }
  return low;
}

/**
 * The rows that can hold the reader's place, best first.
 *
 * Every message block in view, top to bottom, then the nearest one below the
 * viewport and the nearest one above it. More than one, because the row the
 * reader was on may not survive the change: a page boundary can merge the
 * oldest loaded block with the newest one of the page before it, under a new
 * id. The next row down then holds the place instead.
 *
 * Rows are in document order, so the first one in view is found by halving
 * rather than by measuring every row of a long history.
 */
export function readingMarks(
  count: number,
  boxAt: (index: number) => RowBox,
  viewportHeight: number,
  previous?: number,
): ReadingMark[] {
  const marks: ReadingMark[] = [];
  const first = firstVisible(count, boxAt, previous);
  const mark = (box: RowBox) => marks.push({ id: box.id, offset: box.top });

  let index = first;
  for (; index < count; index += 1) {
    const box = boxAt(index);
    if (box.top >= viewportHeight) break;
    if (box.anchor) mark(box);
  }
  for (; index < count; index += 1) {
    const box = boxAt(index);
    if (box.anchor) {
      mark(box);
      break;
    }
  }
  for (let above = first - 1; above >= 0; above -= 1) {
    const box = boxAt(above);
    if (box.anchor) {
      mark(box);
      break;
    }
  }
  return marks;
}

/**
 * The scroll offset that puts the first surviving mark back where it was, or
 * null when none of them is still on the page.
 *
 * `offsetOf` measures a row now, in the same terms as the mark was taken.
 */
export function anchoredScrollTop(
  scrollTop: number,
  marks: readonly ReadingMark[],
  offsetOf: (id: string) => number | undefined,
): number | null {
  for (const mark of marks) {
    const offset = offsetOf(mark.id);
    if (offset === undefined) continue;
    return scrollTop + offset - mark.offset;
  }
  return null;
}

/** The furthest the scroller can go. */
export function endScrollTop(scrollHeight: number, clientHeight: number): number {
  return Math.max(0, scrollHeight - clientHeight);
}

/**
 * Exactly at the end, give or take the pixel a fractional device ratio rounds
 * away. Exact rather than thresholded: a reader 20 px above the end is
 * reading, and is left alone when something below them grows.
 */
export function isAtEnd(scrollTop: number, scrollHeight: number, clientHeight: number): boolean {
  return Math.abs(scrollTop - endScrollTop(scrollHeight, clientHeight)) <= 1;
}

/** How far the reader is from the end. */
export function distanceFromEnd(
  scrollTop: number,
  scrollHeight: number,
  clientHeight: number,
): number {
  return endScrollTop(scrollHeight, clientHeight) - scrollTop;
}

/**
 * The scroll offset that centres a row in the viewport, held to the range the
 * scroller can reach. `rowTop` is measured from the viewport's top edge.
 */
export function centredScrollTop(
  scrollTop: number,
  rowTop: number,
  rowHeight: number,
  clientHeight: number,
  scrollHeight: number,
): number {
  const centred = scrollTop + rowTop - (clientHeight - rowHeight) / 2;
  return Math.min(endScrollTop(scrollHeight, clientHeight), Math.max(0, centred));
}
