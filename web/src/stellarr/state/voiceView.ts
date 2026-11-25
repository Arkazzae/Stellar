import type { CallVM, PersonVM, ScreenShareVM } from "@/components/view-models";
import { initials } from "@/lib/format/format";
import type { VoicePeer, VoiceState } from "@/lib/voice/types";

/** How a room outside the server directory, such as a DM call, presents itself. */
export interface VoiceRoomPresentation {
  /** Human-readable dock label for rooms not present in the server directory. */
  label: string;
  /** People to show before their peer connection has finished negotiating. */
  people?: readonly PersonVM[];
  privateCall?: boolean;
}

const NO_CAMERAS: ReadonlyMap<string, MediaStream> = new Map();
const NONE_HELD: ReadonlySet<string> = new Set();

/** One screen in the call and whose it is, before it is shaped for the stage. */
export interface ShareSource {
  /**
   * Names one concrete stream, so opting in to watch it never carries over to
   * the next stream or owner a reconnect brings.
   */
  key: string;
  ownerId: string;
  /** Null for this user's own screen. */
  remote: VoicePeer | null;
}

/** The key of a remote peer's current screen, or null while it shares none. */
export function remoteShareKey(peer: VoicePeer): string | null {
  return peer.screen === null ? null : `remote:${peer.userId}:${peer.screen.streamId}`;
}

/**
 * Whether two lists name the same screens of the same people, so the older
 * one can be kept: speaking levels replace the peers many times a second.
 */
export function sameSources(a: readonly ShareSource[], b: readonly ShareSource[]): boolean {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  return a.every((source, index) => {
    const other = b[index];
    return (
      other !== undefined &&
      source.key === other.key &&
      source.remote?.screen === other.remote?.screen &&
      source.remote?.displayName === other.remote?.displayName &&
      source.remote?.undecryptable?.video === other.remote?.undecryptable?.video
    );
  });
}

/** Every screen in the call: this user's own first, then the others in call order. */
export function shareSourcesOf(state: VoiceState, userId: string | null): ShareSource[] {
  const sources: ShareSource[] = [];
  const self = userId ?? "self";
  if (state.localScreen !== null) {
    sources.push({ key: `local:${self}:${state.localScreen.streamId}`, ownerId: self, remote: null });
  }
  for (const peer of state.peers) {
    if (peer.self) continue;
    const key = remoteShareKey(peer);
    if (key !== null) sources.push({ key, ownerId: peer.userId, remote: peer });
  }
  return sources;
}

/** Whether two call summaries draw the same dock, so the older one can be kept. */
export function sameCall(a: CallVM | null, b: CallVM | null): boolean {
  if (a === b) return true;
  if (a === null || b === null) return false;
  return (
    a.channelId === b.channelId &&
    a.channelName === b.channelName &&
    a.privateCall === b.privateCall &&
    a.encrypted === b.encrypted &&
    a.startedAt === b.startedAt &&
    a.muted === b.muted &&
    a.deafened === b.deafened &&
    a.cameraOn === b.cameraOn &&
    a.cameraPending === b.cameraPending &&
    a.sharing === b.sharing &&
    a.latencyMs === b.latencyMs &&
    a.connectionQuality === b.connectionQuality &&
    a.headline === b.headline
  );
}

/** The dock's headline: a live screen outranks transport status. */
export function callHeadline(state: VoiceState): CallVM["headline"] {
  const { connection, transport } = state;
  if (connection === "connected" && state.localScreen !== null) return "screen";
  if (connection === "requesting-mic") return "requesting-mic";
  if (connection === "failed" || transport.media === "failed") return "failed";
  if (
    connection === "reconnecting" ||
    transport.signaling === "reconnecting" ||
    transport.signaling === "disconnected"
  ) {
    return "reconnecting";
  }
  if (transport.media === "degraded") return "degraded";
  if (connection === "connecting" || transport.media === "connecting") return "connecting";
  return "connected";
}

function speakingApplier(peers: readonly VoicePeer[]) {
  const byId = new Map(peers.map((peer) => [peer.userId, peer]));
  return (people: PersonVM[]): PersonVM[] =>
    people.map((person) => {
      const peer = byId.get(person.id);
      return peer === undefined
        ? person
        : {
            ...person,
            speaking: peer.speaking,
            muted: peer.muted,
            ...(peer.undecryptable === undefined ? {} : { undecryptable: peer.undecryptable }),
          };
    });
}

/** Remote participants only: the sender's own strip has no receiver-side setting. */
function remoteSettings<T>(
  peers: readonly VoicePeer[],
  pick: (peer: VoicePeer) => T,
): ReadonlyMap<string, T> {
  return new Map(peers.filter((peer) => !peer.self).map((peer) => [peer.userId, pick(peer)]));
}

function cameraStreamsOf(
  state: VoiceState,
  userId: string | null,
): ReadonlyMap<string, MediaStream> {
  // A held picture is no picture: its last frame would look frozen. Nor is
  // one that does not decrypt, of which nothing is played.
  const cameras = state.peers.flatMap((peer) =>
    peer.camera?.stream && !peer.camera.heldBack && !peer.undecryptable?.video
      ? [[peer.userId, peer.camera.stream] as const]
      : [],
  );
  if (state.localCamera?.stream && userId !== null) {
    cameras.push([userId, state.localCamera.stream]);
  }
  return cameras.length > 0 ? new Map(cameras) : NO_CAMERAS;
}

/** Whose cameras the media server holds back because this side's downlink has no room. */
function heldCamerasOf(state: VoiceState): ReadonlySet<string> {
  const held = state.peers.filter((peer) => !peer.self && peer.camera?.heldBack === true);
  return held.length > 0 ? new Set(held.map((peer) => peer.userId)) : NONE_HELD;
}

export interface VoiceViewInput {
  state: VoiceState;
  userId: string | null;
  displayName: string;
  /** Epoch ms the holder saw the call begin; null exactly when there is no call. */
  startedAt: number | null;
  deafened: boolean;
  /** The caller's presentation of the joined channel, when it gave one. */
  room: VoiceRoomPresentation | null;
  /** What the dock calls the joined channel. */
  roomName: string | null;
  /** The key of the one remote share the viewer opted in to, or null. */
  watchedKey: string | null;
  /**
   * The watched screen has not shown a picture since the opt-in: one held
   * back now is the first on its way, not a link that ran out of room.
   */
  watchFresh: boolean;
  occupantsOf: (channelId: string) => PersonVM[];
  sources: readonly ShareSource[];
}

export interface VoiceView {
  call: CallVM | null;
  people: PersonVM[];
  applySpeaking: (people: PersonVM[]) => PersonVM[];
  shares: ScreenShareVM[];
  /** Who watches each screen, by its key: the people in the call who said so. */
  viewers: ReadonlyMap<string, readonly PersonVM[]>;
  privateCall: boolean;
  cameraStreams: ReadonlyMap<string, MediaStream>;
  /** People whose camera is on but held back for this side's downlink. */
  heldCameras: ReadonlySet<string>;
  participantVolumes: ReadonlyMap<string, number>;
  participantGates: ReadonlyMap<string, boolean>;
  participantSuppressions: ReadonlyMap<string, boolean>;
}

/** Everything the call UI reads, derived from one RTC snapshot. */
export function deriveVoiceView(input: VoiceViewInput): VoiceView {
  const { state, room, userId } = input;
  const { channelId } = state;
  const privateCall = channelId !== null && room?.privateCall === true;
  const applySpeaking = speakingApplier(state.peers);

  const people = derivePeople(input, applySpeaking);
  return {
    call: deriveCall(input, privateCall),
    people,
    applySpeaking,
    shares: deriveShares(input),
    viewers: deriveViewers(input, people),
    privateCall,
    cameraStreams: cameraStreamsOf(state, userId),
    heldCameras: heldCamerasOf(state),
    participantVolumes: remoteSettings(state.peers, (peer) => peer.playbackVolume),
    participantGates: remoteSettings(state.peers, (peer) => peer.gated),
    participantSuppressions: remoteSettings(state.peers, (peer) => peer.suppressed),
  };
}

function deriveCall(input: VoiceViewInput, privateCall: boolean): CallVM | null {
  const { state, startedAt, roomName } = input;
  if (state.channelId === null || startedAt === null || roomName === null) return null;
  if (state.connection === "idle") return null;

  const rttMs = state.diagnostics.rttMs;
  return {
    channelId: state.channelId,
    channelName: roomName,
    ...(privateCall ? { privateCall: true } : {}),
    ...(state.encrypted ? { encrypted: true } : {}),
    startedAt,
    muted: state.muted,
    deafened: input.deafened,
    cameraOn: state.localCamera !== null,
    cameraPending: state.cameraPending,
    sharing: state.localScreen !== null,
    latencyMs: rttMs === null ? null : Math.max(0, Math.round(rttMs)),
    connectionQuality: state.diagnostics.quality,
    headline: callHeadline(state),
  };
}

// The directory knows who the server placed on the channel; the RTC layer knows
// who actually negotiated. Neither is a superset of the other during a join, so
// the stage shows the union.
function derivePeople(
  input: VoiceViewInput,
  applySpeaking: (people: PersonVM[]) => PersonVM[],
): PersonVM[] {
  const { state, room } = input;
  const { channelId } = state;
  const presented = room?.people ?? [];
  const listed =
    channelId === null
      ? []
      : applySpeaking(presented.length > 0 ? [...presented] : input.occupantsOf(channelId));

  const known = new Set(listed.map((person) => person.id));
  const negotiated = state.peers
    .filter((peer) => !known.has(peer.userId))
    .map(
      (peer): PersonVM => ({
        id: peer.userId,
        displayName: peer.displayName,
        initials: initials(peer.displayName),
        presence: "online",
        speaking: peer.speaking,
        muted: peer.muted,
        ...(peer.undecryptable === undefined ? {} : { undecryptable: peer.undecryptable }),
      }),
    );
  return [...listed, ...negotiated];
}

const NO_VIEWERS: ReadonlyMap<string, readonly PersonVM[]> = new Map();

/**
 * Who watches which screen. Each person in the call says which screen they
 * watch in their own state; this side knows its own choice. Your own screen
 * has no "you" among its viewers.
 */
function deriveViewers(input: VoiceViewInput, people: readonly PersonVM[]): ReadonlyMap<string, readonly PersonVM[]> {
  const { state, sources, userId } = input;
  if (sources.length === 0) return NO_VIEWERS;
  const byId = new Map(people.map((person) => [person.id, person]));
  const personOf = (id: string, name: string): PersonVM =>
    byId.get(id) ?? { id, displayName: name, initials: initials(name), presence: "online" };

  const viewers = new Map<string, PersonVM[]>();
  for (const source of sources) {
    const screen =
      source.remote === null
        ? state.localScreen === null
          ? null
          : { userId: source.ownerId, streamId: state.localScreen.streamId }
        : source.remote.screen === null
          ? null
          : { userId: source.remote.userId, streamId: source.remote.screen.streamId };
    if (screen === null) continue;
    const watching = state.peers
      .filter(
        (peer) =>
          !peer.self &&
          peer.watching?.userId === screen.userId &&
          peer.watching.streamId === screen.streamId,
      )
      .map((peer) => personOf(peer.userId, peer.displayName));
    if (source.remote !== null && input.watchedKey === source.key && userId !== null) {
      watching.unshift(personOf(userId, input.displayName));
    }
    if (watching.length > 0) viewers.set(source.key, watching);
  }
  return viewers.size > 0 ? viewers : NO_VIEWERS;
}

function deriveShares(input: VoiceViewInput): ScreenShareVM[] {
  const { state } = input;
  return input.sources.flatMap((source): ScreenShareVM[] => {
    if (source.remote === null) {
      if (state.localScreen === null) return [];
      return [
        {
          key: source.key,
          ownerId: source.ownerId,
          ownerName: input.displayName,
          local: true,
          hasAudio: state.localScreen.hasAudio,
          // The capture already belongs to this tab. Previewing it muted costs
          // no second capture and shows the sharer what is really being sent.
          stream: state.localScreen.stream,
          watching: false,
        },
      ];
    }
    const { remote } = source;
    if (remote.screen === null) return [];
    const watching = input.watchedKey === source.key;
    // A picture that does not decrypt is no picture: nothing of it is played.
    const sealed = watching && remote.undecryptable?.video === true;
    const held = watching && !sealed && remote.screen.heldBack === true;
    // Right after the opt-in the media server may hold the first frames back
    // while it makes room; that is the picture on its way, not a weak link.
    const heldBack = held && !input.watchFresh;
    const stream = watching && !sealed && !held ? remote.screen.stream : null;
    const connecting = watching && !sealed && !heldBack && stream === null;
    return [
      {
        key: source.key,
        ownerId: remote.userId,
        ownerName: remote.displayName,
        local: false,
        hasAudio: remote.screen.hasAudio,
        stream,
        watching,
        ...(connecting ? { connecting } : {}),
        ...(heldBack ? { heldBack } : {}),
        ...(sealed ? { sealed } : {}),
      },
    ];
  });
}
