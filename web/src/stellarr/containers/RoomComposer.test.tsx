import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import type { ComposerProps, MentionCandidate } from "@/components/room/composer/Composer";
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
const room = vi.hoisted(() => ({
  ui: {
    channelPanel: null as string | null,
    toggleChannelPanel: vi.fn(),
    revealAccountPanel: vi.fn(),
  },
  closeThread: vi.fn(),
  directory: { canModerateMembers: false, canManageBans: false },
  admin: { supported: { moderation: true }, kick: vi.fn(), ban: vi.fn() },
}));
const ADA = { id: "u-ada", username: "ada", displayName: "Ada Lovelace" };

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
    closeThread: room.closeThread,
  }),
  useDirectory: () => ({
    members: [{ id: "self", username: "me", displayName: "Me" }, ADA],
    server: { id: "server" },
    ...room.directory,
  }),
  useUi: () => room.ui,
  useEmoji: () => ({ emoji: [] }),
  useLanguage: () => ({ locale: "pl", t: (key: string) => key }),
  usePorts: () => ({
    voiceSettings: { subscribe: () => () => undefined, getSnapshot: () => voiceSettings },
    admin: room.admin,
  }),
}));

async function renderComposer() {
  render(<RoomComposer channelName="ogólny" threadRootId={null} />, { wrapper: ConfirmProvider });
  await act(async () => {
    await Promise.resolve();
  });
  return seen.props!;
}

beforeEach(() => {
  seen.props = null;
  room.ui.channelPanel = null;
  room.directory = { canModerateMembers: false, canManageBans: false };
  room.admin.supported.moderation = true;
  vi.clearAllMocks();
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

describe("RoomComposer's commands", () => {
  const host = async () => (await renderComposer()).commandHost!;

  it("offers kicking and banning only to someone who may, and never on oneself", async () => {
    expect((await host()).kick).toBeUndefined();
    expect((await host()).ban).toBeUndefined();

    room.directory = { canModerateMembers: true, canManageBans: false };
    const kicker = await host();
    expect(kicker.kick).toBeDefined();
    expect(kicker.ban).toBeUndefined();
    expect(kicker.people?.map((person) => person.id)).toEqual(["u-ada"]);

    // The permission alone is not enough without an endpoint behind it.
    room.directory = { canModerateMembers: true, canManageBans: true };
    room.admin.supported.moderation = false;
    const unsupported = await host();
    expect(unsupported.kick).toBeUndefined();
    expect(unsupported.ban).toBeUndefined();
  });

  it("bans only from the question card, with the reason already in its field", async () => {
    room.directory = { canModerateMembers: true, canManageBans: true };
    room.admin.ban.mockResolvedValue(undefined);
    const user = userEvent.setup();
    const { ban } = await host();

    act(() => ban!(ADA as MentionCandidate, "spam"));

    const card = screen.getByRole("dialog");
    // The member menu's own ban card (this file's translator hands back keys).
    expect(within(card).getByText("confirm.banTitle")).toBeInTheDocument();
    expect(within(card).getByText("Ada Lovelace")).toBeInTheDocument();
    expect(within(card).getByRole("textbox")).toHaveValue("spam");
    expect(room.admin.ban).not.toHaveBeenCalled();

    await user.click(within(card).getByRole("button", { name: "common.ban" }));
    expect(room.admin.ban).toHaveBeenCalledExactlyOnceWith("u-ada", "spam");
  });

  it("kicks only from the question card", async () => {
    room.directory = { canModerateMembers: true, canManageBans: false };
    room.admin.kick.mockResolvedValue(undefined);
    const user = userEvent.setup();
    const { kick } = await host();

    act(() => kick!(ADA as MentionCandidate));

    const card = screen.getByRole("dialog");
    expect(within(card).getByText("confirm.kickTitle")).toBeInTheDocument();
    expect(room.admin.kick).not.toHaveBeenCalled();
    await user.click(within(card).getByRole("button", { name: "common.kick" }));
    expect(room.admin.kick).toHaveBeenCalledExactlyOnceWith("u-ada");
  });

  it("opens the pinned list as the header's button does, and the account panel through the shell", async () => {
    const { openPinned, openStatus } = await host();

    openPinned!();
    expect(room.closeThread).toHaveBeenCalledTimes(1);
    expect(room.ui.toggleChannelPanel).toHaveBeenCalledExactlyOnceWith("pinned");

    // Already showing: the command leaves it open rather than toggling it shut.
    room.ui.channelPanel = "pinned";
    const again = await host();
    again.openPinned!();
    expect(room.ui.toggleChannelPanel).toHaveBeenCalledTimes(1);

    expect(openStatus).toBe(room.ui.revealAccountPanel);
  });
});
