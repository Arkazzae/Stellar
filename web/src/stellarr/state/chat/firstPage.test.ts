import { describe, expect, it } from "vitest";

import { message } from "@/test/messages";

import { chatReducer, EMPTY_CHAT_DATA, type ChatAction, type ChatData } from "./chatStore";
import { settleFirstPage } from "./firstPage";
import type { RestoredWindows, SyncCursors } from "./shared";

const ids = (state: ChatData) => (state.messages["ch-general"] ?? []).map((m) => m.id);

/** What the device held: messages 1 to 4, current as of its own cursor. */
function held(restoredIds: string[] | null) {
  const restored: RestoredWindows = {
    current:
      restoredIds === null
        ? {}
        : {
            "ch-general": {
              ids: new Set(restoredIds),
              cursor: "stored-cursor",
              floor: message(restoredIds[0] ?? "", 1),
            },
          },
  };
  const syncCursors: SyncCursors = { current: {} };
  return { restored, syncCursors, pendingIds: { current: new Set<string>() } };
}

function paint(messages: ReturnType<typeof message>[]): ChatData {
  return chatReducer(EMPTY_CHAT_DATA, { type: "stored-window", channelId: "ch-general", messages });
}

function settle(
  state: ChatData,
  env: ReturnType<typeof held>,
  messages: ReturnType<typeof message>[],
  nextCursor: string | null = "older",
): { state: ChatData; action: ChatAction } {
  const action = settleFirstPage(env, "ch-general", { messages, nextCursor, syncCursor: "fresh-cursor" });
  return { state: chatReducer(state, action), action };
}

const stored = [message("a", 1), message("b", 2), message("c", 3), message("d", 4)];

describe("the first page over a window from the device", () => {
  it("applies an edit, drops a deletion and drains the journal from the stored cursor", () => {
    const env = held(["a", "b", "c", "d"]);
    // The page overlaps: c was edited, d deleted, e and f are new.
    const page = [message("c", 3, { content: "edited" }), message("e", 5), message("f", 6)];

    const { state } = settle(paint(stored), env, page);

    expect(ids(state)).toEqual(["a", "b", "c", "e", "f"]);
    expect(state.messages["ch-general"]?.[2]?.content).toBe("edited");
    expect(env.syncCursors.current["ch-general"]).toBe("stored-cursor");
    expect(state.windows["ch-general"]).toEqual({ cursor: "stored-cursor", floor: stored[0] });
  });

  it("drops the stored messages when the page shares none with them, so no gap is hidden", () => {
    const env = held(["a", "b", "c", "d"]);
    const page = [message("x", 50), message("y", 51)];

    const { state } = settle(paint(stored), env, page);

    expect(ids(state)).toEqual(["x", "y"]);
    expect(env.syncCursors.current["ch-general"]).toBe("fresh-cursor");
    expect(state.windows["ch-general"]?.cursor).toBe("fresh-cursor");
  });

  it("keeps nothing older than a page that reaches the start of the channel", () => {
    const env = held(["a", "b", "c", "d"]);
    const page = [message("c", 3), message("d", 4)];

    const { state } = settle(paint(stored), env, page, null);

    expect(ids(state)).toEqual(["c", "d"]);
    expect(state.windows["ch-general"]).toEqual({ cursor: "fresh-cursor", floor: null });
  });

  it("does not move a cursor that an earlier page already set", () => {
    const env = held(["a", "b", "c", "d"]);
    env.syncCursors.current["ch-general"] = "earlier";

    const { action } = settle(paint(stored), env, [message("d", 4)]);

    expect(env.syncCursors.current["ch-general"]).toBe("earlier");
    expect(action).toMatchObject({ window: null, keepOlder: true });
  });

  it("is the plain case when nothing was restored", () => {
    const env = held(null);

    const { state } = settle(EMPTY_CHAT_DATA, env, [message("e", 5)]);

    expect(ids(state)).toEqual(["e"]);
    expect(env.syncCursors.current["ch-general"]).toBe("fresh-cursor");
  });
});
