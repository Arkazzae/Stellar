import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { playSystemFx } from "@/lib/audio/systemFx";
import type { VoiceClient, VoicePeer } from "@/lib/voice/types";
import { DEFAULT_SETTINGS } from "@/lib/voice/settings";
import type { Ports, SoundboardState } from "@/ports";
import { FakeVoiceClient } from "@/test/fakes";

import { VoiceProvider, useVoiceCallContext, useVoiceContext } from "./VoiceProvider";

const harness = vi.hoisted(() => ({
  client: undefined as unknown as VoiceClient,
  ports: undefined as unknown as Ports,
  preferences: { joinLeaveSound: true },
}));

vi.mock("@/lib/audio/systemFx", () => ({ playSystemFx: vi.fn() }));
vi.mock("@/lib/platform/haptics", () => ({ triggerHaptic: vi.fn() }));

vi.mock("./AccountProvider", () => ({
  useAccountContext: () => ({ preferences: harness.preferences }),
}));

vi.mock("./DirectoryProvider", () => ({
  useDirectoryContext: () => ({
    channelName: (channelId: string | null) => channelId ?? "",
    occupantsOf: () => [],
  }),
}));

vi.mock("./LanguageProvider", () => ({
  useLanguage: () => ({ t: (key: string) => key }),
}));

vi.mock("./PortsProvider", () => ({
  usePorts: () => harness.ports,
}));

vi.mock("./SessionProvider", () => ({
  useSessionContext: () => ({
    user: { id: "self", displayName: "Self" },
  }),
}));

const SOUNDBOARD: SoundboardState = Object.freeze({
  sounds: [],
  settings: { everyoneCanPlay: true, rateLimited: true },
  canManage: false,
  canPlay: true,
  supported: true,
  loading: false,
  error: null,
});

function peer(userId: string, streamId: string): VoicePeer {
  return {
    userId,
    displayName: userId,
    muted: false,
    speaking: false,
    playbackVolume: 100,
    gated: false,
    suppressed: false,
    self: false,
    screen: { streamId, hasAudio: false, stream: null },
    camera: null,
  };
}

function Probe() {
  const voice = useVoiceContext();
  return (
    <div>
      <output aria-label="share-owner">{voice.shares[0]?.ownerId ?? "none"}</output>
      <output aria-label="watching">{String(voice.shares[0]?.watching ?? false)}</output>
      <button type="button" onClick={() => voice.watchShare(voice.shares[0]?.key ?? "")}>
        Watch
      </button>
      <button type="button" onClick={voice.stopWatchingShare}>Stop watching</button>
    </div>
  );
}

beforeEach(() => {
    harness.client = new FakeVoiceClient();
    harness.preferences = { joinLeaveSound: true };
    vi.mocked(playSystemFx).mockClear();
    harness.ports = {
      createVoice: vi.fn(() => harness.client),
      voiceSettings: {
        getSnapshot: () => DEFAULT_SETTINGS,
        subscribe: () => () => {},
        update: vi.fn(),
        reset: vi.fn(),
      },
      soundboard: {
        getSnapshot: () => SOUNDBOARD,
        subscribe: () => () => {},
        setVolume: vi.fn(),
        setDeafened: vi.fn(),
        preparePlayback: vi.fn(),
        cooldownRemaining: () => 0,
        takeJoinAnnouncement: vi.fn(() => false),
        setArrivalsAudible: vi.fn(),
      },
    } as unknown as Ports;
});

describe("VoiceProvider share opt-in", () => {
  it("asks for the watched screen and its sound only while it is watched", () => {
    render(<VoiceProvider><Probe /></VoiceProvider>);
    const client = harness.client as FakeVoiceClient;
    expect(client.setReceivePreferences).toHaveBeenLastCalledWith({ cameras: true, screen: null });

    act(() => client.set({
      channelId: "voice",
      connection: "connected",
      peers: [peer("ada", "screen-a")],
    }));
    expect(client.setReceivePreferences).toHaveBeenLastCalledWith({ cameras: true, screen: null });

    fireEvent.click(screen.getByRole("button", { name: "Watch" }));
    expect(client.setReceivePreferences).toHaveBeenLastCalledWith({
      cameras: true,
      screen: { userId: "ada", streamId: "screen-a" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Stop watching" }));
    expect(client.setReceivePreferences).toHaveBeenLastCalledWith({ cameras: true, screen: null });
  });

  it("does not carry watching across a new stream or owner during reconnect", async () => {
    render(
      <VoiceProvider>
        <Probe />
      </VoiceProvider>,
    );
    await waitFor(() => expect(harness.ports.createVoice).toHaveBeenCalledOnce());

    act(() => {
      (harness.client as FakeVoiceClient).set({
        channelId: "voice",
        connection: "connected",
        peers: [peer("ada", "screen-a-1")],
      });
    });
    expect(screen.getByLabelText("share-owner")).toHaveTextContent("ada");
    expect(screen.getByLabelText("watching")).toHaveTextContent("false");

    fireEvent.click(screen.getByRole("button", { name: "Watch" }));
    expect(screen.getByLabelText("watching")).toHaveTextContent("true");
    expect((harness.client as FakeVoiceClient).setReceivePreferences).toHaveBeenLastCalledWith({
      cameras: true,
      screen: { userId: "ada", streamId: "screen-a-1" },
    });

    act(() => {
      (harness.client as FakeVoiceClient).set({ peers: [peer("ada", "screen-a-2")] });
    });
    expect(screen.getByLabelText("watching")).toHaveTextContent("false");
    expect((harness.client as FakeVoiceClient).setReceivePreferences).toHaveBeenLastCalledWith({
      cameras: true,
      screen: null,
    });

    act(() => {
      (harness.client as FakeVoiceClient).set({ peers: [peer("bartek", "screen-b-1")] });
    });
    expect(screen.getByLabelText("share-owner")).toHaveTextContent("bartek");
    expect(screen.getByLabelText("watching")).toHaveTextContent("false");
  });
});

describe("VoiceProvider arrival sounds", () => {
  function talker(userId: string): VoicePeer {
    return { ...peer(userId, "unused"), screen: null };
  }

  it("announces a join and a departure while the preference stands", async () => {
    render(
      <VoiceProvider>
        <Probe />
      </VoiceProvider>,
    );
    await waitFor(() => expect(harness.ports.createVoice).toHaveBeenCalledOnce());

    act(() => {
      (harness.client as FakeVoiceClient).set({
        channelId: "voice",
        connection: "connected",
        peers: [],
      });
    });
    expect(playSystemFx).toHaveBeenCalledWith("join");

    act(() => {
      (harness.client as FakeVoiceClient).set({ peers: [talker("ada")] });
    });
    act(() => {
      (harness.client as FakeVoiceClient).set({ peers: [] });
    });
    expect(playSystemFx).toHaveBeenCalledWith("leave");
  });

  it("stays quiet for every coming and going once the preference is off", async () => {
    harness.preferences = { joinLeaveSound: false };
    render(
      <VoiceProvider>
        <Probe />
      </VoiceProvider>,
    );
    await waitFor(() => expect(harness.ports.createVoice).toHaveBeenCalledOnce());

    act(() => {
      (harness.client as FakeVoiceClient).set({
        channelId: "voice",
        connection: "connected",
        peers: [],
      });
    });
    act(() => {
      (harness.client as FakeVoiceClient).set({ peers: [talker("ada")] });
    });
    act(() => {
      (harness.client as FakeVoiceClient).set({ peers: [] });
    });

    expect(playSystemFx).not.toHaveBeenCalledWith("join");
    expect(playSystemFx).not.toHaveBeenCalledWith("leave");
    // The soundboard is told as well: an arrival clip is the same
    // announcement, only longer.
    expect(harness.ports.soundboard.setArrivalsAudible).toHaveBeenCalledWith(false);
  });
});

describe("VoiceProvider call dock", () => {
  it("leaves the dock alone while only speech moves, and follows a mute", async () => {
    const docks: unknown[] = [];
    function Dock() {
      docks.push(useVoiceCallContext());
      return null;
    }
    render(
      <VoiceProvider>
        <Dock />
      </VoiceProvider>,
    );
    await waitFor(() => expect(harness.ports.createVoice).toHaveBeenCalledOnce());
    const client = harness.client as FakeVoiceClient;
    const ada = { ...peer("ada", "unused"), screen: null };
    act(() => client.set({ channelId: "voice", connection: "connected", peers: [ada] }));
    const rendered = docks.length;

    act(() => client.set({ peers: [{ ...ada, speaking: true }] }));
    act(() => client.set({ peers: [{ ...ada, speaking: false }] }));
    expect(docks).toHaveLength(rendered);

    act(() => client.set({ muted: true }));
    expect(docks).toHaveLength(rendered + 1);
    expect(docks.at(-1)).toMatchObject({ call: { muted: true } });
  });
});
