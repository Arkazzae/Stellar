import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { DmRoomProps } from "@/components/friends/DmRoom";
import type { FriendsViewProps } from "@/components/friends/FriendsView";
import { formatDate } from "@/lib/format/format";
import type {
  FriendsValue,
  LanguageValue,
  UiValue,
  VoiceValue,
} from "@/state/hooks";
import { fakeLanguage, fakeUi, fakeVoice } from "@/test/roomFakes";

import { FriendsRoomContainer } from "./FriendsRoomContainer";

const mocks = vi.hoisted(() => ({
  friends: null as unknown as FriendsValue,
  ui: null as unknown as UiValue,
  voice: null as unknown as VoiceValue,
  language: null as unknown as LanguageValue,
  profile: { toggle: vi.fn() },
  seen: {
    room: null as DmRoomProps | null,
    view: null as FriendsViewProps | null,
  },
}));

vi.mock("@/state/hooks", () => ({
  useFriends: () => mocks.friends,
  useUi: () => mocks.ui,
  useVoice: () => mocks.voice,
  useActivities: () => ({ personal: null }),
  useActivitySlot: () => () => undefined,
  useLanguage: () => mocks.language,
  useProfile: () => mocks.profile,
  useEncryption: () => ({ status: { support: "supported", deviceLimit: false } }),
}));

vi.mock("./NotificationInboxContainer", () => ({
  NotificationInboxButton: () => <button type="button">inbox</button>,
}));

vi.mock("./DmChatContainer", () => ({
  DmChatContainer: () => <div data-testid="dm-chat" />,
}));

vi.mock("./DmCallContainer", () => ({
  DmCallContainer: () => <div data-testid="dm-call" />,
}));

vi.mock("@/components/friends/DmRoom", () => ({
  DmRoom: (props: DmRoomProps) => {
    mocks.seen.room = props;
    return (
      <div data-testid="dm-room">
        {props.notificationButton}
        {props.children}
      </div>
    );
  },
}));

vi.mock("@/components/friends/FriendsView", () => ({
  FriendsView: (props: FriendsViewProps) => {
    mocks.seen.view = props;
    return <div data-testid="friends-view">{props.notificationButton}</div>;
  },
}));

const ADA = { id: "u-ada", username: "ada", displayName: "Ada Lovelace", presence: "online" } as const;
const BOB = { id: "u-bob", username: "bob", displayName: "Bob", presence: "offline" } as const;
const EVE = { id: "u-eve", username: "eve", displayName: "Eve", presence: "away", avatarUrl: "/eve.png" } as const;

function fakeFriends(overrides: Record<string, unknown> = {}): FriendsValue {
  return {
    friends: [],
    requests: [],
    blocked: [],
    active: null,
    sections: [],
    hasOlderMessages: false,
    loadingOlderMessages: false,
    chips: [],
    messagePerson: vi.fn(),
    callPerson: vi.fn(),
    callConversation: vi.fn(),
    send: vi.fn(async () => {}),
    loadOlderMessages: vi.fn(),
    attach: vi.fn(),
    retryChip: vi.fn(),
    removeChip: vi.fn(),
    reorderChips: vi.fn(),
    addFriend: vi.fn(),
    accept: vi.fn(),
    decline: vi.fn(),
    cancel: vi.fn(),
    remove: vi.fn(),
    block: vi.fn(),
    unblock: vi.fn(),
    ...overrides,
  } as unknown as FriendsValue;
}

const conversation = { id: "dm-ada", peer: ADA, callParticipants: [] as string[] };
const view = () => mocks.seen.view!;
const room = () => mocks.seen.room!;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.friends = fakeFriends();
  mocks.ui = fakeUi({ friendsTab: "all" } as Partial<UiValue>);
  mocks.voice = fakeVoice();
  mocks.language = fakeLanguage();
  mocks.profile = { toggle: vi.fn() };
  mocks.seen.room = null;
  mocks.seen.view = null;
});

afterEach(() => {
  vi.useRealTimers();
});

describe("FriendsRoomContainer without a conversation", () => {
  it("shows the list of people, not a conversation", () => {
    render(<FriendsRoomContainer />);

    expect(screen.getByTestId("friends-view")).toBeInTheDocument();
    expect(screen.queryByTestId("dm-room")).toBeNull();
    expect(screen.getByRole("button", { name: "inbox" })).toBeInTheDocument();
  });

  it("lists friends alphabetically with the date the friendship began and their own status", () => {
    mocks.friends = fakeFriends({
      friends: [
        { user: { ...BOB, customStatus: "  w trasie  " }, since: "2026-03-01T10:00:00Z" },
        { user: ADA, since: "2026-01-15T10:00:00Z" },
      ],
    });

    render(<FriendsRoomContainer />);

    expect(view().friends).toEqual([
      {
        person: expect.objectContaining({ id: "u-ada", initials: "AL" }),
        username: "ada",
        since: formatDate("2026-01-15T10:00:00Z", "pl"),
      },
      {
        person: expect.objectContaining({ id: "u-bob", initials: "BO" }),
        username: "bob",
        since: formatDate("2026-03-01T10:00:00Z", "pl"),
        customStatus: "w trasie",
      },
    ]);
  });

  it("describes requests by direction and blocked people without a date", () => {
    mocks.friends = fakeFriends({
      requests: [
        { id: "r-1", user: BOB, direction: "incoming", sentAt: new Date().toISOString() },
      ],
      blocked: [EVE],
    });

    render(<FriendsRoomContainer />);

    expect(view().requests).toEqual([
      expect.objectContaining({ id: "r-1", username: "bob", direction: "incoming" }),
    ]);
    expect(view().requests[0]!.when).not.toBe("");
    expect(view().blocked).toEqual([
      {
        person: expect.objectContaining({ id: "u-eve", avatarUrl: "/eve.png" }),
        username: "eve",
      },
    ]);
    expect(view().blocked[0]).not.toHaveProperty("since");
  });

  it("hands every action to the friends state, and the tab and search request to the ui", () => {
    render(<FriendsRoomContainer />);

    expect(view()).toMatchObject({
      tab: "all",
      onTabChange: mocks.ui.setFriendsTab,
      searchRequested: mocks.ui.friendsSearchRequested,
      onSearchFocused: mocks.ui.takeFriendsSearch,
      onMessage: mocks.friends.messagePerson,
      onCall: mocks.friends.callPerson,
      onRemove: mocks.friends.remove,
      onBlock: mocks.friends.block,
      onUnblock: mocks.friends.unblock,
      onAccept: mocks.friends.accept,
      onDecline: mocks.friends.decline,
      onCancel: mocks.friends.cancel,
      onAddFriend: mocks.friends.addFriend,
      onOpenProfile: mocks.profile.toggle,
    });
  });
});

describe("FriendsRoomContainer with a conversation", () => {
  beforeEach(() => {
    mocks.friends = fakeFriends({ active: conversation });
  });

  it("shows that conversation, not the list, with what the person says about themselves", () => {
    mocks.friends = fakeFriends({
      active: { ...conversation, unread: 2, peer: { ...ADA, customStatus: "  w trasie  " } },
    });
    render(<FriendsRoomContainer />);

    expect(screen.getByTestId("dm-room")).toBeInTheDocument();
    expect(screen.queryByTestId("friends-view")).toBeNull();
    expect(screen.getByRole("button", { name: "inbox" })).toBeInTheDocument();
    expect(room().peer).toMatchObject({ id: "u-ada", displayName: "Ada Lovelace" });
    expect(room()).toMatchObject({
      status: "w trasie",
      profilePanelOpen: mocks.ui.profilePanelOpen,
      onToggleProfilePanel: mocks.ui.toggleProfilePanel,
      chatOpen: mocks.ui.callChatOpen,
      onToggleChat: mocks.ui.toggleCallChat,
      chatUnread: 2,
    });
  });

  it("shows the messages until you are in the call, and then the call", () => {
    mocks.friends = fakeFriends({
      active: { ...conversation, callParticipants: ["u-ada"] },
    });
    const rendered = render(<FriendsRoomContainer />);
    expect(room()).toMatchObject({ callActive: true, inCall: false, videoOn: false });
    expect(screen.getByTestId("dm-chat")).toBeInTheDocument();
    expect(screen.queryByTestId("dm-call")).toBeNull();

    mocks.voice = fakeVoice({
      channelId: "dm-ada",
      call: { cameraOn: true } as VoiceValue["call"],
    });
    rendered.rerender(<FriendsRoomContainer />);
    expect(room()).toMatchObject({ callActive: true, inCall: true, videoOn: true });
    expect(screen.getByTestId("dm-call")).toBeInTheDocument();
    expect(screen.queryByTestId("dm-chat")).toBeNull();
  });

  it("starts or joins the conversation's call", () => {
    render(<FriendsRoomContainer />);

    room().onCall();

    expect(mocks.friends.callConversation).toHaveBeenCalledExactlyOnceWith("dm-ada");
  });

  it("turns the camera on once a video call has been joined", () => {
    vi.useFakeTimers();
    render(<FriendsRoomContainer />);

    room().onVideoCall();

    expect(mocks.friends.callConversation).toHaveBeenCalledExactlyOnceWith("dm-ada");
    expect(mocks.voice.toggleCamera).not.toHaveBeenCalled();
    vi.advanceTimersByTime(500);
    expect(mocks.voice.toggleCamera).toHaveBeenCalledOnce();
  });

  it("only toggles the camera when the call is already up", () => {
    mocks.voice = fakeVoice({ channelId: "dm-ada" });
    render(<FriendsRoomContainer />);

    room().onVideoCall();

    expect(mocks.voice.toggleCamera).toHaveBeenCalledOnce();
    expect(mocks.friends.callConversation).not.toHaveBeenCalled();
  });
});
