import { describe, expect, it } from "vitest";

import {
  clampShift,
  closestIndex,
  edgeScrollSpeed,
  moveItem,
  revealDelta,
  sortOffsets,
  spanAt,
  stepIndex,
  type Span,
} from "./geometry";

/** Three 32 px rows stacked from y = 100, the middle one with two 44 px threads open. */
const rows: Span[] = [
  { top: 100, height: 32 },
  { top: 132, height: 120 },
  { top: 252, height: 32 },
];

describe("moveItem", () => {
  it("lands a downward move on the slot it was dropped on, not one short of it", () => {
    expect(moveItem(["a", "b", "c"], 0, 2)).toEqual(["b", "c", "a"]);
  });

  it("moves upwards and leaves the source untouched", () => {
    const ids = ["a", "b", "c"];
    expect(moveItem(ids, 2, 0)).toEqual(["c", "a", "b"]);
    expect(ids).toEqual(["a", "b", "c"]);
  });

  it("keeps the order when the item stays where it was", () => {
    expect(moveItem(["a", "b", "c"], 1, 1)).toEqual(["a", "b", "c"]);
  });
});

describe("closestIndex", () => {
  it("picks the row whose centre is nearest, however tall it is", () => {
    expect(closestIndex(rows, 116)).toBe(0);
    // Past the midpoint between the first centre (116) and the tall one (192).
    expect(closestIndex(rows, 160)).toBe(1);
    expect(closestIndex(rows, 300)).toBe(2);
  });

  it("finds nothing in an empty list", () => {
    expect(closestIndex([], 10)).toBe(-1);
  });
});

describe("sortOffsets", () => {
  it("moves nothing while the item is over its own slot", () => {
    expect(sortOffsets(rows, 1, 1)).toEqual([0, 0, 0]);
  });

  it("shifts the others by the carried item's own height, threads included", () => {
    // The tall row goes last: the bottom row rises by 120, the tall one drops by 32.
    expect(sortOffsets(rows, 1, 2)).toEqual([0, 32, -120]);
    // A short row goes below the tall one: the tall one rises by 32 only.
    expect(sortOffsets(rows, 0, 1)).toEqual([120, -32, 0]);
  });

  it("keeps the spacing between positions", () => {
    const spaced: Span[] = [
      { top: 0, height: 30 },
      { top: 34, height: 30 },
      { top: 68, height: 30 },
    ];
    expect(sortOffsets(spaced, 2, 0)).toEqual([34, 34, -68]);
  });

  it("gives an empty list nothing to move", () => {
    expect(sortOffsets([], 0, 0)).toEqual([]);
  });
});

describe("clampShift", () => {
  const bounds = { top: 100, bottom: 284 };

  it("keeps the carried row inside the list", () => {
    expect(clampShift(rows[0]!, -50, bounds)).toBe(0);
    expect(clampShift(rows[0]!, 500, bounds)).toBe(152);
    expect(clampShift(rows[2]!, -500, bounds)).toBe(-152);
  });

  it("passes a shift that stays inside unchanged", () => {
    expect(clampShift(rows[0]!, 40, bounds)).toBe(40);
  });
});

describe("stepIndex", () => {
  it("moves one target at a time and stops at the ends", () => {
    expect(stepIndex(0, 1, 3)).toBe(1);
    expect(stepIndex(2, 1, 3)).toBe(2);
    expect(stepIndex(0, -1, 3)).toBe(0);
  });
});

describe("spanAt", () => {
  it("finds the span under a point and nothing between or beyond them", () => {
    const zones: Span[] = [
      { top: 0, height: 50 },
      { top: 60, height: 50 },
    ];
    expect(spanAt(zones, 0)).toBe(0);
    expect(spanAt(zones, 49)).toBe(0);
    expect(spanAt(zones, 55)).toBe(-1);
    expect(spanAt(zones, 60)).toBe(1);
    expect(spanAt(zones, 200)).toBe(-1);
  });
});

describe("revealDelta", () => {
  const view = { top: 100, bottom: 400 };

  it("scrolls only as far as the target needs", () => {
    expect(revealDelta({ top: 150, bottom: 182 }, view)).toBe(0);
    expect(revealDelta({ top: 80, bottom: 112 }, view)).toBe(-20);
    expect(revealDelta({ top: 390, bottom: 422 }, view)).toBe(22);
  });
});

describe("edgeScrollSpeed", () => {
  const view = { top: 0, bottom: 500 };

  it("stays still away from the edges", () => {
    expect(edgeScrollSpeed(250, view)).toBe(0);
  });

  it("speeds up towards an edge and goes full speed past it", () => {
    const near = edgeScrollSpeed(480, view);
    const nearer = edgeScrollSpeed(495, view);
    expect(near).toBeGreaterThan(0);
    expect(nearer).toBeGreaterThan(near);
    expect(edgeScrollSpeed(600, view)).toBe(edgeScrollSpeed(500, view));
    expect(edgeScrollSpeed(-20, view)).toBe(-edgeScrollSpeed(520, view));
  });

  it("does nothing in a list with no height", () => {
    expect(edgeScrollSpeed(0, { top: 0, bottom: 0 })).toBe(0);
  });
});
