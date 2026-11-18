import { describe, expect, it } from "vitest";

import {
  edgeScrollSpeed,
  nearestSlot,
  revealDelta,
  shiftAt,
  shiftBreaks,
  slotsOf,
  spanAt,
  stepIndex,
  type CarriedSpan,
  type MeasuredList,
  type Span,
} from "./geometry";
import { planShifts, type Block } from "./layout";

/*
 * Two lists of 32 px rows with a 40 px caption between them:
 *
 *   list 0: a 100, b 132 (two 44 px threads open, 120 tall), c 252; ends at 284
 *   list 1: d 324, e 356; ends at 388
 *   list 2: closed, under its caption at 428
 */
const rows = (...tops: [number, number][]): Span[] => tops.map(([top, height]) => ({ top, height }));
const lists: MeasuredList[] = [
  { items: rows([100, 32], [132, 120], [252, 32]), end: 284, closed: false },
  { items: rows([324, 32], [356, 32]), end: 388, closed: false },
  { items: rows([428, 32]), end: 428, closed: true },
];
const carryA: CarriedSpan = { list: 0, index: 0, top: 100, height: 32 };

describe("slotsOf", () => {
  it("lands each slot where the carried item would be after the drop", () => {
    const slots = slotsOf(lists, carryA);

    expect(slots.map(({ list, index, lands }) => [list, index, lands])).toEqual([
      // Its own place, then below b and c, each lifted by the 32 px it left.
      [0, 0, 100],
      [0, 1, 220],
      [0, 2, 252],
      // The next list: before d, before e, at its end.
      [1, 0, 292],
      [1, 1, 324],
      [1, 2, 356],
      // A closed list: one slot, at its end, right under its caption.
      [2, 1, 396],
    ]);
    expect(slots.at(-1)?.room).toBe(0);
  });

  it("makes no room in the list a tall item leaves, and room for all of it in front", () => {
    const carryB: CarriedSpan = { list: 0, index: 1, top: 132, height: 120 };
    const slots = slotsOf(lists, carryB);

    // Before a, its own place, then below c once c has risen by 120.
    expect(slots.slice(0, 3).map((slot) => slot.lands)).toEqual([100, 132, 164]);
    expect(slots[3]?.room).toBe(120);
  });
});

describe("nearestSlot", () => {
  it("picks the slot that lands nearest the held top; the first wins a tie", () => {
    const slots = slotsOf(lists, carryA);

    expect(nearestSlot(slots, 90)).toBe(0);
    expect(nearestSlot(slots, 160)).toBe(0);
    expect(nearestSlot(slots, 161)).toBe(1);
    expect(nearestSlot(slots, 300)).toBe(3);
    expect(nearestSlot(slots, 900)).toBe(slots.length - 1);
    expect(nearestSlot([], 10)).toBe(-1);
  });
});

describe("shiftAt", () => {
  it("closes up behind the carried item and opens in front of it, and nowhere else", () => {
    const slots = slotsOf(lists, carryA);
    const beforeE = slots.find((slot) => slot.list === 1 && slot.index === 1)!;

    expect(shiftAt(132, carryA, beforeE)).toBe(-32);
    expect(shiftAt(300, carryA, beforeE)).toBe(-32);
    expect(shiftAt(356, carryA, beforeE)).toBe(0);
    expect(shiftAt(60, carryA, beforeE)).toBe(0);
    // Over its own place nothing moves.
    expect(shiftAt(132, carryA, slots[0]!)).toBe(0);
  });

  it("only closes up for a closed list, which shows nothing it takes", () => {
    const slots = slotsOf(lists, carryA);

    expect(shiftAt(500, carryA, slots.at(-1)!)).toBe(-32);
  });
});

describe("planShifts", () => {
  const node = () => document.createElement("div");
  const leaf = (top: number, height: number): Block => ({
    node: node(),
    top,
    height,
    children: null,
    holdsCarried: false,
  });

  it("moves whole blocks, opening only the ones the change runs through or that hold the item", () => {
    const carried = leaf(100, 32);
    const b = leaf(132, 120);
    const c = leaf(252, 32);
    const listZero: Block = { node: node(), top: 100, height: 184, children: [carried, b, c], holdsCarried: true };
    const caption = leaf(284, 40);
    const d = leaf(324, 32);
    const e = leaf(356, 32);
    const listOne: Block = { node: node(), top: 324, height: 64, children: [d, e], holdsCarried: false };
    const below = leaf(388, 200);
    const slot = slotsOf(lists, carryA).find((each) => each.list === 1 && each.index === 1)!;

    const moves = planShifts([listZero, caption, listOne, below], carried.node, shiftBreaks(carryA, slot), (y) =>
      shiftAt(y, carryA, slot),
    );

    expect([...moves]).toEqual([
      [b.node, -32],
      [c.node, -32],
      [caption.node, -32],
      [d.node, -32],
    ]);
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
