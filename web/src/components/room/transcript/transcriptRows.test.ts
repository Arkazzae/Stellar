import { describe, expect, it } from "vitest";

import type { MessageGroupVM } from "./MessageGroup";
import {
  arrivedAtEnd,
  indexOfMessage,
  messageTail,
  oldestGroupId,
  sharesAMessage,
  toRows,
  type TranscriptSectionVM,
} from "./transcriptRows";

function group(id: string, ...bodyIds: string[]): MessageGroupVM {
  return {
    id,
    authorId: "u1",
    authorName: "Ada",
    timestamp: "2026-08-14T10:00:00Z",
    bodies: (bodyIds.length > 0 ? bodyIds : [id]).map((bodyId) => ({
      id: bodyId,
      parts: [],
    })),
  };
}

const sections: TranscriptSectionVM[] = [
  { day: "Wczoraj", groups: [group("a"), group("b", "b1", "b2")] },
  { day: "Dzisiaj", groups: [group("c")] },
];

describe("toRows", () => {
  it("gives every day and every block a row of its own, in reading order", () => {
    expect(toRows(sections, null).map((row) => `${row.kind}:${row.id}`)).toEqual([
      "day:day-Wczoraj",
      "group:a",
      "group:b",
      "day:day-Dzisiaj",
      "group:c",
    ]);
  });

  it("puts the unread line directly above the first unread block, even across a day", () => {
    expect(toRows(sections, "c").map((row) => row.kind)).toEqual([
      "day",
      "group",
      "group",
      "day",
      "unread",
      "group",
    ]);
  });

  it("adds no unread line when the block is not in the list", () => {
    expect(toRows(sections, "gone").some((row) => row.kind === "unread")).toBe(false);
  });

  it("is empty for an empty transcript", () => {
    expect(toRows([], null)).toEqual([]);
  });
});

describe("messageTail", () => {
  it("counts messages, not blocks, and names the newest one", () => {
    expect(messageTail(toRows(sections, null))).toEqual({
      count: 4,
      lastMessageId: "c",
    });
  });

  it("has no newest message in an empty transcript", () => {
    expect(messageTail([])).toEqual({ count: 0, lastMessageId: undefined });
  });
});

describe("indexOfMessage", () => {
  const rows = toRows(sections, "b");

  it("finds a block by its own id", () => {
    expect(indexOfMessage(rows, "b")).toBe(3);
  });

  it("finds the block that holds a later message of a run", () => {
    expect(indexOfMessage(rows, "b2")).toBe(3);
  });

  it("does not mistake a separator for a message", () => {
    expect(indexOfMessage(rows, "day-Wczoraj")).toBe(-1);
    expect(indexOfMessage(rows, "unread-b")).toBe(-1);
  });

  it("answers -1 for a message older than the loaded history", () => {
    expect(indexOfMessage(rows, "older")).toBe(-1);
  });
});

describe("arrivedAtEnd", () => {
  it("is a message more and a different newest one", () => {
    expect(arrivedAtEnd({ count: 2, lastMessageId: "b" }, { count: 3, lastMessageId: "c" })).toBe(
      true,
    );
  });

  it("is not the server's id replacing the optimistic one", () => {
    expect(
      arrivedAtEnd(
        { count: 3, lastMessageId: "local-3" },
        { count: 3, lastMessageId: "server-3" },
      ),
    ).toBe(false);
  });

  it("is not an older page, which leaves the newest message where it was", () => {
    expect(arrivedAtEnd({ count: 3, lastMessageId: "c" }, { count: 53, lastMessageId: "c" })).toBe(
      false,
    );
  });
});

describe("sharesAMessage", () => {
  const before = toRows([{ day: "Dzisiaj", groups: [group("b", "b1", "b2"), group("c")] }], null);

  it("holds when an older page merged the oldest block into a new one", () => {
    const after = toRows(
      [{ day: "Dzisiaj", groups: [group("a", "a1", "b1", "b2"), group("c")] }],
      null,
    );

    expect(sharesAMessage(before, after)).toBe(true);
  });

  it("does not hold for another conversation", () => {
    const after = toRows([{ day: "Dzisiaj", groups: [group("x"), group("y")] }], null);

    expect(sharesAMessage(before, after)).toBe(false);
  });

  it("does not hold when the first page lands in an empty transcript", () => {
    expect(sharesAMessage([], before)).toBe(false);
  });
});

describe("oldestGroupId", () => {
  it("is the first block, past any separator", () => {
    expect(oldestGroupId(toRows(sections, "a"))).toBe("a");
  });

  it("is nothing in an empty transcript", () => {
    expect(oldestGroupId([])).toBeNull();
  });
});
