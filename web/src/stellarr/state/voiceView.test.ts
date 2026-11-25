import { describe, expect, it } from "vitest";

import type { PersonVM } from "@/components/view-models";
import { IDLE_VOICE_STATE, type VoicePeer, type VoiceState } from "@/lib/voice/types";

import {
  callHeadline,
  deriveVoiceView,
  shareSourcesOf,
  type VoiceViewInput,
} from "./voiceView";

function stateWith(patch: Partial<VoiceState>): VoiceState {
  return { ...IDLE_VOICE_STATE, ...patch };
}

function peer(userId: string, patch: Partial<VoicePeer> = {}): VoicePeer {
  return {
    userId,
    displayName: userId.toUpperCase(),
    muted: false,
    speaking: false,
    playbackVolume: 100,
    gated: false,
    suppressed: false,
    self: false,
    screen: null,
    camera: null,
    ...patch,
  };
}

function person(id: string): PersonVM {
  return { id, displayName: id, initials: id.slice(0, 2), presence: "online" };
}

function inputFor(state: VoiceState, patch: Partial<VoiceViewInput> = {}): VoiceViewInput {
  return {
    state,
    userId: "me",
    displayName: "Me",
    startedAt: 1_000,
    deafened: false,
    room: null,
    roomName: state.channelId,
    watchedKey: null,
    watchFresh: false,
    occupantsOf: () => [],
    sources: shareSourcesOf(state, "me"),
    ...patch,
  };
}

const transport = IDLE_VOICE_STATE.transport;

describe("callHeadline", () => {
  it("shows the screen over a healthy connection", () => {
    const state = stateWith({
      connection: "connected",
      localScreen: { streamId: "s", hasAudio: false, stream: null },
    });

    expect(callHeadline(state)).toBe("screen");
  });

  it("lets a failed connection outrank a share that is still live", () => {
    const state = stateWith({
      connection: "failed",
      localScreen: { streamId: "s", hasAudio: false, stream: null },
    });

    expect(callHeadline(state)).toBe("failed");
  });

  it("reports failed media before a reconnecting signal", () => {
    const state = stateWith({
      connection: "connected",
      transport: { ...transport, media: "failed", signaling: "reconnecting" },
    });

    expect(callHeadline(state)).toBe("failed");
  });

  it.each(["reconnecting", "disconnected"] as const)(
    "reports a %s signalling channel as reconnecting",
    (signaling) => {
      const state = stateWith({
        connection: "connected",
        transport: { ...transport, signaling },
      });

      expect(callHeadline(state)).toBe("reconnecting");
    },
  );

  it("reports degraded media on an otherwise connected call", () => {
    const state = stateWith({
      connection: "connected",
      transport: { ...transport, media: "degraded" },
    });

    expect(callHeadline(state)).toBe("degraded");
  });

  it("waits on the microphone before anything else", () => {
    expect(callHeadline(stateWith({ connection: "requesting-mic" }))).toBe("requesting-mic");
  });

  it("is connecting while media negotiates", () => {
    const state = stateWith({
      connection: "connected",
      transport: { ...transport, media: "connecting" },
    });

    expect(callHeadline(state)).toBe("connecting");
  });
});

describe("shareSourcesOf", () => {
  it("has no source when nobody shares", () => {
    const state = stateWith({ peers: [peer("ada")] });

    expect(shareSourcesOf(state, "me")).toEqual([]);
  });

  it("lists the local share before the remote ones", () => {
    const ada = peer("ada", { screen: { streamId: "theirs", hasAudio: false, stream: null } });
    const state = stateWith({
      localScreen: { streamId: "mine", hasAudio: false, stream: null },
      peers: [ada],
    });

    expect(shareSourcesOf(state, "me")).toEqual([
      { key: "local:me:mine", ownerId: "me", remote: null },
      { key: "remote:ada:theirs", ownerId: "ada", remote: ada },
    ]);
  });

  it("names the remote stream, so a new stream is a different source", () => {
    const withStream = (streamId: string) =>
      shareSourcesOf(
        stateWith({
          peers: [peer("ada", { screen: { streamId, hasAudio: false, stream: null } })],
        }),
        "me",
      )[0]!;

    expect(withStream("one").key).toBe("remote:ada:one");
    expect(withStream("two").key).not.toBe(withStream("one").key);
    expect(withStream("one").ownerId).toBe("ada");
  });

  it("still names the local share before the user is known", () => {
    const state = stateWith({
      localScreen: { streamId: "mine", hasAudio: false, stream: null },
    });

    expect(shareSourcesOf(state, null)[0]?.ownerId).toBe("self");
  });
});

describe("deriveVoiceView", () => {
  describe("call", () => {
    it("is absent while the client is idle", () => {
      const view = deriveVoiceView(inputFor(stateWith({ channelId: "voice" })));

      expect(view.call).toBeNull();
    });

    it("is absent without a channel", () => {
      const view = deriveVoiceView(inputFor(stateWith({ connection: "connected" })));

      expect(view.call).toBeNull();
    });

    it("carries the room name, mute, deafen and rounded latency", () => {
      const state = stateWith({
        channelId: "voice",
        connection: "connected",
        muted: true,
        diagnostics: { ...IDLE_VOICE_STATE.diagnostics, rttMs: 41.6, quality: "good" },
      });

      const { call } = deriveVoiceView(
        inputFor(state, { roomName: "Lounge", deafened: true, startedAt: 7 }),
      );

      expect(call).toMatchObject({
        channelId: "voice",
        channelName: "Lounge",
        startedAt: 7,
        muted: true,
        deafened: true,
        latencyMs: 42,
        connectionQuality: "good",
        headline: "connected",
      });
      expect(call).not.toHaveProperty("privateCall");
    });

    it("reports no latency before the first sample", () => {
      const state = stateWith({ channelId: "voice", connection: "connected" });

      expect(deriveVoiceView(inputFor(state)).call?.latencyMs).toBeNull();
    });

    it("marks a private call", () => {
      const state = stateWith({ channelId: "dm", connection: "connected" });

      const view = deriveVoiceView(
        inputFor(state, { room: { label: "Ada", privateCall: true } }),
      );

      expect(view.privateCall).toBe(true);
      expect(view.call?.privateCall).toBe(true);
    });
  });

  describe("people", () => {
    it("is empty outside a call", () => {
      const view = deriveVoiceView(inputFor(stateWith({}), { occupantsOf: () => [person("ada")] }));

      expect(view.people).toEqual([]);
    });

    it("lists the directory's occupants with live speech applied", () => {
      const state = stateWith({
        channelId: "voice",
        connection: "connected",
        peers: [peer("ada", { speaking: true, muted: true })],
      });

      const view = deriveVoiceView(inputFor(state, { occupantsOf: () => [person("ada")] }));

      expect(view.people).toEqual([{ ...person("ada"), speaking: true, muted: true }]);
    });

    it("adds a negotiated peer the directory has not caught up with", () => {
      const state = stateWith({
        channelId: "voice",
        connection: "connected",
        peers: [peer("ada"), peer("bartek", { displayName: "Bartek Nowak", speaking: true })],
      });

      const view = deriveVoiceView(inputFor(state, { occupantsOf: () => [person("ada")] }));

      expect(view.people.map((p) => p.id)).toEqual(["ada", "bartek"]);
      expect(view.people[1]).toMatchObject({
        displayName: "Bartek Nowak",
        initials: "BN",
        presence: "online",
        speaking: true,
      });
    });

    it("prefers the caller's own list over the directory's", () => {
      const state = stateWith({ channelId: "dm", connection: "connecting" });

      const view = deriveVoiceView(
        inputFor(state, {
          room: { label: "Ada", people: [person("ada")] },
          occupantsOf: () => [person("someone-else")],
        }),
      );

      expect(view.people.map((p) => p.id)).toEqual(["ada"]);
    });
  });

  describe("shares", () => {
    const remoteScreen: NonNullable<VoicePeer["screen"]> = {
      streamId: "s",
      hasAudio: true,
      stream: {} as MediaStream,
    };
    const KEY = "remote:ada:s";

    function stateOf(screen = remoteScreen, patch: Partial<VoicePeer> = {}) {
      return stateWith({
        channelId: "voice",
        connection: "connected",
        peers: [peer("ada", { screen, ...patch })],
      });
    }

    function remoteShare(watching: boolean, patch: Partial<VoiceViewInput> = {}) {
      const state = stateOf();
      return deriveVoiceView(inputFor(state, { watchedKey: watching ? KEY : null, ...patch })).shares[0];
    }

    it("withholds a remote stream until the viewer opts in", () => {
      expect(remoteShare(false)).toEqual({
        key: KEY,
        ownerId: "ada",
        ownerName: "ADA",
        local: false,
        hasAudio: true,
        stream: null,
        watching: false,
      });
    });

    it("hands the remote stream over once the viewer opts in", () => {
      expect(remoteShare(true)).toMatchObject({ stream: remoteScreen.stream, watching: true });
      expect(remoteShare(true)).not.toHaveProperty("heldBack");
      expect(remoteShare(true)).not.toHaveProperty("connecting");
    });

    it("says a watched share is connecting until its first picture arrives", () => {
      const state = stateOf({ ...remoteScreen, stream: null as unknown as MediaStream });

      expect(deriveVoiceView(inputFor(state, { watchedKey: KEY })).shares[0]).toMatchObject({
        stream: null,
        watching: true,
        connecting: true,
      });
    });

    it("calls a picture held back right after the opt-in connecting, and held back later on", () => {
      const state = stateOf({ ...remoteScreen, heldBack: true });

      const fresh = deriveVoiceView(inputFor(state, { watchedKey: KEY, watchFresh: true })).shares[0];
      expect(fresh).toMatchObject({ stream: null, connecting: true });
      expect(fresh).not.toHaveProperty("heldBack");

      const later = deriveVoiceView(inputFor(state, { watchedKey: KEY })).shares[0];
      expect(later).toMatchObject({ stream: null, heldBack: true });
      expect(later).not.toHaveProperty("connecting");

      // A share nobody watches is not asked for, so nothing of it is held back.
      expect(deriveVoiceView(inputFor(state)).shares[0]).not.toHaveProperty("heldBack");
    });

    it("plays nothing of a watched picture that does not decrypt", () => {
      const state = stateOf(remoteScreen, { undecryptable: { audio: false, video: true } });

      expect(deriveVoiceView(inputFor(state, { watchedKey: KEY })).shares[0]).toMatchObject({
        stream: null,
        sealed: true,
      });
    });

    it("lists every screen in the call, the user's own first, and watches one at a time", () => {
      const own = {} as MediaStream;
      const state = stateWith({
        channelId: "voice",
        connection: "connected",
        localScreen: { streamId: "mine", hasAudio: false, stream: own },
        peers: [
          peer("ada", { screen: remoteScreen }),
          peer("bo", { screen: { streamId: "t", hasAudio: false, stream: {} as MediaStream } }),
        ],
      });

      const shares = deriveVoiceView(inputFor(state, { watchedKey: "remote:bo:t" })).shares;

      expect(shares.map((share) => [share.key, share.watching])).toEqual([
        ["local:me:mine", false],
        [KEY, false],
        ["remote:bo:t", true],
      ]);
      expect(shares[0]).toMatchObject({ local: true, ownerName: "Me", stream: own });
    });

    it("says who watches each screen, the viewer included, and who watches yours", () => {
      const state = stateWith({
        channelId: "voice",
        connection: "connected",
        localScreen: { streamId: "mine", hasAudio: false, stream: null },
        peers: [
          peer("ada", { screen: remoteScreen, watching: { userId: "me", streamId: "mine" } }),
          peer("bo", { watching: { userId: "ada", streamId: "s" } }),
          // An old stream of Ada's is not the one on show.
          peer("cy", { watching: { userId: "ada", streamId: "old" } }),
        ],
      });

      const viewers = deriveVoiceView(inputFor(state, { watchedKey: KEY })).viewers;

      expect(viewers.get(KEY)?.map((person) => person.id)).toEqual(["me", "bo"]);
      expect(viewers.get("local:me:mine")?.map((person) => person.id)).toEqual(["ada"]);
    });

    it("is empty when nobody shares", () => {
      const state = stateWith({ channelId: "voice", connection: "connected", peers: [peer("ada")] });

      expect(deriveVoiceView(inputFor(state)).shares).toEqual([]);
    });
  });

  describe("per-participant settings", () => {
    it("leaves the user's own strip out", () => {
      const state = stateWith({
        channelId: "voice",
        connection: "connected",
        peers: [
          peer("me", { self: true, playbackVolume: 100 }),
          peer("ada", { playbackVolume: 150, gated: true, suppressed: true }),
        ],
      });

      const view = deriveVoiceView(inputFor(state));

      expect([...view.participantVolumes]).toEqual([["ada", 150]]);
      expect([...view.participantGates]).toEqual([["ada", true]]);
      expect([...view.participantSuppressions]).toEqual([["ada", true]]);
    });
  });

  describe("camera streams", () => {
    it("collects remote cameras and the user's own preview by id", () => {
      const remote = {} as MediaStream;
      const local = {} as MediaStream;
      const state = stateWith({
        channelId: "voice",
        connection: "connected",
        peers: [peer("ada", { camera: { streamId: "c", stream: remote } })],
        localCamera: { streamId: "mine", stream: local },
      });

      const view = deriveVoiceView(inputFor(state));

      expect(view.cameraStreams.get("ada")).toBe(remote);
      expect(view.cameraStreams.get("me")).toBe(local);
    });

    it("shows no frozen frame of a held camera and names whose it is", () => {
      const stream = {} as MediaStream;
      const state = stateWith({
        channelId: "voice",
        connection: "connected",
        peers: [
          peer("ada", { camera: { streamId: "c", stream, heldBack: true } }),
          peer("ilya", { camera: { streamId: "d", stream } }),
        ],
      });

      const view = deriveVoiceView(inputFor(state));

      expect(view.cameraStreams.has("ada")).toBe(false);
      expect(view.cameraStreams.get("ilya")).toBe(stream);
      expect([...view.heldCameras]).toEqual(["ada"]);
      expect(deriveVoiceView(inputFor(stateWith({}))).heldCameras.size).toBe(0);
    });

    it("hands over no picture that does not decrypt, and says whose sound or picture it is", () => {
      const stream = {} as MediaStream;
      const state = stateWith({
        channelId: "voice",
        connection: "connected",
        encrypted: true,
        peers: [peer("ada", { camera: { streamId: "c", stream }, undecryptable: { audio: false, video: true } })],
      });

      const view = deriveVoiceView(inputFor(state));

      expect(view.cameraStreams.has("ada")).toBe(false);
      expect(view.people.find((person) => person.id === "ada")?.undecryptable).toEqual({ audio: false, video: true });
      expect(view.call?.encrypted).toBe(true);
    });

    it("shares one empty map while no camera is on", () => {
      const first = deriveVoiceView(inputFor(stateWith({}))).cameraStreams;
      const second = deriveVoiceView(inputFor(stateWith({ channelId: "x" }))).cameraStreams;

      expect(first.size).toBe(0);
      expect(second).toBe(first);
    });
  });
});
