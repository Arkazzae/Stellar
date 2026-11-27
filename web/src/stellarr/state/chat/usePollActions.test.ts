import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { Poll, PollsPort } from "@/ports";
import { message } from "@/test/messages";

import { usePollActions } from "./usePollActions";

const poll = (extra: Partial<Poll> = {}): Poll => ({
  question: "Kiedy?",
  answers: [{ id: 1, text: "Piątek", votes: 0 }],
  multiple: false,
  endsAt: "2026-11-28T12:00:00Z",
  voters: 0,
  mine: [],
  ...extra,
});

function setup() {
  const port: PollsPort = {
    vote: vi.fn(async () => poll({ answers: [{ id: 1, text: "Piątek", votes: 1 }], voters: 1, mine: [1] })),
    end: vi.fn(async () => poll({ endedAt: "2026-11-27T12:00:00Z" })),
    voters: vi.fn(async () => ({ voters: [], nextCursor: null })),
    onPollUpdated: () => () => {},
    onPollVoteChanged: () => () => {},
  };
  const dispatch = vi.fn();
  const messages = { c1: [message("m1", 1, { channelId: "c1", content: "", poll: { ...poll(), sequence: 4 } })] };
  const { result } = renderHook(() =>
    usePollActions({ polls: port, dispatch, channelId: "c1", messages, faces: new Map([["u1", "/u1.png"]]) }),
  );
  return { port, dispatch, actions: result.current };
}

describe("the polls on screen", () => {
  it("votes through the port and merges the answer as of what was known when asked", async () => {
    const { port, dispatch, actions } = setup();

    await actions.vote("m1", [1]);

    expect(port.vote).toHaveBeenCalledExactlyOnceWith("m1", [1]);
    expect(dispatch).toHaveBeenCalledWith({
      type: "poll",
      channelId: "c1",
      messageId: "m1",
      change: {
        kind: "answered",
        poll: expect.objectContaining({ mine: [1], voters: 1 }),
        seen: { sequence: 4, mineSequence: undefined },
      },
    });
  });

  it("ends through the port, lists voters, and knows the members' faces", async () => {
    const { port, dispatch, actions } = setup();

    await actions.end("m1");
    await actions.voters("m1", 1);

    expect(port.end).toHaveBeenCalledWith("m1");
    expect(dispatch).toHaveBeenCalledWith(
      expect.objectContaining({ change: expect.objectContaining({ poll: expect.objectContaining({ endedAt: "2026-11-27T12:00:00Z" }) }) }),
    );
    expect(port.voters).toHaveBeenCalledWith("m1", 1, undefined);
    expect(actions.faceOf?.("u1")).toBe("/u1.png");
  });
});
