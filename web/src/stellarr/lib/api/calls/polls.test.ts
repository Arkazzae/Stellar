import { describe, expect, it } from "vitest";

import { PollDraftRefusedError, PollRefusedError } from "@/ports/polls";
import { jsonResponse, mockFetch, sentRequest } from "@/test/http";

import { parseMessage } from "../chatParsers";
import { ApiError } from "../errors";
import {
  parsePoll,
  parsePollUpdated,
  parsePollVoteChanged,
  pollBody,
  pollCalls,
  pollDraftRefusal,
} from "./polls";

const fetchMock = mockFetch();

/** A poll as the server writes it: every missing value a null. */
const wire = (extra: Record<string, unknown> = {}) => ({
  question: "Kiedy gramy?",
  answers: [
    { id: 1, text: "Piątek", emoji: "🎲", votes: 2 },
    { id: 2, text: "Sobota", emoji: ":dice:", votes: 5 },
    { id: 3, text: "Niedziela", emoji: null, votes: 0 },
  ],
  multiple: false,
  endsAt: "2026-11-28T12:00:00Z",
  endedAt: null,
  endedBy: null,
  voters: 7,
  mine: [2],
  ...extra,
});

const problem = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/problem+json" } });

describe("a poll from the server", () => {
  it("leaves out what is null, and keeps the reader's vote only where it is known", () => {
    const read = parsePoll(wire(), "poll");
    expect(read.answers[2]).toEqual({ id: 3, text: "Niedziela", votes: 0 });
    expect(read.answers[1]?.emoji).toBe(":dice:");
    expect(read).not.toHaveProperty("endedAt");
    expect(read.mine).toEqual([2]);

    expect(parsePoll(wire({ mine: [] }), "poll").mine).toEqual([]);
    // A copy shared with everyone: not this reader's to know.
    expect(parsePoll(wire({ mine: null }), "poll")).not.toHaveProperty("mine");
  });

  it("shows the message without a poll it cannot read, rather than losing the page", () => {
    const message = parseMessage({
      id: "m1",
      channelId: "c1",
      author: { id: "u1", displayName: "Kenji" },
      content: "",
      poll: { question: "?", answers: "nope" },
    });
    expect(message.id).toBe("m1");
    expect(message).not.toHaveProperty("poll");
  });

  it("reads the live counts and the reader's own vote, and passes over ones it cannot read", () => {
    expect(
      parsePollUpdated({
        channelId: "c1",
        messageId: "m1",
        sequence: 4,
        answers: [{ id: 1, votes: 3 }],
        voters: 3,
        endedAt: null,
        endedBy: null,
      }),
    ).toEqual({ channelId: "c1", messageId: "m1", sequence: 4, answers: [{ id: 1, votes: 3 }], voters: 3 });
    expect(parsePollUpdated({ channelId: "c1", messageId: "m1", answers: [] })).toBeNull();

    expect(parsePollVoteChanged({ channelId: "c1", messageId: "m1", sequence: 2, mine: [1, 3] })).toEqual({
      channelId: "c1",
      messageId: "m1",
      sequence: 2,
      mine: [1, 3],
    });
    expect(parsePollVoteChanged({ channelId: "c1", messageId: "m1", sequence: 2 })).toBeNull();
  });
});

describe("sending a poll", () => {
  it("sends the trimmed words, and no emoji as null", () => {
    expect(
      pollBody({
        question: "  Kiedy gramy?  ",
        answers: [{ text: " Piątek ", emoji: "🎲" }, { text: "Sobota" }],
        multiple: true,
        duration: "3d",
      }),
    ).toEqual({
      question: "Kiedy gramy?",
      answers: [
        { text: "Piątek", emoji: "🎲" },
        { text: "Sobota", emoji: null },
      ],
      multiple: true,
      duration: "3d",
    });
  });

  it("names the fields the server refused, answers by the place they were sent in", () => {
    const refused = pollDraftRefusal(
      new ApiError("invalid", "x", 400, {
        fields: { "poll.question": "Too long.", "poll.answers[1]": "Bad emoji.", attachmentIds: "No files." },
      }),
    );
    expect(refused).toBeInstanceOf(PollDraftRefusedError);
    expect(refused?.fields).toEqual(["question", { answer: 1 }, "attachments"]);

    expect(pollDraftRefusal(new ApiError("forbidden", "x", 403))).toBeNull();
  });
});

describe("voting, ending and the voters", () => {
  it("sets the reader's whole vote and answers with the poll", async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse(wire({ mine: [1, 2] })));

    const answered = await pollCalls().votePoll("m1", [1, 2]);

    expect(sentRequest(fetchMock)).toMatchObject({ method: "PUT", url: "/api/messages/m1/poll/vote", body: { answers: [1, 2] } });
    expect(answered.mine).toEqual([1, 2]);
  });

  it("says why a vote was refused: the poll ended, or the choice was not one", async () => {
    fetchMock.mockResolvedValueOnce(problem(409, { code: "poll-ended" }));
    await expect(pollCalls().votePoll("m1", [1])).rejects.toMatchObject({ reason: "ended" });

    fetchMock.mockResolvedValueOnce(problem(400, { errors: { answers: ["One answer only."] } }));
    const refusal = pollCalls().votePoll("m1", [1, 2]);
    await expect(refusal).rejects.toBeInstanceOf(PollRefusedError);
    await expect(refusal).rejects.toMatchObject({ reason: "selection" });
  });

  it("ends the poll, and a reader who may not is told so", async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse(wire({ endedAt: "2026-11-27T12:00:00Z", endedBy: "u1" })));
    expect(await pollCalls().endPoll("m1")).toMatchObject({ endedBy: "u1" });
    expect(sentRequest(fetchMock)).toMatchObject({ method: "POST", url: "/api/messages/m1/poll/end" });

    fetchMock.mockResolvedValueOnce(problem(403, { title: "Forbidden" }));
    await expect(pollCalls().endPoll("m1")).rejects.toMatchObject({ reason: "forbidden" });
  });

  it("asks for a page of who chose an answer", async () => {
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ voters: [{ id: "u1", displayName: "Kenji" }], nextCursor: "next" }),
    );

    const page = await pollCalls().pollVoters("m1", 2, "after");

    expect(sentRequest(fetchMock).url).toBe("/api/messages/m1/poll/answers/2/voters?limit=20&cursor=after");
    expect(page).toEqual({ voters: [{ id: "u1", displayName: "Kenji" }], nextCursor: "next" });
  });
});
