import type { ChannelId, UserId } from "@/lib/api/types";

import type { WorkletSuppressor } from "./audioProfiles";
import type { NetworkQuality } from "./rtc/bitrate";
import type { VoiceSettings } from "./settings";

export type VoiceConnectionState =
  | "idle"
  | "requesting-mic"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "failed";

export type VoiceSignalingState =
  | "idle"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "disconnected";

export type VoiceMediaState =
  | "idle"
  | "ready"
  | "connecting"
  | "connected"
  | "degraded"
  | "failed";

/**
 * Signaling and media health are tracked separately: signaling can reconnect
 * while SRTP keeps flowing, and a healthy hub says nothing about whether a
 * peer connection ever established media.
 */
export interface VoiceTransportState {
  signaling: VoiceSignalingState;
  media: VoiceMediaState;
  peerCount: number;
  connectedPeerCount: number;
}

export const IDLE_VOICE_TRANSPORT: VoiceTransportState = Object.freeze({
  signaling: "idle",
  media: "idle",
  peerCount: 0,
  connectedPeerCount: 0,
});

export type VoiceErrorKind =
  | "mic-denied"
  | "mic-unavailable"
  | "unsupported"
  | "ice-config"
  | "signaling"
  | "transport"
  | "session-replaced"
  | "channel-kicked"
  | "camera"
  | "screen-share"
  /** An encrypted conversation's call that cannot be made encrypted, so it is not made. */
  | "encryption";

export interface VoiceError {
  kind: VoiceErrorKind;
  message: string;
  /** True when the call is still usable and only a sub-action failed. */
  recoverable?: boolean;
}

export interface VoiceScreenShare {
  /** Stable for as long as the share lasts. */
  streamId: string;
  hasAudio: boolean;
  /** Playable when this client owns or watches the share. */
  stream: MediaStream | null;
  /**
   * A remote share this side watches whose picture the media server holds
   * back, because this side's downlink has no room for it now.
   */
  heldBack?: boolean;
}

export interface VoiceCamera {
  /** Stable for as long as this capture is active. */
  streamId: string;
  stream: MediaStream | null;
  /**
   * A remote camera whose picture the media server holds back, because this
   * side's downlink has no room for it now.
   */
  heldBack?: boolean;
}

export interface VoicePeer {
  userId: UserId;
  displayName: string;
  muted: boolean;
  speaking: boolean;
  /** Receiver-side volume for this participant, 0 to 200. */
  playbackVolume: number;
  /** Local only: this person's strip is muted between words. */
  gated: boolean;
  /** Local only: this person's strip is denoised before the gate. */
  suppressed: boolean;
  self: boolean;
  /** Null when the participant is not sharing a screen. */
  screen: VoiceScreenShare | null;
  /** Null when the participant's camera is off. */
  camera: VoiceCamera | null;
  /**
   * In an encrypted call: this person's sound or picture reaches this side
   * but does not decrypt, so nothing of it is played. Absent when all is well.
   */
  undecryptable?: { audio: boolean; video: boolean };
}

/** Coarse, non-alarming call health for the dock. */
export interface VoiceDiagnostics {
  quality: NetworkQuality;
  /** Current send ceiling in bits per second; null before the first sample. */
  bitrate: number | null;
  /** 0..1 */
  lossFraction: number | null;
  rttMs: number | null;
}

export const IDLE_DIAGNOSTICS: VoiceDiagnostics = Object.freeze({
  quality: "unknown",
  bitrate: null,
  lossFraction: null,
  rttMs: null,
});

export interface VoiceState {
  connection: VoiceConnectionState;
  transport: VoiceTransportState;
  channelId: ChannelId | null;
  muted: boolean;
  peers: VoicePeer[];
  error: VoiceError | null;
  /** The local share, when this user is the one sharing. */
  localScreen: VoiceScreenShare | null;
  /** True while `startScreenShare` waits on the picker. */
  screenSharePending: boolean;
  /** The local camera preview and published stream. */
  localCamera: VoiceCamera | null;
  /** True while permission and device capture are being resolved. */
  cameraPending: boolean;
  /** Browser capability, independent of whether a call is joined. */
  cameraSupported: boolean;
  /** True when RNNoise was requested and is actually running. */
  noiseSuppressionActive: boolean;
  /** Set when a requested processing mode silently degraded. */
  degraded: string | null;
  /**
   * The chosen suppressor's files on their way, once the download has run
   * long enough to be worth showing. Null otherwise, in a call or not.
   */
  suppressorDownload: SuppressorDownload | null;
  diagnostics: VoiceDiagnostics;
  /** This side encrypts and decrypts the call's frames end to end. */
  encrypted: boolean;
}

/** How far the chosen engine's download has come. */
export interface SuppressorDownload {
  kind: WorkletSuppressor;
  /**
   * Whole percent of the bytes received, never more than 100 and never lower
   * than it was; null while the total is unknown.
   */
  percent: number | null;
}

export const IDLE_VOICE_STATE: VoiceState = Object.freeze({
  connection: "idle",
  transport: IDLE_VOICE_TRANSPORT,
  channelId: null,
  muted: false,
  peers: [],
  error: null,
  localScreen: null,
  screenSharePending: false,
  localCamera: null,
  cameraPending: false,
  cameraSupported: false,
  noiseSuppressionActive: false,
  degraded: null,
  suppressorDownload: null,
  diagnostics: IDLE_DIAGNOSTICS,
  encrypted: false,
});

/** What this receiver wants; voices remain enabled independently of video preferences. */
export interface VoiceJoinOptions {
  /**
   * The room is a private conversation. Its call is decided from the
   * conversation's encryption before anything is opened: encrypted, plain,
   * or not at all.
   */
  private?: boolean;
}

export interface VoiceReceivePreferences {
  screen: { userId: UserId; streamId: string } | null;
  cameras: boolean;
}

/**
 * The only RTC surface the UI may touch. Signaling, the connection to the
 * media server and everything else of WebRTC sit behind it and can change
 * without touching a component. Snapshots are immutable and referentially stable between
 * changes, so `getSnapshot` and `subscribe` plug straight into
 * `useSyncExternalStore`.
 */
export interface VoiceClient {
  getSnapshot(): VoiceState;
  subscribe(listener: () => void): () => void;
  join(channelId: ChannelId, options?: VoiceJoinOptions): Promise<void>;
  leave(): Promise<void>;
  /**
   * Repair the call in progress without leaving it: the others never see this
   * person go. A call that has no seat joins again.
   */
  reconnect(): Promise<void>;
  /** Updates the local identity without reopening any call media. */
  setDisplayName(displayName: string): void;
  setMuted(muted: boolean): void;
  /** Silences or restores every remote voice and screen sink; capture is unaffected. */
  setDeafened(deafened: boolean): void;
  /** Select what the media server forwards; microphone speech is always received. */
  setReceivePreferences(preferences: VoiceReceivePreferences): void;
  /** Local only; never signalled to the peer. */
  setPeerVolume(userId: UserId, volume: number): void;
  /**
   * Gates one remote microphone strip so what that person sends between
   * words never reaches your ears. Local only; never signalled to the peer.
   */
  setPeerGate(userId: UserId, gated: boolean): void;
  /**
   * Runs one remote microphone strip through a local noise suppressor, for a
   * person whose own settings you cannot change. Local only; costs this
   * browser the CPU, for that one strip.
   */
  setPeerSuppression(userId: UserId, suppressed: boolean): void;
  /**
   * Applies changed settings to a live call. Replaces the outgoing track on
   * the connection when a capture-affecting setting changed, without dropping
   * the call. A newly chosen noise suppressor's files start downloading here,
   * in a call or not.
   */
  applySettings(settings: VoiceSettings): Promise<void>;
  /** Must be called synchronously from the click, or the user gesture is lost. */
  startScreenShare(): Promise<void>;
  stopScreenShare(): Promise<void>;
  startCamera(): Promise<void>;
  stopCamera(): Promise<void>;
  dispose(): void;
}
