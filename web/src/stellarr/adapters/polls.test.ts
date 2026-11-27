import { describe, expect, it, vi } from "vitest";

import { ApiError } from "@/lib/api/errors";
import type { Message } from "@/ports";
import { PollDraftRefusedError } from "@/ports";
import { FakeRealtimeClient, fakeApi } from "@/test/fakes";
import { message } from "@/test/messages";

import { createChatAdapter } from "./chat";
import { createPollsAdapter } from "./polls";

describe("sending a poll", () => {
  it("goes out as a message of its own, with the poll's words trimmed and no files", async () => {
    const sendMessage = vi.fn(async (): Promise<Message> => message("m1", 1, { content: "" }));
    const chat = createChatAdapter(fakeApi({ sendMessage }), new FakeRealtimeClient());

    await chat.send({
      channelId: "c1",
      content: "",
      attachmentIds: [],
      clientMessageId: "client-1",
      replyToMessageId: "root",
      poll: { question: " Kiedy? ", answers: [{ text: "Piątek", emoji: "🎲" }, { text: "Sobota" }], multiple: false, duration: "1d" },
    });

    expect(sendMessage).toHaveBeenCalledWith("c1", {
      content: "",
      attachmentIds: [],
      clientMessageId: "client-1",
      replyToMessageId: "root",
      poll: {
        question: "Kiedy?",
        answers: [
          { text: "Piątek", emoji: "🎲" },
          { text: "Sobota", emoji: null },
        ],
        multiple: false,
        duration: "1d",
      },
    });
  });

  it("names the poll's fields the server refused, so the sheet can show them", async () => {
    const sendMessage = vi.fn(async (): Promise<Message> => {
      throw new ApiError("invalid", "x", 400, { fields: { "poll.answers": "Two at least." } });
    });
    const chat = createChatAdapter(fakeApi({ sendMessage }), new FakeRealtimeClient());

    const sent = chat.send({
      channelId: "c1",
      content: "",
      attachmentIds: [],
      clientMessageId: "client-1",
      poll: { question: "Kiedy?", answers: [{ text: "Piątek" }], multiple: false, duration: "1d" },
    });

    await expect(sent).rejects.toBeInstanceOf(PollDraftRefusedError);
    await expect(sent).rejects.toMatchObject({ fields: ["answers"] });
  });
});

describe("the polls port", () => {
  it("votes, ends and lists voters over the poll routes", async () => {
    const api = {
      votePoll: vi.fn(async () => ({ question: "?", answers: [], multiple: false, endsAt: "x", voters: 1, mine: [1] })),
      endPoll: vi.fn(async () => ({ question: "?", answers: [], multiple: false, endsAt: "x", voters: 1 })),
      pollVoters: vi.fn(async () => ({ voters: [], nextCursor: null })),
    };
    const port = createPollsAdapter(api, new FakeRealtimeClient());

    await port.vote("m1", [1]);
    await port.end("m1");
    await port.voters("m1", 2, "after");

    expect(api.votePoll).toHaveBeenCalledWith("m1", [1]);
    expect(api.endPoll).toHaveBeenCalledWith("m1");
    expect(api.pollVoters).toHaveBeenCalledWith("m1", 2, "after");
  });

  it("hands on the live counts and the reader's own vote", () => {
    const realtime = new FakeRealtimeClient();
    const port = createPollsAdapter(
      { votePoll: vi.fn(), endPoll: vi.fn(), pollVoters: vi.fn() },
      realtime,
    );
    const counts = vi.fn();
    const votes = vi.fn();
    port.onPollUpdated(counts);
    port.onPollVoteChanged(votes);

    const update = { channelId: "c1", messageId: "m1", sequence: 3, answers: [{ id: 1, votes: 2 }], voters: 2 };
    const vote = { channelId: "c1", messageId: "m1", sequence: 1, mine: [1] };
    realtime.push("PollUpdated", update);
    realtime.push("PollVoteChanged", vote);

    expect(counts).toHaveBeenCalledWith(update);
    expect(votes).toHaveBeenCalledWith(vote);
  });
});
