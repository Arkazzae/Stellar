import { describe, expect, it } from "vitest";

import type { Poll } from "@/ports/polls";
import { message } from "@/test/messages";

import {
  applyPollAnswer,
  applyPollUpdate,
  applyVoteChange,
  isPollClosed,
  keepLoadedPoll,
  leadingAnswers,
  pickAnswer,
  seenOf,
  shareOf,
} from "./pollState";

const NOW = new Date("2026-11-27T12:00:00Z");

const poll = (extra: Partial<Poll> = {}): Poll => ({
  question: "Kiedy gramy?",
  answers: [
    { id: 1, text: "Piątek", votes: 2 },
    { id: 2, text: "Sobota", votes: 5 },
    { id: 3, text: "Niedziela", votes: 0 },
  ],
  multiple: false,
  endsAt: "2026-11-28T12:00:00Z",
  voters: 7,
  mine: [],
  ...extra,
});

const update = (sequence: number, votes: [number, number, number], extra = {}) => ({
  channelId: "c1",
  messageId: "m1",
  sequence,
  answers: votes.map((count, index) => ({ id: index + 1, votes: count })),
  voters: votes.reduce((sum, count) => sum + count, 0),
  ...extra,
});

const votesOf = (value: Poll) => value.answers.map((answer) => answer.votes);

describe("picking answers", () => {
  it("swaps the one answer of a single-answer poll, and toggles any answer of a multi-answer one", () => {
    expect(pickAnswer([], 2, false)).toEqual([2]);
    expect(pickAnswer([2], 3, false)).toEqual([3]);
    expect(pickAnswer([2], 2, false)).toEqual([2]);

    expect(pickAnswer([], 3, true)).toEqual([3]);
    expect(pickAnswer([3], 1, true)).toEqual([1, 3]);
    expect(pickAnswer([1, 3], 3, true)).toEqual([1]);
  });
});

describe("a poll's end and results", () => {
  it("is over once it was ended or its time ran out, whichever came first", () => {
    expect(isPollClosed(poll(), NOW)).toBe(false);
    expect(isPollClosed(poll({ endedAt: "2026-11-27T11:00:00Z" }), NOW)).toBe(true);
    // Before the server has said so.
    expect(isPollClosed(poll({ endsAt: "2026-11-27T12:00:00Z" }), NOW)).toBe(true);
  });

  it("counts shares of the people who voted, so several answers can add up past a hundred", () => {
    expect(shareOf(9, 14)).toBe(64);
    expect(shareOf(6, 14)).toBe(43);
    expect(shareOf(0, 0)).toBe(0);
  });

  it("leads with the most votes, keeps every answer of a tie, and has no leader before a vote", () => {
    expect([...leadingAnswers(poll())]).toEqual([2]);
    expect([...leadingAnswers(poll({ answers: [{ id: 1, text: "a", votes: 3 }, { id: 2, text: "b", votes: 3 }] }))]).toEqual([1, 2]);
    expect(leadingAnswers(poll({ answers: [{ id: 1, text: "a", votes: 0 }], voters: 0 })).size).toBe(0);
  });
});

describe("changes from the server", () => {
  it("takes counts in order and drops one that arrives after a newer one", () => {
    const current = applyPollUpdate(poll(), update(4, [3, 5, 0]));
    expect(votesOf(current)).toEqual([3, 5, 0]);

    const late = applyPollUpdate(current, update(3, [2, 5, 0]));
    expect(votesOf(late)).toEqual([3, 5, 0]);

    const ended = applyPollUpdate(current, update(5, [3, 6, 0], { endedAt: "2026-11-27T12:00:00Z", endedBy: "u-ada" }));
    expect(ended).toMatchObject({ voters: 9, endedAt: "2026-11-27T12:00:00Z", endedBy: "u-ada" });
  });

  it("orders the reader's own vote apart from the counts, and never takes it from the counts", () => {
    const counted = applyPollUpdate(poll({ mine: [1] }), update(9, [3, 5, 0]));
    expect(counted.mine).toEqual([1]);

    // A vote from the reader's other session, older in the counts' order but newest among votes.
    const voted = applyVoteChange(counted, { channelId: "c1", messageId: "m1", sequence: 8, mine: [2] });
    expect(voted.mine).toEqual([2]);
    expect(applyVoteChange(voted, { channelId: "c1", messageId: "m1", sequence: 7, mine: [1] }).mine).toEqual([2]);
  });

  it("takes a vote's answer only where nothing newer arrived while it was asked for", () => {
    const before = applyPollUpdate(poll(), update(4, [2, 5, 0]));
    const seen = seenOf(before);
    const answer = poll({ answers: before.answers.map((a) => (a.id === 3 ? { ...a, votes: 1 } : a)), voters: 8, mine: [3] });

    const quiet = applyPollAnswer(before, answer, seen);
    expect(votesOf(quiet)).toEqual([2, 5, 1]);
    expect(quiet.mine).toEqual([3]);

    // Someone else's vote was heard meanwhile: its counts are newer than the answer's.
    const busy = applyPollAnswer(applyPollUpdate(before, update(5, [3, 5, 1])), answer, seen);
    expect(votesOf(busy)).toEqual([3, 5, 1]);
    expect(busy.mine).toEqual([3]);
  });

  it("never reopens a poll an answer saw open", () => {
    const ended = poll({ endedAt: "2026-11-27T11:00:00Z" });
    expect(applyPollAnswer(ended, poll(), seenOf(ended)).endedAt).toBe("2026-11-27T11:00:00Z");
  });
});

describe("a message pushed to everyone", () => {
  it("keeps the reader's vote and the order of changes known here", () => {
    const loaded = message("m1", 1, { poll: { ...poll({ mine: [2] }), sequence: 6, mineSequence: 2 } });
    const { mine: _mine, ...shared } = poll();
    const pushed = keepLoadedPoll(message("m1", 1, { content: "", pinnedAt: "2026-11-27T12:00:00Z", poll: shared }), loaded);

    expect(pushed.poll).toMatchObject({ mine: [2], sequence: 6, mineSequence: 2 });
    expect(pushed.pinnedAt).toBe("2026-11-27T12:00:00Z");
  });

  it("takes a vote a read for this reader brings, even an empty one", () => {
    const loaded = message("m1", 1, { poll: poll({ mine: [2] }) });
    expect(keepLoadedPoll(message("m1", 1, { poll: poll({ mine: [] }) }), loaded).poll?.mine).toEqual([]);
  });
});
