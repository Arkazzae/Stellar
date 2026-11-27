import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { createChatHarness, page } from "@/test/chatHarness";
import type { Poll } from "@/ports";
import { at, message } from "@/test/messages";

import { ChatProvider, useChatContext, type ChatValue } from "./ChatProvider";

const world = vi.hoisted(() => ({
  h: undefined as unknown as ReturnType<typeof createChatHarness>,
}));

vi.mock("./PortsProvider", () => ({ usePorts: () => world.h.ports }));
vi.mock("./AccountProvider", () => ({ useAccountContext: () => world.h.account }));
vi.mock("./SessionProvider", () => ({ useSessionContext: () => world.h.session }));
vi.mock("./DirectoryProvider", () => ({ useDirectoryContext: () => world.h.directory }));
vi.mock("./EmojiProvider", () => ({ useEmoji: () => world.h.emoji }));
vi.mock("./LanguageProvider", () => ({ useLanguage: () => world.h.language }));
vi.mock("./UiProvider", () => ({ useUiContext: () => world.h.ui }));
vi.mock("@/lib/audio/systemFx", () => ({ playSystemFx: vi.fn() }));

let h: typeof world.h;

beforeEach(() => {
  h = createChatHarness();
  world.h = h;
});

function Wrapper({ children }: { children: ReactNode }) {
  return <ChatProvider>{children}</ChatProvider>;
}

const POLL: Poll = {
  question: "Kiedy gramy?",
  answers: [
    { id: 1, text: "Piątek", votes: 1 },
    { id: 2, text: "Sobota", votes: 2 },
  ],
  multiple: false,
  endsAt: "2999-01-01T00:00:00Z",
  voters: 3,
  mine: [2],
};

const pollBody = (chat: ChatValue) =>
  chat.sections.flatMap((section) => section.groups.flatMap((group) => group.bodies)).find((body) => body.id === "p");

describe("a poll in the open channel", () => {
  it("keeps the reader's vote through a copy shared with everyone, takes a vote from their other session, and draws no empty text", async () => {
    h.chat.listMessages.mockResolvedValueOnce(page([message("p", 1, { content: "", poll: POLL })]));
    const { result } = renderHook(() => useChatContext(), { wrapper: Wrapper });
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(pollBody(result.current)?.parts).toEqual([]);

    const { mine: _mine, ...shared } = POLL;
    act(() => h.emit.updated(message("p", 1, { content: "", pinnedAt: at(3), poll: shared })));
    expect(pollBody(result.current)?.poll?.mine).toEqual([2]);

    act(() => h.emit.pollVote({ channelId: "ch-general", messageId: "p", sequence: 2, mine: [1] }));
    expect(pollBody(result.current)?.poll?.mine).toEqual([1]);
  });

  it("merges live counts into the poll on screen", async () => {
    h.chat.listMessages.mockResolvedValueOnce(page([message("p", 1, { content: "", poll: POLL })]));
    const { result } = renderHook(() => useChatContext(), { wrapper: Wrapper });
    await waitFor(() => expect(result.current.loading).toBe(false));

    act(() =>
      h.emit.pollUpdated({
        channelId: "ch-general",
        messageId: "p",
        sequence: 7,
        answers: [
          { id: 1, votes: 1 },
          { id: 2, votes: 4 },
        ],
        voters: 5,
      }),
    );

    expect(pollBody(result.current)?.poll).toMatchObject({ voters: 5, mine: [2] });
    expect(pollBody(result.current)?.poll?.answers.map((answer) => answer.votes)).toEqual([1, 4]);
  });
});
