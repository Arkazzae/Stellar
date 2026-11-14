import { describe, expect, it } from "vitest";

import {
  anchoredScrollTop,
  centredScrollTop,
  distanceFromEnd,
  isAtEnd,
  readingMarks,
  type RowBox,
} from "./scrollAnchor";

/** Rows stacked from `start`, each `[id, height, anchor]`, measured from the viewport top. */
function stack(start: number, ...rows: Array<[string, number, boolean?]>): RowBox[] {
  let top = start;
  return rows.map(([id, height, anchor = true]) => {
    const box = { id, anchor, top, bottom: top + height };
    top += height;
    return box;
  });
}

const marksOf = (boxes: RowBox[], viewport: number) =>
  readingMarks(boxes.length, (index) => boxes[index]!, viewport);

describe("readingMarks", () => {
  it("takes every block in view, top first, then the nearest below and above", () => {
    // Viewport 0..250: `b` straddles its top edge, `e` starts at its bottom one.
    const boxes = stack(-250, ["a", 100], ["b", 200], ["c", 100], ["d", 100], ["e", 100], ["f", 100]);

    expect(marksOf(boxes, 250)).toEqual([
      { id: "b", offset: -150 },
      { id: "c", offset: 50 },
      { id: "d", offset: 150 },
      { id: "e", offset: 250 },
      { id: "a", offset: -250 },
    ]);
  });

  it("never anchors on a separator", () => {
    const boxes = stack(0, ["day-d", 40, false], ["unread-b", 20, false], ["b", 100], ["c", 100]);

    expect(marksOf(boxes, 300).map((mark) => mark.id)).toEqual(["b", "c"]);
  });

  it("skips separators when looking past the viewport too", () => {
    const boxes = stack(-200, ["a", 100], ["day-x", 100, false], ["b", 300], ["day-y", 50, false], ["c", 100]);

    expect(marksOf(boxes, 300)).toEqual([
      { id: "b", offset: 0 },
      { id: "c", offset: 350 },
      { id: "a", offset: -200 },
    ]);
  });

  it("finds the first row in view among many by halving, not by measuring them all", () => {
    const boxes = stack(-100_000, ...Array.from({ length: 2_000 }, (_, index): [string, number] => [`m${index}`, 100]));
    const measured = new Set<number>();

    const marks = readingMarks(
      boxes.length,
      (index) => {
        measured.add(index);
        return boxes[index]!;
      },
      300,
    );

    expect(marks[0]).toEqual({ id: "m1000", offset: 0 });
    expect(measured.size).toBeLessThan(30);
  });

  it("has nothing to offer without rows", () => {
    expect(marksOf([], 300)).toEqual([]);
  });
});

describe("anchoredScrollTop", () => {
  const now = (offsets: Record<string, number>) => (id: string) => offsets[id];

  it("moves by as much as the reader's row moved, which is what was prepended above it", () => {
    const marks = [{ id: "b", offset: 40 }];

    // A page of 2 000 px landed above `b`; the offset has not moved yet.
    expect(anchoredScrollTop(120, marks, now({ b: 2_040 }))).toBe(2_120);
  });

  it("ignores what landed below the reader at the same time", () => {
    const marks = [{ id: "b", offset: 40 }, { id: "c", offset: 140 }];

    // `b` and `c` keep their distance; the live message under them is irrelevant.
    expect(anchoredScrollTop(120, marks, now({ b: 2_040, c: 2_140, d: 2_240 }))).toBe(2_120);
  });

  it("falls back to the next row when the reader's row was merged into a new block", () => {
    const marks = [{ id: "b", offset: 40 }, { id: "c", offset: 140 }];

    expect(anchoredScrollTop(120, marks, now({ ab: 0, c: 2_140 }))).toBe(2_120);
  });

  it("falls back to a row above when nothing below survives", () => {
    const marks = [{ id: "b", offset: 40 }, { id: "a", offset: -60 }];

    expect(anchoredScrollTop(120, marks, now({ a: 1_940 }))).toBe(2_120);
  });

  it("puts a row back where it was, so doing it twice moves nothing the second time", () => {
    const marks = [{ id: "b", offset: 40 }];
    const once = anchoredScrollTop(120, marks, now({ b: 340 }));
    expect(once).toBe(420);

    // After the first correction `b` is back at 40.
    expect(anchoredScrollTop(once!, marks, now({ b: 40 }))).toBe(420);
  });

  it("gives up when none of the rows is still there", () => {
    expect(anchoredScrollTop(120, [{ id: "b", offset: 40 }], now({ x: 0 }))).toBeNull();
  });
});

describe("the end of the scroller", () => {
  it("is reached exactly, give or take a rounded pixel", () => {
    expect(isAtEnd(400, 1_000, 600)).toBe(true);
    expect(isAtEnd(399.2, 1_000, 600)).toBe(true);
    expect(isAtEnd(380, 1_000, 600)).toBe(false);
  });

  it("is where a transcript too short to scroll already is", () => {
    expect(isAtEnd(0, 300, 600)).toBe(true);
    expect(distanceFromEnd(0, 300, 600)).toBe(0);
  });

  it("is measured from where the reader is", () => {
    expect(distanceFromEnd(352, 1_000, 600)).toBe(48);
  });
});

describe("centredScrollTop", () => {
  it("puts the middle of the row in the middle of the viewport", () => {
    // Row at 900..1000 below a viewport of 600 scrolled to 0.
    expect(centredScrollTop(0, 900, 100, 600, 3_000)).toBe(650);
  });

  it("cannot go above the first row or past the last", () => {
    expect(centredScrollTop(500, -400, 100, 600, 3_000)).toBe(0);
    expect(centredScrollTop(2_000, 900, 100, 600, 3_000)).toBe(2_400);
  });
});
