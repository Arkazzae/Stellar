import { describe, expect, it } from "vitest";

import { serverEvent } from "@/test/events";

import { applySignupChange, mergeEvent } from "./eventStore";

const shared = (extra: Parameters<typeof serverEvent>[0]) => {
  const { mine: _mine, ...copy } = serverEvent(extra);
  return copy;
};

describe("copies of one event", () => {
  it("drops a copy older than the one held", () => {
    const held = serverEvent({ revision: 5, signedUp: 8 });
    expect(mergeEvent(held, serverEvent({ revision: 4, signedUp: 7 }))).toBe(held);
    expect(mergeEvent(held, serverEvent({ revision: 6, signedUp: 9 })).signedUp).toBe(9);
  });

  it("keeps the reader's place through a copy shared with everyone, for the same date", () => {
    const held = mergeEvent(undefined, serverEvent({ revision: 2, mine: "waiting", myPosition: 3 }));
    const next = mergeEvent(held, shared({ revision: 3, signedUp: 10, waiting: 4 }));
    expect(next).toMatchObject({ mine: "waiting", myPosition: 3, waiting: 4 });
  });

  it("starts a new date of a repeating event with nobody signed up, the reader included", () => {
    const held = mergeEvent(undefined, serverEvent({ revision: 2, mine: "signed" }));
    const next = mergeEvent(held, shared({ revision: 3, occurrenceId: "occ-2", signedUp: 0 }));
    expect(next.mine).toBe("none");
  });
});

describe("the reader's own place", () => {
  it("follows the changes about the date held, in their own order", () => {
    const held = mergeEvent(undefined, serverEvent({ revision: 4, mine: "none" }));
    const waiting = applySignupChange(held, { eventId: "ev-1", occurrenceId: "occ-1", revision: 5, mine: "waiting", myPosition: 3 });
    expect(waiting).toMatchObject({ mine: "waiting", myPosition: 3 });

    // Someone ahead left: one place up.
    const moved = applySignupChange(waiting, { eventId: "ev-1", occurrenceId: "occ-1", revision: 7, mine: "waiting", myPosition: 2 });
    expect(moved?.myPosition).toBe(2);
    // A late one from before is dropped.
    expect(applySignupChange(moved, { eventId: "ev-1", occurrenceId: "occ-1", revision: 6, mine: "waiting", myPosition: 3 })).toBe(moved);
    // Promoted: a seat, and no place in the queue.
    const seated = applySignupChange(moved, { eventId: "ev-1", occurrenceId: "occ-1", revision: 8, mine: "signed" });
    expect(seated).toMatchObject({ mine: "signed" });
    expect(seated).not.toHaveProperty("myPosition");
  });

  it("ignores a change about a date the event has moved on from", () => {
    const held = serverEvent({ occurrenceId: "occ-2" });
    expect(applySignupChange(held, { eventId: "ev-1", occurrenceId: "occ-1", revision: 9, mine: "signed" })).toBe(held);
  });
});
