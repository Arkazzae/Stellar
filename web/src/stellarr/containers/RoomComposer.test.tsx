import { act, render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ComposerProps } from "@/components/room/composer/Composer";
import { readGiphyConfig } from "@/lib/giphy/giphy";
import type * as GiphyModule from "@/lib/giphy/giphy";
import { giphyChoice } from "@/lib/giphy/giphyChoice";

import { RoomComposer } from "./RoomComposer";

const seen = vi.hoisted(() => ({ props: null as ComposerProps | null }));

vi.mock("@/components/room/composer/Composer", () => ({
  Composer: (props: ComposerProps) => {
    seen.props = props;
    return null;
  },
}));

vi.mock("@/lib/giphy/giphy", async (importOriginal) => ({
  ...(await importOriginal<typeof GiphyModule>()),
  readGiphyConfig: vi.fn(),
  answeredGiphyConfig: vi.fn(() => undefined),
}));

const sendGif = vi.fn();
const voiceSettings = {};

vi.mock("@/state/hooks", () => ({
  useSession: () => ({ user: { id: "self" } }),
  useChatRoom: () => ({
    channelId: "c1",
    chips: [],
    replyingTo: null,
    send: vi.fn(),
    sendVoice: vi.fn(),
    sendGif,
    attach: vi.fn(),
    removeChip: vi.fn(),
    retryChip: vi.fn(),
    reorderChips: vi.fn(),
    notifyTyping: vi.fn(),
    cancelReply: vi.fn(),
  }),
  useDirectory: () => ({ members: [], server: { id: "server" } }),
  useEmoji: () => ({ emoji: [] }),
  useLanguage: () => ({ locale: "pl", t: (key: string) => key }),
  usePorts: () => ({
    voiceSettings: { subscribe: () => () => undefined, getSnapshot: () => voiceSettings },
  }),
}));

async function renderComposer() {
  render(<RoomComposer channelName="ogólny" threadRootId={null} />);
  await act(async () => {
    await Promise.resolve();
  });
  return seen.props!;
}

beforeEach(() => {
  seen.props = null;
  window.localStorage.clear();
  vi.mocked(readGiphyConfig).mockResolvedValue({ enabled: true, apiKey: "key" });
  giphyChoice.set(true);
  giphyChoice.accountRead();
});

describe("RoomComposer's GIF button", () => {
  it("is there when the instance offers GIFs and the person's switch is on", async () => {
    expect((await renderComposer()).onGif).toBe(sendGif);
  });

  it("is not there when the person turned GIPHY off", async () => {
    giphyChoice.set(false);

    expect((await renderComposer()).onGif).toBeUndefined();
  });

  it("is not there until the account's copy of the switch has come", async () => {
    giphyChoice.awaitAccount();

    expect((await renderComposer()).onGif).toBeUndefined();
    act(() => giphyChoice.accountRead());
    expect(seen.props?.onGif).toBe(sendGif);
  });

  it("is not there when the instance offers no GIFs", async () => {
    vi.mocked(readGiphyConfig).mockResolvedValue({ enabled: false, apiKey: "" });

    expect((await renderComposer()).onGif).toBeUndefined();
  });
});
