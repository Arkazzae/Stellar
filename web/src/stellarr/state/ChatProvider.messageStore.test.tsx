import "fake-indexeddb/auto";

import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";

import { createMessageStore, wipeAll } from "@/lib/messageStore/messageStore";
import { createChatHarness, deferred, journal, page } from "@/test/chatHarness";
import { SELF } from "@/test/fakes";
import { message } from "@/test/messages";
import type { MessageHistoryPage } from "@/lib/api/types";

import { ChatProvider, useChatContext, type ChatValue } from "./ChatProvider";

const world = vi.hoisted(() => ({
  h: undefined as unknown as ReturnType<typeof createChatHarness>,
}));

vi.mock("./PortsProvider", () => ({ usePorts: () => world.h.ports }));
vi.mock("./AccountProvider", () => ({ useAccountContext: () => world.h.account }));
vi.mock("./SessionProvider", () => ({ useSessionContext: () => world.h.session }));
vi.mock("./DirectoryProvider", () => ({
  useDirectoryContext: () => world.h.directory,
}));
vi.mock("./EmojiProvider", () => ({ useEmoji: () => world.h.emoji }));
vi.mock("./LanguageProvider", () => ({ useLanguage: () => world.h.language }));
vi.mock("./UiProvider", () => ({ useUiContext: () => world.h.ui }));
vi.mock("@/lib/audio/systemFx", () => ({ playSystemFx: vi.fn() }));

let h: typeof world.h;

beforeEach(async () => {
  await wipeAll();
  h = createChatHarness();
  world.h = h;
});
afterEach(() => {
  vi.unstubAllGlobals();
});

function Wrapper({ children }: { children: ReactNode }) {
  return <ChatProvider>{children}</ChatProvider>;
}

const mount = () => renderHook(() => useChatContext(), { wrapper: Wrapper });

const ids = (chat: ChatValue) =>
  chat.sections.flatMap((section) => section.groups.flatMap((group) => group.bodies.map((body) => body.id)));

/** What a device that read the channel earlier left behind. */
async function leaveOnDevice(messages: ReturnType<typeof message>[], cursor = "stored-cursor") {
  const store = createMessageStore({ userId: SELF.id, serverId: "srv" });
  await store.keepOnly(["ch-general"]);
  await store.write("ch-general", messages, cursor, new Set());
}

async function onDevice() {
  const store = createMessageStore({ userId: SELF.id, serverId: "srv" });
  await store.keepOnly(["ch-general"]);
  return store.read("ch-general");
}

const stored = () => [message("a", 1), message("b", 2), message("c", 3), message("d", 4)];

describe("opening a channel with messages on the device", () => {
  it("shows the stored transcript before the server answers, then applies what changed", async () => {
    await leaveOnDevice(stored());
    const answer = deferred<MessageHistoryPage>();
    h.chat.listMessages.mockReturnValue(answer.promise);

    const mounted = mount();
    await waitFor(() => expect(ids(mounted.result.current)).toEqual(["a", "b", "c", "d"]));
    expect(mounted.result.current.loading).toBe(false);

    // Meanwhile c was edited and d deleted, and e arrived.
    await act(async () => {
      answer.resolve(page([message("c", 3, { content: "edited" }), message("e", 5)], "older", "fresh"));
    });
    await waitFor(() => expect(ids(mounted.result.current)).toEqual(["a", "b", "c", "e"]));
  });

  it("drains the journal from the stored cursor and shows an edit made below the page", async () => {
    await leaveOnDevice(stored());
    const answer = deferred<MessageHistoryPage>();
    h.chat.listMessages.mockReturnValue(answer.promise);
    h.chat.syncMessages.mockResolvedValue(
      journal(
        [{ cursor: "9", kind: "upsert", messageId: "a", message: message("a", 1, { content: "old one, edited" }) }],
        "9",
      ),
    );

    const mounted = mount();
    await waitFor(() => expect(mounted.result.current.loading).toBe(false));
    act(() => h.setConnection("connected"));
    await act(async () => {
      answer.resolve(page([message("c", 3), message("e", 5)], "older", "fresh"));
    });

    await waitFor(() => expect(h.chat.syncMessages).toHaveBeenCalledWith("ch-general", "stored-cursor"));
    await waitFor(() =>
      expect(JSON.stringify(mounted.result.current.sections)).toContain("old one, edited"));
  });

  it("drops the stored messages when the channel has moved on by more than a page", async () => {
    await leaveOnDevice(stored());
    const answer = deferred<MessageHistoryPage>();
    h.chat.listMessages.mockReturnValue(answer.promise);

    const mounted = mount();
    await waitFor(() => expect(ids(mounted.result.current)).toHaveLength(4));
    await act(async () => {
      answer.resolve(page([message("x", 50), message("y", 51)], "older", "fresh"));
    });

    await waitFor(() => expect(ids(mounted.result.current)).toEqual(["x", "y"]));
    await act(() => {
      h.setConnection("connected");
    });
    await waitFor(() => expect(h.chat.syncMessages).toHaveBeenCalledWith("ch-general", "fresh"));
  });

  it("replaces the stored window when the journal asks for a reset", async () => {
    await leaveOnDevice(stored());
    const answer = deferred<MessageHistoryPage>();
    h.chat.listMessages.mockReturnValue(answer.promise);
    h.chat.syncMessages.mockResolvedValueOnce(journal([], "x", { resetRequired: true }));

    const mounted = mount();
    await waitFor(() => expect(mounted.result.current.loading).toBe(false));
    act(() => h.setConnection("connected"));
    await act(async () => {
      answer.resolve(page([message("c", 3)], "older", "fresh"));
    });

    await waitFor(() => expect(ids(mounted.result.current)).toEqual(["c"]));
    await waitFor(async () => expect((await onDevice())?.messages.map((m) => m.id)).toEqual(["c"]), {
      timeout: 6000,
    });
  });
});

describe("writing to the device", () => {
  it("writes the messages together with the cursor once the changes settle", async () => {
    h.chat.listMessages.mockResolvedValue(page([message("a", 1), message("b", 2)], null, "cursor-5"));

    const mounted = mount();
    await waitFor(() => expect(mounted.result.current.loading).toBe(false));

    await waitFor(async () => {
      const window = await onDevice();
      expect(window?.messages.map((m) => m.id)).toEqual(["a", "b"]);
      expect(window?.cursor).toBe("cursor-5");
    }, { timeout: 6000 });
  });

  it("does not write a message the server has not confirmed", async () => {
    h.chat.listMessages.mockResolvedValue(page([message("a", 1)], null, "cursor-5"));
    const sent = deferred<ReturnType<typeof message>>();
    h.chat.send.mockReturnValue(sent.promise);

    const mounted = mount();
    await waitFor(() => expect(mounted.result.current.loading).toBe(false));
    act(() => void mounted.result.current.send("hello"));
    await waitFor(() => expect(ids(mounted.result.current)).toHaveLength(2));

    await waitFor(async () => expect((await onDevice())?.messages.map((m) => m.id)).toEqual(["a"]), {
      timeout: 6000,
    });
  });
});

describe("a device that cannot keep messages", () => {
  it("leaves the channel loading as it always did", async () => {
    vi.stubGlobal("indexedDB", {
      open: () => {
        throw new Error("no storage in a private window");
      },
    });
    h.chat.listMessages.mockResolvedValue(page([message("a", 1)], null, "cursor-5"));

    const mounted = mount();
    expect(mounted.result.current.loading).toBe(true);
    await waitFor(() => expect(ids(mounted.result.current)).toEqual(["a"]));
    expect(mounted.result.current.loading).toBe(false);
  });
});

describe("channels that can no longer be read", () => {
  it("loses its stored window when the session starts", async () => {
    const store = createMessageStore({ userId: SELF.id, serverId: "srv" });
    await store.keepOnly(["ch-general", "ch-gone"]);
    await store.write("ch-gone", [message("a", 1, { channelId: "ch-gone" })], "c", new Set());

    mount();

    await waitFor(async () => {
      await store.keepOnly(["ch-general", "ch-gone"]);
      expect(await store.read("ch-gone")).toBeUndefined();
    });
  });
});
