import { act, render, screen, waitFor } from "@testing-library/react";
import { Profiler } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import { RailContainer } from "@/containers/RailContainer";
import { RoomContainer } from "@/containers/RoomContainer";
import type * as GiphyModule from "@/lib/giphy/giphy";
import { DEFAULT_SETTINGS } from "@/lib/voice/settings";
import { createChatHarness, page } from "@/test/chatHarness";
import { SELF } from "@/test/fakes";
import { resetStubs, setStubs, stubs } from "@/test/hookStubs";
import { message } from "@/test/messages";
import { renderLog } from "@/test/renderLog";
import { fakeUi, fakeVoice } from "@/test/roomFakes";

import { ChatProvider } from "./ChatProvider";

// The chat provider and the containers read the same stand-ins: the
// provider through its own modules, the containers through `@/state/hooks`,
// whose chat hooks are the real ones.
vi.mock("./PortsProvider", async () => {
  const { stubs } = await import("@/test/hookStubs");
  return { usePorts: () => stubs.ports };
});
vi.mock("./AccountProvider", async () => {
  const { stubs } = await import("@/test/hookStubs");
  return { useAccountContext: () => stubs.account };
});
vi.mock("./SessionProvider", async () => {
  const { stubs } = await import("@/test/hookStubs");
  return { useSessionContext: () => stubs.session };
});
vi.mock("./DirectoryProvider", async () => {
  const { stubs } = await import("@/test/hookStubs");
  return { useDirectoryContext: () => stubs.directory };
});
vi.mock("./EmojiProvider", async () => {
  const { stubs } = await import("@/test/hookStubs");
  return { useEmoji: () => stubs.emoji };
});
vi.mock("./UiProvider", async () => {
  const { stubs } = await import("@/test/hookStubs");
  return { useUiContext: () => stubs.ui };
});
vi.mock("@/state/hooks", async (importOriginal) => {
  const real = await importOriginal<Record<string, unknown>>();
  const { hooksModule } = await import("@/test/hookStubs");
  const chat = Object.entries(real).filter(([name]) => name.startsWith("useChat"));
  return { ...hooksModule, ...Object.fromEntries(chat) };
});
vi.mock("@/containers/NotificationInboxContainer", () => ({
  NotificationInboxButton: () => null,
}));
vi.mock("@/lib/giphy/giphy", async (importOriginal) => ({
  ...(await importOriginal<typeof GiphyModule>()),
  readGiphyConfig: vi.fn(async () => ({ enabled: false, apiKey: "" })),
}));
vi.mock("@/lib/audio/systemFx", () => ({ playSystemFx: vi.fn() }));

const ADA = { id: "u-ada", displayName: "Ada" };

/** Ada's message written just now, so it counts as unread where it is not read. */
const newMessage = (id: string, channelId: string) =>
  message(id, 0, { author: ADA, channelId, createdAt: new Date().toISOString() });

let h: ReturnType<typeof createChatHarness>;

beforeEach(() => {
  resetStubs();
  h = createChatHarness();
  setStubs({
    ports: {
      ...h.ports,
      voiceSettings: { ...stubs.ports.voiceSettings, getSnapshot: () => DEFAULT_SETTINGS },
    },
    session: { user: SELF, hasEnded: () => false },
    account: {
      preferences: {
        ...stubs.account.preferences,
        messageSound: true,
        mentionsOnly: false,
      },
    },
    directory: {
      members: [SELF],
      textChannels: [
        { id: "ch-general", name: "general" },
        { id: "ch-random", name: "random" },
      ],
      channels: [],
      channelName: () => "",
      canManageMessages: false,
    },
    emoji: { emoji: [], byShortcode: new Map() },
    ui: fakeUi({ activeChannelId: "ch-general", activeTextChannelId: "ch-general" }),
    voice: fakeVoice(),
  });
});

/** The server's rail beside the room, under one chat, as the shell lays them out. */
async function mountServerView() {
  h.chat.listMessages.mockResolvedValue(page([message("m1", 1), message("m2", 2)]));
  const log = renderLog();
  const view = render(
    <Profiler id="server-view" onRender={log.commit}>
      <ConfirmProvider>
        <ChatProvider>
          <RailContainer />
          <RoomContainer />
        </ChatProvider>
      </ConfirmProvider>
    </Profiler>,
  );
  await waitFor(() => expect(view.getByText("content of m2")).toBeInTheDocument());
  // The first page, the pins and the thread lists settle in their own tasks.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  log.watch(view.container);
  return log;
}

describe("what a chat event renders", () => {
  it("renders nothing when the server unfurls nothing for a message", async () => {
    const log = await mountServerView();
    act(() => h.emit.message(message("m3", 3, { author: ADA })));
    log.take();

    // Sent after every message, mostly to say that it has no cards.
    act(() => h.emit.embeds({ messageId: "m3", channelId: "ch-general", embeds: [] }));

    expect(log.take().flat()).toEqual([]);
  });

  it("leaves the rail out of a message in the open channel", async () => {
    const log = await mountServerView();

    act(() => h.emit.message(message("m3", 3, { author: ADA })));

    expect(log.take().flat()).not.toContain("RailContainer");
  });

  it("renders a message in another channel on that channel's row and on no other", async () => {
    const log = await mountServerView();

    act(() => h.emit.message(newMessage("r1", "ch-random")));

    const commits = log.take();
    expect(commits).toHaveLength(1);
    expect(commits.flat().filter((name) => name === "ChannelButton")).toHaveLength(1);
    expect(screen.getByLabelText("Nieprzeczytane: 1")).toBeInTheDocument();
  });

  it("renders a typing notice on the typing line alone, and one elsewhere nowhere", async () => {
    const log = await mountServerView();

    act(() => h.emit.typing("ch-general", ["Ada"]));
    expect(log.take()).toEqual([["RoomTyping", "Typing"]]);
    expect(screen.getByText("Ada pisze…")).toBeInTheDocument();

    act(() => h.emit.typing("ch-random", ["Bob"]));
    act(() => h.emit.typing("ch-general", []));
    expect(log.take()).toEqual([["RoomTyping", "Typing"]]);
  });
});
