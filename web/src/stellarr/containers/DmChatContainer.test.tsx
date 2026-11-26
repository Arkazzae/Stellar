import { render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { DmChatProps } from "@/components/friends/DmChat";
import type { FriendsValue, LanguageValue } from "@/state/hooks";
import { fakeLanguage } from "@/test/roomFakes";

import { DmChatContainer } from "./DmChatContainer";

const mocks = vi.hoisted(() => ({
  friends: null as unknown as FriendsValue,
  language: null as unknown as LanguageValue,
  profile: { toggle: vi.fn() },
  seen: null as DmChatProps | null,
}));

vi.mock("@/state/hooks", () => ({
  useSession: () => ({ user: { id: "self" } }),
  useFriends: () => mocks.friends,
  useLanguage: () => mocks.language,
  useProfile: () => mocks.profile,
  useEmoji: () => ({ emoji: [] }),
}));

vi.mock("./useDmEncryption", () => ({ useDmEncryptionView: () => ({}) }));

vi.mock("@/components/friends/DmChat", () => ({
  DmChat: (props: DmChatProps) => {
    mocks.seen = props;
    return null;
  },
}));

const ADA = { id: "u-ada", username: "ada", displayName: "Ada Lovelace", presence: "online" } as const;

function fakeFriends(): FriendsValue {
  return {
    active: { id: "dm-ada", peer: ADA },
    sections: [],
    hasOlderMessages: false,
    loadingOlderMessages: false,
    chips: [],
    send: vi.fn(async () => {}),
    loadOlderMessages: vi.fn(),
    attach: vi.fn(),
    retryChip: vi.fn(),
    removeChip: vi.fn(),
    reorderChips: vi.fn(),
  } as unknown as FriendsValue;
}

const chat = () => mocks.seen!;

beforeEach(() => {
  mocks.friends = fakeFriends();
  mocks.language = fakeLanguage();
  mocks.profile = { toggle: vi.fn() };
  mocks.seen = null;
});

describe("DmChatContainer", () => {
  it("sends, pages and attaches through the friends state", () => {
    render(<DmChatContainer />);

    expect(chat()).toMatchObject({
      username: "ada",
      onSend: mocks.friends.send,
      onLoadOlderMessages: mocks.friends.loadOlderMessages,
      onAttach: mocks.friends.attach,
      onRemoveChip: mocks.friends.removeChip,
      onRetryChip: mocks.friends.retryChip,
      onReorderChips: mocks.friends.reorderChips,
      onOpenProfile: mocks.profile.toggle,
    });
  });

  it("opens the conversation's pictures and clips in the media viewer", () => {
    render(<DmChatContainer />);

    expect(typeof chat().attachments?.onOpenImage).toBe("function");
    expect(typeof chat().attachments?.onPlayVideo).toBe("function");
  });

  it("passes the translation settings on", () => {
    mocks.language = fakeLanguage({
      chatTranslationEnabled: true,
      chatTranslationTargetLanguage: "pl",
    });

    render(<DmChatContainer />);

    expect(chat().translation).toMatchObject({ enabled: true, targetLanguage: "pl" });
  });
});
