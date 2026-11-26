import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { DEFAULT_SETTINGS } from "@/lib/voice/settings";
import type {
  ActivitiesValue,
  ChatValue,
  DirectoryValue,
  UiValue,
  VoiceValue,
} from "@/state/hooks";
import {
  fakeChat,
  fakeDirectory,
  fakeLanguage,
  fakeUi,
  fakeVoice,
  t,
} from "@/test/roomFakes";

import { RoomContainer } from "./RoomContainer";

const mocks = vi.hoisted(() => ({
  directory: null as unknown as DirectoryValue,
  chat: null as unknown as ChatValue,
  ui: null as unknown as UiValue,
  voice: null as unknown as VoiceValue,
  activities: null as unknown as ActivitiesValue,
  language: null as unknown,
  joinFromPopoutLink: vi.fn(),
  profile: { toggle: vi.fn() },
  ports: {
    typing: { onTypingChanged: () => () => {}, typingIn: () => NOBODY },
    voiceSettings: { subscribe: () => () => {}, getSnapshot: () => ({}), update: vi.fn() },
    soundboard: {
      live: {
        subscribe: () => () => undefined,
        playing: () => [],
        cooldownSeconds: () => 0,
      },
    },
    chat: { listIndexed: async () => ({ messages: [], nextCursor: null }) },
  },
}));

vi.mock("@/state/hooks", () => ({
  useDirectory: () => mocks.directory,
  useSession: () => ({ user: { id: "u-maja" } }),
  useUi: () => mocks.ui,
  useChatRoom: () => mocks.chat,
  useChatTranscript: () => mocks.chat,
  useVoice: () => mocks.voice,
  useActivities: () => mocks.activities,
  useActivitySlot: () => mocks.activities.publishSlot.voice,
  useEmoji: () => ({ emoji: [] }),
  // The call's bar names the talk key; none is set here.
  useKeybinds: () => ({ bindings: { pushToTalk: null } }),
  useLanguage: () => mocks.language,
  useAccount: () => ({ preferences: { developerMode: false } }),
  useProfile: () => mocks.profile,
  usePorts: () => mocks.ports,
}));

vi.mock("./callPopout", async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  useJoinFromPopoutLink: mocks.joinFromPopoutLink,
}));
vi.mock("./NotificationInboxContainer", () => ({ NotificationInboxButton: () => null }));
vi.mock("@/components/room/transcript/Transcript", () => ({
  Transcript: () => <div role="log" />,
}));
vi.mock("@/components/room/panels/ThreadPanel", () => ({
  ThreadPanel: (props: { composer?: ReactNode }) => (
    <aside aria-label="thread-stub">{props.composer}</aside>
  ),
}));
vi.mock("@/components/room/composer/Composer", () => ({
  Composer: (props: { placeholder?: string }) => (
    <form aria-label="composer-stub" data-placeholder={props.placeholder ?? ""} />
  ),
}));
vi.mock("@/components/room/panels/ChannelPanel", () => ({
  ChannelPanel: (props: { open: boolean }) => (
    <div data-testid="channel-panel" data-open={props.open} />
  ),
}));

const MAJA = { id: "u-maja", displayName: "Maja", initials: "MA", presence: "online" as const };
const ADA = { id: "u-ada", displayName: "Ada", initials: "AD", presence: "online" as const };
// Read only once the room renders, after this module has run.
const NOBODY: string[] = [];

function inCall(overrides: Partial<VoiceValue> = {}) {
  mocks.ui = fakeUi({ roomView: "voice", activeTextChannelId: "ch-general" });
  mocks.voice = fakeVoice({
    channelId: "ch-salon",
    people: [MAJA, ADA],
    call: { channelId: "ch-salon", channelName: "Salon", cameraOn: false } as VoiceValue["call"],
    ...overrides,
  });
}

function views(container: HTMLElement) {
  const [text, call] = Array.from(container.querySelectorAll<HTMLElement>(".room-body > .view"));
  return { text: text!, call: call! };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.directory = fakeDirectory({
    channels: [
      { id: "ch-general", name: "general", kind: "text" },
      { id: "ch-salon", name: "Salon", kind: "voice", topic: "Daily sync" },
    ] as DirectoryValue["channels"],
    channelName: (id) => (id === "ch-salon" ? "Salon" : ""),
  });
  mocks.chat = fakeChat();
  mocks.language = fakeLanguage();
  mocks.profile = { toggle: vi.fn() };
  mocks.ports.voiceSettings.getSnapshot = () => DEFAULT_SETTINGS;
  mocks.ui = fakeUi();
  mocks.voice = fakeVoice();
  mocks.activities = {
    live: new Map(),
    voice: null,
    joining: null,
    joinFailure: null,
    ended: null,
    openPicker: vi.fn(),
    join: vi.fn(),
    publishSlot: { voice: vi.fn(), personal: vi.fn() },
  } as unknown as ActivitiesValue;
});

afterEach(() => {
  Reflect.deleteProperty(HTMLElement.prototype, "requestFullscreen");
});

describe("RoomContainer in a call", () => {
  it("shows the stage with the people in the call and the controls under it", () => {
    inCall();

    const { container } = render(<RoomContainer />);

    const { call, text } = views(container);
    expect(call).toHaveClass("active");
    expect(text).not.toHaveClass("active");
    expect(container.querySelector("main")).toHaveClass("in-voice");
    expect(within(call).getByText("Maja")).toBeInTheDocument();
    expect(within(call).getByText("Ada")).toBeInTheDocument();
    expect(within(call).getByRole("button", { name: t("voiceBar.disconnect") })).toBeInTheDocument();
  });

  it("keeps the transcript on screen, and the stage out of sight, in a text channel", () => {
    mocks.voice = fakeVoice({ people: [MAJA] });

    const { container } = render(<RoomContainer />);

    const { call, text } = views(container);
    expect(text).toHaveClass("active");
    expect(call).not.toHaveClass("active");
    expect(container.querySelector("main")).not.toHaveClass("in-voice");
  });

  it("names the room after the call and shows the voice channel's topic", () => {
    inCall();

    render(<RoomContainer />);

    expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent("Salon");
    expect(screen.getByText("Daily sync")).toBeInTheDocument();
  });

  it("falls back to the channel's name, then the text channel's, for an untitled call", () => {
    inCall({ call: null });
    const view = render(<RoomContainer />);
    expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent("Salon");

    mocks.voice = fakeVoice({ channelId: "ch-unknown" });
    view.rerender(<RoomContainer />);
    expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent("general");
  });

  it("closes the panels and the drop target while the stage has the room", () => {
    inCall();
    mocks.ui = fakeUi({
      roomView: "voice",
      channelPanel: "pinned",
      channelPanelTab: "pinned",
    });
    mocks.chat = fakeChat({
      thread: { rootId: "m-1", initialExpanded: false, openRevision: 1 },
    });

    const { container } = render(<RoomContainer />);
    const payload = { dataTransfer: { types: ["Files"], files: [new File(["a"], "a.txt")] } };
    fireEvent.dragEnter(window, payload);
    fireEvent.drop(window, payload);

    expect(screen.getByTestId("channel-panel")).toHaveAttribute("data-open", "false");
    expect(container.querySelector("main")).not.toHaveClass("channel-panel-open");
    expect(container.querySelector("main")).not.toHaveClass("thread-panel-open");
    expect(mocks.chat.attach).not.toHaveBeenCalled();
    // The thread is not visible, so the composer belongs to the channel again.
    const channelComposer = within(views(container).text).getByRole("form");
    expect(channelComposer).toHaveAttribute("data-placeholder", "");
  });

  it("lets a popped-out window dial the call its address names", () => {
    inCall();

    render(<RoomContainer />);

    expect(mocks.joinFromPopoutLink).toHaveBeenCalledWith(
      mocks.ui.joinVoiceChannel, mocks.directory.server.id, mocks.directory.channels,
    );
  });

  it("asks for full screen on the whole room, not on the stage", async () => {
    const user = userEvent.setup();
    const requestFullscreen = vi.fn();
    HTMLElement.prototype.requestFullscreen = requestFullscreen;
    inCall();
    const { container } = render(<RoomContainer />);

    await user.click(screen.getByRole("button", { name: t("voiceBar.fullscreen") }));

    expect(requestFullscreen).toHaveBeenCalledOnce();
    expect(requestFullscreen.mock.contexts[0]).toBe(container.querySelector("main"));
  });
});

describe("RoomContainer with an activity on the channel", () => {
  const synctube = {
    install: "synctube",
    name: "SyncTube",
    icon: null,
    host: "u-ada",
    participants: ["u-ada"],
    seats: null,
  };

  it("joins what runs in one click from outside, and gives it the stage once inside", () => {
    inCall();
    mocks.activities = { ...mocks.activities, live: new Map([["ch-salon", synctube]]) };
    const { container, rerender } = render(<RoomContainer />);

    expect(screen.queryByRole("button", { name: "Aktywności" })).toBeNull();
    expect(within(views(container).call).getByRole("img", { name: "W SyncTube" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Dołącz do SyncTube" }));
    expect(mocks.activities.join).toHaveBeenCalledWith("ch-salon");
    expect(container.querySelector(".act-slot")).toBeNull();

    mocks.activities = {
      ...mocks.activities,
      voice: { place: "ch-salon", install: "synctube", access: "a", src: "", title: "SyncTube", icon: null, permissions: [], embedHosts: [] },
    };
    rerender(<RoomContainer />);

    expect(container.querySelector(".act-slot")).not.toBeNull();
    expect(container.querySelector(".voice-stage")).toHaveAttribute("data-layout", "strip");
    expect(screen.queryByRole("button", { name: "Dołącz do SyncTube" })).toBeNull();
    expect(mocks.activities.publishSlot.voice).toHaveBeenCalledWith(container.querySelector(".act-slot"));
  });

  it("opens the picker from the bar while nothing runs", () => {
    inCall();
    render(<RoomContainer />);

    fireEvent.click(screen.getByRole("button", { name: "Aktywności" }));
    expect(mocks.activities.openPicker).toHaveBeenCalledWith({ scope: "voice", channelId: "ch-salon", change: false });
  });
});

describe("RoomContainer call controls", () => {
  it("drives the call from the control bar", async () => {
    const user = userEvent.setup();
    inCall();
    render(<RoomContainer />);

    await user.click(screen.getByRole("button", { name: t("voice.microphone") }));
    await user.click(screen.getByRole("button", { name: t("voiceBar.cameraStart") }));
    await user.click(screen.getByRole("button", { name: t("voiceBar.shareScreen") }));
    await user.click(screen.getByRole("button", { name: t("voiceBar.cinema") }));
    await user.click(screen.getByRole("button", { name: t("voiceBar.disconnect") }));

    expect(mocks.voice.toggleMute).toHaveBeenCalledOnce();
    expect(mocks.voice.toggleCamera).toHaveBeenCalledOnce();
    expect(mocks.voice.toggleShare).toHaveBeenCalledOnce();
    expect(mocks.ui.toggleCinema).toHaveBeenCalledOnce();
    expect(mocks.ui.leaveCall).toHaveBeenCalledOnce();
  });

  it("offers an invitation only to someone who can manage the server", async () => {
    const user = userEvent.setup();
    inCall();
    const view = render(<RoomContainer />);
    expect(screen.queryByRole("button", { name: t("voiceBar.inviteChannel") })).toBeNull();

    mocks.directory = fakeDirectory({ canManageServer: true });
    view.rerender(<RoomContainer />);
    await user.click(screen.getByRole("button", { name: t("voiceBar.inviteChannel") }));

    expect(mocks.ui.openModal).toHaveBeenCalledExactlyOnceWith("invite");
  });

  it("opens the call in a window of its own", async () => {
    const user = userEvent.setup();
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    inCall();
    render(<RoomContainer />);

    await user.click(screen.getByRole("button", { name: t("voiceBar.popOut") }));

    const [url, name] = open.mock.calls[0]!;
    expect(String(url)).toContain("popout=1");
    expect(String(url)).toContain("voiceChannel=ch-salon");
    expect(new URL(String(url)).searchParams.get("serverId")).toBe(mocks.directory.server.id);
    expect(name).toBe("stellarr-call");
    open.mockRestore();
  });
});
