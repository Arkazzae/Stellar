import { describe, expect, it, vi } from "vitest";
import type { ChannelId, UserId } from "@/lib/api/types";
import { VoiceStateStore } from "./store";
import { IDLE_DIAGNOSTICS, IDLE_VOICE_STATE, type VoicePeer } from "./types";

function peer(overrides: Partial<VoicePeer> = {}): VoicePeer {
  return {
    userId: "u1" as UserId,
    displayName: "Ada",
    muted: false,
    speaking: false,
    playbackVolume: 100,
    gated: false,
    suppressed: false,
    self: false,
    screen: null,
    camera: null,
    ...overrides,
  };
}

function subscribed(store: VoiceStateStore) {
  const listener = vi.fn();
  store.subscribe(listener);
  return listener;
}

describe("VoiceStateStore", () => {
  it("starts on the shared idle snapshot", () => {
    expect(new VoiceStateStore().getSnapshot()).toBe(IDLE_VOICE_STATE);
  });

  it("publishes a new frozen snapshot when something changed", () => {
    const store = new VoiceStateStore();
    const listener = subscribed(store);
    const before = store.getSnapshot();

    store.set({ connection: "connecting", channelId: "c1" as ChannelId });

    const after = store.getSnapshot();
    expect(after).not.toBe(before);
    expect(after).toMatchObject({ connection: "connecting", channelId: "c1" });
    expect(Object.isFrozen(after)).toBe(true);
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("keeps the snapshot and stays silent when a patch changes nothing", () => {
    const store = new VoiceStateStore();
    store.set({ muted: true });
    const listener = subscribed(store);
    const before = store.getSnapshot();

    store.set({ muted: true });
    store.set({});

    expect(store.getSnapshot()).toBe(before);
    expect(listener).not.toHaveBeenCalled();
  });

  it("treats an equal copy of the peers as unchanged", () => {
    const store = new VoiceStateStore();
    store.set({ peers: [peer()] });
    const listener = subscribed(store);

    store.set({ peers: [peer()] });
    expect(listener).not.toHaveBeenCalled();

    store.set({ peers: [peer({ muted: true })] });
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("notices which screen a peer watches", () => {
    const store = new VoiceStateStore();
    store.set({ peers: [peer()] });
    const listener = subscribed(store);

    store.set({ peers: [peer({ watching: { userId: "u2" as UserId, streamId: "s1" } })] });
    store.set({ peers: [peer({ watching: { userId: "u2" as UserId, streamId: "s1" } })] });
    store.set({ peers: [peer({ watching: { userId: "u2" as UserId, streamId: "s2" } })] });

    expect(listener).toHaveBeenCalledTimes(2);
  });

  it("notices a peer joining or leaving", () => {
    const store = new VoiceStateStore();
    store.set({ peers: [peer()] });
    const listener = subscribed(store);

    store.set({ peers: [peer(), peer({ userId: "u2" as UserId })] });
    store.set({ peers: [] });

    expect(listener).toHaveBeenCalledTimes(2);
  });

  it("keeps the RTP counters beside the snapshot, where moving them renders nothing", () => {
    const store = new VoiceStateStore();
    const listener = subscribed(store);

    store.setCounters({ bytesSent: 400, bytesReceived: 560, packetsSent: 5, packetsReceived: 7 });

    expect(listener).not.toHaveBeenCalled();
    expect(store.getCounters().packetsReceived).toBe(7);
    store.set({ muted: true });
    store.reset();
    expect(store.getCounters().packetsReceived).toBe(0);
  });

  it("ignores loss and round-trip jitter below what the readout displays", () => {
    const store = new VoiceStateStore();
    store.set({ diagnostics: { ...IDLE_DIAGNOSTICS, lossFraction: 0.0101, rttMs: 40.2 } });
    const listener = subscribed(store);

    store.set({ diagnostics: { ...IDLE_DIAGNOSTICS, lossFraction: 0.0103, rttMs: 40.4 } });
    expect(listener).not.toHaveBeenCalled();

    store.set({ diagnostics: { ...IDLE_DIAGNOSTICS, lossFraction: 0.0204, rttMs: 40.4 } });
    store.set({ diagnostics: { ...IDLE_DIAGNOSTICS, lossFraction: 0.0204, rttMs: 85 } });
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it("notices a reading appearing where there was none", () => {
    const store = new VoiceStateStore();
    const listener = subscribed(store);

    store.set({ diagnostics: { ...IDLE_DIAGNOSTICS, rttMs: 0 } });

    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("tells errors apart by kind and message", () => {
    const store = new VoiceStateStore();
    store.set({ error: { kind: "transport", message: "lost" } });
    const listener = subscribed(store);

    store.set({ error: { kind: "transport", message: "lost" } });
    expect(listener).not.toHaveBeenCalled();

    store.set({ error: { kind: "transport", message: "lost again" } });
    store.set({ error: { kind: "signaling", message: "lost again" } });
    store.set({ error: null });
    expect(listener).toHaveBeenCalledTimes(3);
  });

  it("notices a screen share whose stream was swapped", () => {
    const store = new VoiceStateStore();
    const first = {} as MediaStream;
    store.set({ localScreen: { streamId: "s", hasAudio: false, stream: first } });
    const listener = subscribed(store);

    store.set({ localScreen: { streamId: "s", hasAudio: false, stream: first } });
    expect(listener).not.toHaveBeenCalled();

    store.set({ localScreen: { streamId: "s", hasAudio: false, stream: {} as MediaStream } });
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("notices a picture of a peer being held back and coming back", () => {
    const store = new VoiceStateStore();
    const stream = {} as MediaStream;
    const camera = { streamId: "c", stream };
    const screen = { streamId: "s", hasAudio: false, stream };
    store.set({ peers: [peer({ camera, screen })] });
    const listener = subscribed(store);

    store.set({ peers: [peer({ camera: { ...camera, heldBack: true }, screen })] });
    store.set({ peers: [peer({ camera: { ...camera, heldBack: true }, screen: { ...screen, heldBack: true } })] });
    store.set({ peers: [peer({ camera, screen })] });

    expect(listener).toHaveBeenCalledTimes(3);
  });

  it("stops notifying a listener once it unsubscribes", () => {
    const store = new VoiceStateStore();
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);

    unsubscribe();
    store.set({ muted: true });

    expect(listener).not.toHaveBeenCalled();
  });

  it("returns to the shared idle snapshot on reset", () => {
    const store = new VoiceStateStore();
    store.set({ connection: "connected" });
    const listener = subscribed(store);

    store.reset();

    expect(store.getSnapshot()).toBe(IDLE_VOICE_STATE);
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("does not notify when reset finds the store already idle", () => {
    const store = new VoiceStateStore();
    const listener = subscribed(store);

    store.reset();

    expect(listener).not.toHaveBeenCalled();
  });
});
