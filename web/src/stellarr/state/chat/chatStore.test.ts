import { describe, expect, it } from "vitest";

import type { Message, MessageEmbed } from "@/ports";
import { at, message } from "@/test/messages";

import {
  EMPTY_CHAT_DATA,
  chatReducer,
  type ChatAction,
  type ChatData,
} from "./chatStore";

function run(actions: ChatAction[], from: ChatData = EMPTY_CHAT_DATA): ChatData {
  return actions.reduce(chatReducer, from);
}

const ids = (messages: readonly Message[] | undefined) => (messages ?? []).map((m) => m.id);
const reference = (id: string) => ({ id, authorId: "u-ada", authorName: "Ada", preview: id });
const none = new Set<string>();
const embed = (id: string): MessageEmbed => ({
  id,
  url: `https://example.com/${id}`,
  kind: "link",
  provider: "example.com",
  domain: "example.com",
});

describe("first page", () => {
  it("fills the channel, remembers the cursor and marks it loaded", () => {
    const state = run([
      {
        type: "first-page",
        channelId: "ch-general",
        page: { messages: [message("a", 1), message("b", 2)], nextCursor: "older" },
        pendingIds: none,
      keepOlder: true,
      window: null,
      },
    ]);
    expect(ids(state.messages["ch-general"])).toEqual(["a", "b"]);
    expect(state.nextCursor["ch-general"]).toBe("older");
    expect(state.loaded["ch-general"]).toBe(true);
  });

  it("settles as loaded, with nothing older, when the read fails", () => {
    const state = run([{ type: "first-page-failed", channelId: "ch-general" }]);
    expect(state.loaded["ch-general"]).toBe(true);
    expect(state.nextCursor["ch-general"]).toBeNull();
  });

  it("prepends an older page without duplicating the overlap", () => {
    const state = run([
      {
        type: "first-page",
        channelId: "ch-general",
        page: { messages: [message("c", 3)], nextCursor: "p2" },
        pendingIds: none,
      keepOlder: true,
      window: null,
      },
      {
        type: "older-page",
        channelId: "ch-general",
        page: { messages: [message("b", 2), message("c", 3)], nextCursor: null },
      },
    ]);
    expect(ids(state.messages["ch-general"])).toEqual(["b", "c"]);
    expect(state.nextCursor["ch-general"]).toBeNull();
  });
});

describe("realtime", () => {
  const seeded = run([
    {
      type: "first-page",
      channelId: "ch-general",
      page: { messages: [message("a", 1)], nextCursor: null },
      pendingIds: none,
      keepOlder: true,
      window: null,
    },
  ]);

  it("merges a received message and counts a reply on its thread", () => {
    const reply = message("r", 2, { replyTo: reference("a") });
    const state = run([{ type: "received", message: reply }, { type: "received", message: reply }], seeded);
    expect(ids(state.messages["ch-general"])).toEqual(["a", "r"]);
    expect(state.threads["ch-general"]?.[0]?.replyCount).toBe(2);
  });

  it("does not count a thread for a plain message", () => {
    const state = run([{ type: "received", message: message("b", 2) }], seeded);
    expect(state.threads["ch-general"]).toBeUndefined();
  });

  it("shows a message without counting it as a reply", () => {
    const reply = message("r", 2, { replyTo: reference("a") });
    expect(run([{ type: "shown", message: reply }], seeded).threads["ch-general"]).toBeUndefined();
  });

  it("applies an edit only to a message that is loaded", () => {
    const edited = message("a", 1, { content: "edited" });
    const state = run([{ type: "updated", message: edited, locale: "en" }], seeded);
    expect(state.messages["ch-general"]?.[0]?.content).toBe("edited");

    const elsewhere = message("x", 1, { channelId: "ch-other" });
    const untouched = run([{ type: "updated", message: elsewhere, locale: "en" }], seeded);
    expect(untouched).toBe(seeded);
  });

  it("keeps the pinned list and the thread rail in step with an edit", () => {
    const state = run([
      { type: "pins-set", channelId: "ch-general", messages: [] },
      { type: "threads-set", channelId: "ch-general", threads: [{
        channelId: "ch-general", root: reference("a"), replyCount: 1, lastReplyAt: at(5),
      }] },
      {
        type: "updated",
        message: message("a", 1, { content: "new words", pinnedAt: at(9) }),
        locale: "en",
      },
    ], seeded);
    expect(ids(state.pins["ch-general"])).toEqual(["a"]);
    expect(state.threads["ch-general"]?.[0]?.root.preview).toBe("new words");
  });

  it("removes a deleted message from the transcript and from the pins", () => {
    const state = run([
      { type: "pins-set", channelId: "ch-general", messages: [message("a", 1, { pinnedAt: at(2) })] },
      { type: "deleted", channelId: "ch-general", messageId: "a" },
    ], seeded);
    expect(state.messages["ch-general"]).toEqual([]);
    expect(state.pins["ch-general"]).toEqual([]);
  });

  it("returns the same state for a deletion of something never loaded", () => {
    expect(run([{ type: "deleted", channelId: "ch-general", messageId: "nope" }], seeded)).toBe(seeded);
  });

  it("merges unfurled cards into the message", () => {
    const embeds = [embed("e1")];
    const state = run([{ type: "embeds", channelId: "ch-general", messageId: "a", embeds }], seeded);
    expect(state.messages["ch-general"]?.[0]?.embeds).toEqual(embeds);
  });

  it("returns the same state for cards the message already has", () => {
    const carded = run([{ type: "embeds", channelId: "ch-general", messageId: "a", embeds: [embed("e1")] }], seeded);
    expect(run([{ type: "embeds", channelId: "ch-general", messageId: "a", embeds: [] }], seeded)).toBe(seeded);
    expect(
      run([{ type: "embeds", channelId: "ch-general", messageId: "a", embeds: [embed("e1")] }], carded),
    ).toBe(carded);
  });
});

describe("catching up", () => {
  const seeded = run([
    {
      type: "first-page",
      channelId: "ch-general",
      page: { messages: [message("a", 1), message("b", 2)], nextCursor: null },
      pendingIds: none,
      keepOlder: true,
      window: null,
    },
    { type: "pins-set", channelId: "ch-general", messages: [message("b", 2, { pinnedAt: at(3) })] },
  ]);

  it("replays a journal page over messages and pins", () => {
    const state = run([{
      type: "synced",
      channelId: "ch-general",
      changes: [
        { cursor: "1", kind: "delete", messageId: "b" },
        { cursor: "2", kind: "upsert", messageId: "c", message: message("c", 3, { pinnedAt: at(4) }) },
      ],
      cursor: "2",
    }], seeded);
    expect(ids(state.messages["ch-general"])).toEqual(["a", "c"]);
    expect(ids(state.pins["ch-general"])).toEqual(["c"]);
  });

  it("ignores an empty journal page", () => {
    expect(run([{ type: "synced", channelId: "ch-general", changes: [], cursor: "9" }], seeded)).toBe(seeded);
  });

  it("rebuilds the window on a reset but keeps an unsent message", () => {
    const withDraft = run([{ type: "shown", message: message("draft", 9) }], seeded);
    const state = run([{
      type: "reset",
      channelId: "ch-general",
      page: { messages: [message("z", 5)], nextCursor: "n" },
      pendingIds: new Set(["draft"]),
      cursor: "c-new",
    }], withDraft);
    expect(ids(state.messages["ch-general"])).toEqual(["z", "draft"]);
    expect(state.nextCursor["ch-general"]).toBe("n");
  });
});

describe("side lists", () => {
  it("sorts pins newest first and threads by their last reply", () => {
    const state = run([
      { type: "pins-set", channelId: "c", messages: [
        message("a", 1, { channelId: "c", pinnedAt: at(2) }),
        message("b", 1, { channelId: "c", pinnedAt: at(8) }),
      ] },
      { type: "threads-set", channelId: "c", threads: [
        { channelId: "c", root: reference("x"), replyCount: 1, lastReplyAt: at(1) },
        { channelId: "c", root: reference("y"), replyCount: 1, lastReplyAt: at(2) },
      ] },
      { type: "pins-settled", channelId: "c" },
      { type: "threads-settled", channelId: "c" },
    ]);
    expect(ids(state.pins.c)).toEqual(["b", "a"]);
    expect(state.threads.c?.map((t) => t.root.id)).toEqual(["y", "x"]);
    expect(state.pinsLoaded.c).toBe(true);
    expect(state.threadsLoaded.c).toBe(true);
  });

  it("merges a whole thread into the transcript", () => {
    const state = run([
      { type: "thread-opened", channelId: "ch-general", messages: [message("root", 1), message("r", 2)] },
    ]);
    expect(ids(state.messages["ch-general"])).toEqual(["root", "r"]);
  });
});

describe("local changes", () => {
  const seeded = run([{ type: "shown", message: message("a", 1) }]);

  it("swaps an optimistic message for the server's copy", () => {
    const state = run([
      { type: "shown", message: message("local", 5) },
      { type: "settled", channelId: "ch-general", placeholderId: "local", message: message("srv", 6) },
    ], seeded);
    expect(ids(state.messages["ch-general"])).toEqual(["a", "srv"]);
  });

  it("drops a message and rewrites another", () => {
    const state = run([
      { type: "rewritten", channelId: "ch-general", messageId: "a", content: "x", editedAt: at(7) },
    ], seeded);
    expect(state.messages["ch-general"]?.[0]).toMatchObject({ content: "x", editedAt: at(7) });
    expect(run([{ type: "dropped", channelId: "ch-general", messageId: "a" }], state)
      .messages["ch-general"]).toEqual([]);
  });

  it("tracks an older page in flight per channel", () => {
    const state = run([{ type: "older-loading", channelId: "c", loading: true }]);
    expect(state.loadingOlder.c).toBe(true);
    expect(run([{ type: "older-loading", channelId: "c", loading: false }], state).loadingOlder.c).toBe(false);
  });
});

describe("the window the device may keep", () => {
  const open = (extra: Partial<Extract<ChatAction, { type: "first-page" }>> = {}): ChatAction => ({
    type: "first-page",
    channelId: "ch-general",
    page: { messages: [message("a", 5), message("b", 6)], nextCursor: "older" },
    pendingIds: none,
    keepOlder: true,
    window: { cursor: "c1", floor: message("a", 5) },
    ...extra,
  });

  it("moves its cursor with the journal page that was applied, in the same step", () => {
    const state = run([
      open(),
      {
        type: "synced",
        channelId: "ch-general",
        changes: [{ cursor: "2", kind: "upsert", messageId: "c", message: message("c", 7) }],
        cursor: "c2",
      },
    ]);
    expect(ids(state.messages["ch-general"])).toEqual(["a", "b", "c"]);
    expect(state.windows["ch-general"]?.cursor).toBe("c2");
  });

  it("lowers its floor when an older page is read, and not for a page of nothing but the start", () => {
    const state = run([
      open(),
      {
        type: "older-page",
        channelId: "ch-general",
        page: { messages: [message("x", 1), message("y", 2)], nextCursor: "oldest" },
      },
    ]);
    expect(state.windows["ch-general"]?.floor?.id).toBe("x");

    const atStart = run([
      { type: "older-page", channelId: "ch-general", page: { messages: [message("w", 0)], nextCursor: null } },
    ], state);
    expect(atStart.windows["ch-general"]?.floor).toBeNull();
  });

  it("takes the fresh page's cursor and floor on a reset", () => {
    const state = run([
      open(),
      {
        type: "reset",
        channelId: "ch-general",
        page: { messages: [message("z", 9)], nextCursor: "n" },
        pendingIds: none,
        cursor: "c-fresh",
      },
    ]);
    expect(state.windows["ch-general"]).toEqual({ cursor: "c-fresh", floor: message("z", 9) });
  });

  it("is not set by a message that is only shown, such as a search hit", () => {
    const state = run([open(), { type: "shown", message: message("far", 1) }]);
    expect(state.windows["ch-general"]?.floor?.id).toBe("a");
  });
});

describe("a window painted from the device", () => {
  it("shows the stored messages and marks the channel restored, not loaded", () => {
    const state = run([{ type: "stored-window", channelId: "ch-general", messages: [message("a", 1)] }]);
    expect(ids(state.messages["ch-general"])).toEqual(["a"]);
    expect(state.restored["ch-general"]).toBe(true);
    expect(state.loaded["ch-general"]).toBeUndefined();
  });

  it("is ignored once the server has answered", () => {
    const loaded = run([{ type: "first-page-failed", channelId: "ch-general" }]);
    expect(run([{ type: "stored-window", channelId: "ch-general", messages: [message("a", 1)] }], loaded)).toBe(
      loaded,
    );
  });
});

describe("polls", () => {
  const poll = (extra: Partial<NonNullable<Message["poll"]>> = {}): NonNullable<Message["poll"]> => ({
    question: "Kiedy?",
    answers: [
      { id: 1, text: "Piątek", votes: 1 },
      { id: 2, text: "Sobota", votes: 2 },
    ],
    multiple: false,
    endsAt: "2026-11-28T12:00:00Z",
    voters: 3,
    mine: [2],
    ...extra,
  });
  const loaded = (extra: Partial<NonNullable<Message["poll"]>> = {}): ChatAction => ({
    type: "first-page",
    channelId: "ch-general",
    page: { messages: [message("p", 1, { content: "", poll: poll(extra) })], nextCursor: null },
    pendingIds: none,
    keepOlder: true,
    window: null,
  });
  const pollIn = (state: ChatData) => state.messages["ch-general"]?.[0]?.poll;

  it("merges live counts into the poll's message, and drops ones that arrive late", () => {
    const update = (sequence: number, sobota: number) => ({
      type: "poll" as const,
      channelId: "ch-general",
      messageId: "p",
      change: {
        kind: "updated" as const,
        update: { channelId: "ch-general", messageId: "p", sequence, answers: [{ id: 2, votes: sobota }], voters: 1 + sobota },
      },
    });

    const state = run([loaded(), update(5, 4), update(4, 3)]);

    expect(pollIn(state)?.answers.map((answer) => answer.votes)).toEqual([1, 4]);
    expect(pollIn(state)?.mine).toEqual([2]);
  });

  it("keeps the reader's vote when the message comes again in a copy shared with everyone", () => {
    const { mine: _mine, ...shared } = poll();
    const pinnedCopy = message("p", 1, { content: "", pinnedAt: at(3), poll: shared });

    const updated = run([loaded(), { type: "updated", message: pinnedCopy, locale: "pl" }]);
    expect(pollIn(updated)?.mine).toEqual([2]);
    expect(updated.messages["ch-general"]?.[0]?.pinnedAt).toBe(at(3));

    const received = run([loaded(), { type: "received", message: pinnedCopy }]);
    expect(pollIn(received)?.mine).toEqual([2]);
  });
});
