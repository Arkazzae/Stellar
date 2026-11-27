import type { Translator } from "@/i18n";
import type { ChannelId, UserId, VoiceOccupant } from "@/lib/api/types";
import type {
  MediaSessionClosedEvent,
  MediaSignal,
  RealtimeClient,
} from "@/lib/realtime/types";
import {
  decodeMediaServerAnswer,
  decodeMediaServerOffer,
  decodeMediaTracks,
  encodeBrowserAnswer,
  encodeBrowserOffer,
  encodeMediaCandidate,
  encodeMediaSubscriptions,
  RTC_PROTOCOL_VERSION,
  type BrowserMediaSignalKind,
  type MediaServerSignalKind,
} from "@/lib/rtc/protocol";
import type { VoiceIceProvider } from "./iceSession";
import type { ParticipantPreferencesStore } from "../participantPreferences";
import type { VoiceSettings } from "../settings";
import type { VoiceReceivePreferences } from "../types";
import type { VoiceStateStore } from "../store";
import { mediaServerLostError } from "./webrtcErrors";
import { expectFreshMedia, readLink } from "./webrtcHealth";
import type { LocalCamera, LocalScreen } from "../video/webrtcLocalVideo";
import type { LocalMicrophone } from "../capture/webrtcMicrophone";
import {
  createParticipant,
  type Participant,
  type ParticipantMediaEnv,
} from "./webrtcParticipant";
import {
  applyEncoding,
  createLink,
  Negotiator,
  type AudioSectionUse,
  type Link,
  type LinkRecovery,
} from "./webrtcPeer";
import type { RemoteAudio } from "./webrtcRemoteAudio";
import { TrackDirectory } from "./webrtcSfuDirectory";
import { SfuHealth } from "./webrtcSfuHealth";
import type { CallTopology, StatsRound, TransportReport } from "./webrtcTopology";
import type { CallCrypto } from "../e2ee/callCrypto";

const DEFAULT_RECOVERY_GRACE_MS = 10_000;
/**
 * 2 s, 4 s, then 8 s, each stretched by up to half again at random. A restart
 * of the media server closes every session on the instance at once, and each
 * join costs the media server's single task time that grows with the room, so
 * the browsers must not all come back in the same instant. The API polls a
 * media server that went away every 2 s, so the three attempts span a restart
 * of the media server process rather than being spent before the API has
 * noticed it is back.
 */
const DEFAULT_REPLACEMENT_PAUSE_MS = 2_000;
/** Replacements in a row before the call is given up; a replacement that proves media resets it. */
const MAX_REPLACEMENTS = 3;
/** The steady round of statistics comes every 3 s; a microphone is late, not missing, until then. */
const DEFAULT_ARRIVAL_GRACE_MS = 3_000;
/** The list a media session starts with: no section is forwarded until it is asked for. */
const NO_SUBSCRIPTIONS = encodeMediaSubscriptions([]);
/**
 * A list goes out at most this often. What it holds follows the others'
 * state frames, which the hub lets one person send far faster than it lets
 * this side forward signals to the media server; somebody whose state keeps
 * changing must not use up that allowance and cost everyone their session.
 */
const SUBSCRIPTIONS_PACE_MS = 250;

export interface SfuDeps {
  readonly selfUserId: UserId;
  /** Words what goes wrong, in the language the person has now. */
  readonly translate: Translator;
  readonly store: VoiceStateStore;
  readonly realtime: Pick<RealtimeClient, "openMediaSession" | "sendMediaSignal" | "getStatus">;
  readonly ice: VoiceIceProvider;
  readonly preferences: ParticipantPreferencesStore;
  readonly remoteAudio: RemoteAudio;
  /** Files a person's tracks: their sinks, streams and tiles. */
  readonly personMedia: ParticipantMediaEnv;
  readonly microphone: LocalMicrophone;
  readonly camera: LocalCamera;
  readonly screen: LocalScreen;
  /** Grace before a `disconnected` connection is replaced. */
  recoveryGraceMs?: number;
  /** Pause before the first replacement in a row; each further one waits twice as long. */
  replacementPauseMs?: number;
  /** A number in [0, 1) that spreads the replacement pauses; `Math.random` unless a test sets it. */
  random?: () => number;
  /** How long somebody's microphone may take to arrive before the call counts as degraded. */
  arrivalGraceMs?: number;
  settings(): VoiceSettings;
  /** The send ceiling the adaptive controller currently allows. */
  bitrate(): number;
  /** Whether the shared sound being sent has two channels. */
  screenAudioStereo(): boolean;
  /** The room this side is seated in, or null between calls. */
  channelId(): ChannelId | null;
  /** Tell one person where this side stands, which they may have missed. */
  announceTo(userId: UserId): void;
  /** The session in progress, or null between calls. */
  activeSession(): number | null;
  /** What a person publishes to the UI has changed. */
  changed(): void;
  /** What `transport()` reports may have changed. */
  transportChanged(): void;
  /** Prove what flows now rather than at the next round of statistics. */
  probe(): void;
  /** The id of the stream this side's microphone and camera are sent in. */
  voiceStreamId(): string | null;
  /**
   * The frame encryption of the call, or null for a plain call. Read for
   * every connection, so a replacement keeps the call's keys, and a call
   * that turns encrypted (`renew`) gets them on its next one.
   */
  crypto(): CallCrypto | null;
  /**
   * Whether nothing of `person` may be asked for or played yet: in a private
   * call this side does not encrypt, somebody whose frames could be
   * ciphertext, which would play as noise.
   */
  holds(person: Participant): boolean;
}

/** One media session of the seat, and the one connection that belongs to it. */
interface MediaSessionLink {
  readonly id: string;
  readonly channelId: ChannelId;
  readonly link: Link;
  readonly directory: TrackDirectory;
  /**
   * Every step of the negotiation, strictly in order: the media server's
   * signals as they arrived and this side's own offers.
   */
  chain: Promise<void>;
  /** Signals to the hub, one at a time, in the order they were made. */
  outbox: Promise<void>;
  /**
   * The list of sections wanted now, the one the outbox will send and the
   * last one handed to the hub, each encoded.
   */
  desiredSubscriptions: string;
  queuedSubscriptions: string;
  sentSubscriptions: string;
  /** Running for the pace after a list was put in the outbox; a change meanwhile waits for it. */
  subscriptionsPace: ReturnType<typeof setTimeout> | null;
  /** What this side publishes changed since its last offer. */
  wantsOffer: boolean;
  /** The number of the media server offer being answered, echoed in the answer. */
  answering: number | null;
  /** When the connection last came up, or null while it is not up. */
  upAt: number | null;
  closed: boolean;
}

/**
 * How a call is connected: one connection to Stellarr Media, which forwards
 * everybody else's tracks on it (the "Client rules" of
 * docs/stellarr-media.md).
 *
 * The connection belongs to a media session that the hub opens for this
 * seat. The browser offers what it publishes and answers what the media
 * server offers; it is the impolite side, so a media server offer that meets
 * one of its own is ignored, and the media server offers again. A directory
 * that comes with the media server's signals says whose track arrives on
 * which section (`webrtcSfuDirectory`). A connection that dies is not
 * repaired but replaced by a new session, at most three times in a row.
 */
export class Sfu implements CallTopology {
  private session: MediaSessionLink | null = null;
  private readonly negotiator: Negotiator;
  private readonly health: SfuHealth;
  private readonly recovery: LinkRecovery;
  /** Bumped whenever what is being opened stops mattering: a leave, a move, a newer attempt. */
  private generation = 0;
  /** Replacements in a row without proven media. */
  private attempts = 0;
  private exhausted = false;
  /** A connection was lost while the hub was away; the rejoin opens the next one. */
  private waitingForHub = false;
  private replacementTimer: ReturnType<typeof setTimeout> | null = null;
  private receivePreferences: VoiceReceivePreferences = { screen: null, cameras: true };

  constructor(
    /** Everyone else in the call, shared with the client that shows and plays them. */
    private readonly people: Map<UserId, Participant>,
    private readonly deps: SfuDeps,
  ) {
    this.negotiator = new Negotiator({
      settings: () => deps.settings(),
      bitrate: () => deps.bitrate(),
      screenAudioStereo: () => deps.screenAudioStereo(),
      send: (link, type, sdp) => this.sendDescription(link, type, sdp),
      audioUse: (link, section) => this.audioUse(link, section),
    });
    this.health = new SfuHealth({
      translate: deps.translate,
      store: deps.store,
      people,
      arrivalGraceMs: deps.arrivalGraceMs ?? DEFAULT_ARRIVAL_GRACE_MS,
    });
    this.recovery = {
      graceMs: deps.recoveryGraceMs ?? DEFAULT_RECOVERY_GRACE_MS,
      // The watchdog only says when; the replacement is the whole repair.
      maxAttempts: 1,
      onEscalate: (link) => {
        // After the watchdog has scheduled what follows, so closing the link
        // cancels that too.
        void Promise.resolve().then(() => {
          if (this.session?.link === link) this.replace();
        });
      },
      onExhausted: () => undefined,
    };
  }

  // ------------------------------------------------------------- the seat

  /** Every confirmed seat gets a session of its own and a fresh budget. */
  async seated(): Promise<void> {
    this.cancelReplacement();
    this.attempts = 0;
    this.exhausted = false;
    this.waitingForHub = false;
    await this.open();
  }

  async reconcile(participants: readonly VoiceOccupant[]): Promise<void> {
    const seen = new Set<UserId>();
    const arrived: UserId[] = [];

    for (const participant of participants) {
      if (participant.userId === this.deps.selfUserId) continue;
      seen.add(participant.userId);
      const existing = this.people.get(participant.userId);
      if (existing) {
        existing.displayName = participant.displayName;
        // The hub says what a seat was given, which on a rejoin is unmuted;
        // once the person has said otherwise themselves, their word stands.
        if (!existing.stateKnown) existing.muted = participant.muted;
        continue;
      }
      this.people.set(
        participant.userId,
        createParticipant({
          user: participant,
          // How this person sounded last time is how they start this time.
          preference: this.deps.preferences.get(participant.userId),
          createVoice: () => this.deps.remoteAudio.createVoice(),
        }),
      );
      arrived.push(participant.userId);
    }

    const left: Participant[] = [];
    for (const [userId, person] of [...this.people]) {
      if (seen.has(userId)) continue;
      this.people.delete(userId);
      left.push(person);
    }

    // Whoever just arrived gets what was held for them, and whoever left
    // stops playing, before their sinks go.
    this.session?.directory.place();
    this.refreshReceivePreferences();
    for (const person of left) this.deps.remoteAudio.release(person);
    // A newcomer cannot tell this side's camera from its screen without it.
    for (const userId of arrived) this.deps.announceTo(userId);

    this.deps.transportChanged();
    this.deps.changed();
    // Their microphone is on its way; say so as soon as it arrives.
    if (arrived.length > 0) this.deps.probe();
  }

  handleMediaSignal(signal: MediaSignal<MediaServerSignalKind>): Promise<void> {
    const session = this.session;
    // A signal of an earlier session, or of the room before a move, is stale.
    if (!session || signal.sessionId !== session.id || signal.channelId !== session.channelId) {
      return Promise.resolve();
    }
    return this.enqueue(session, () => this.apply(session, signal));
  }

  mediaSessionClosed(event: MediaSessionClosedEvent): void {
    const session = this.session;
    if (!session || event.sessionId !== session.id || event.channelId !== session.channelId) return;
    this.replace();
  }

  /**
   * The call changed how it is protected: replace the connection now, with a
   * fresh budget of replacements, so the next one is made the new way.
   */
  async renew(): Promise<void> {
    if (this.deps.activeSession() === null) return;
    this.cancelReplacement();
    this.attempts = 0;
    this.exhausted = false;
    this.waitingForHub = false;
    await this.open();
  }

  /**
   * Whose sound or picture this side cannot open now: a section of theirs
   * whose frames stopped opening, or everything of somebody who encrypts
   * while this side does not.
   */
  undecryptable(): Map<UserId, { audio: boolean; video: boolean }> {
    const out = new Map<UserId, { audio: boolean; video: boolean }>();
    const mark = (userId: UserId, kind: "audio" | "video") => {
      const entry = out.get(userId) ?? { audio: false, video: false };
      entry[kind] = true;
      out.set(userId, entry);
    };
    const session = this.session;
    const crypto = session?.link.crypto ?? null;
    if (session && crypto) {
      const receivers = new Map(
        session.link.connection.getTransceivers().map((transceiver) => [transceiver.mid, transceiver.receiver]),
      );
      for (const { mid, userId, kind } of session.directory.filedSections()) {
        const receiver = receivers.get(mid);
        if (receiver && crypto.failing(receiver)) mark(userId, kind);
      }
    }
    for (const person of this.people.values()) {
      // Somebody who says they encrypt, while this side does not.
      if (this.deps.crypto() !== null || !person.encrypted) continue;
      mark(person.userId, "audio");
      if (person.media.camera !== null || person.media.screen !== null) mark(person.userId, "video");
    }
    return out;
  }

  // ------------------------------------------------------- what is published

  renegotiate(): Promise<void> {
    const session = this.session;
    return session ? this.requestOffer(session) : Promise.resolve();
  }

  async applyMicEncoding(): Promise<void> {
    const sender = this.session?.link.micSender;
    if (sender) await applyEncoding(sender, this.deps.bitrate());
  }

  setReceivePreferences(preferences: VoiceReceivePreferences): void {
    this.receivePreferences = {
      screen: preferences.screen === null ? null : { ...preferences.screen },
      cameras: preferences.cameras,
    };
    this.deps.remoteAudio.setWatchedScreen(this.receivePreferences.screen);
    this.refreshReceivePreferences();
  }

  refreshReceivePreferences(): void {
    const session = this.session;
    if (session) this.syncSubscriptions(session);
  }

  links(): Link[] {
    return this.session ? [this.session.link] : [];
  }

  // ------------------------------------------------------------- recovery

  dropAll(): void {
    this.generation += 1;
    this.cancelReplacement();
    this.closeSession();
    this.attempts = 0;
    this.exhausted = false;
    this.waitingForHub = false;
    for (const [userId, person] of [...this.people]) {
      this.people.delete(userId);
      this.deps.remoteAudio.release(person);
    }
    this.deps.transportChanged();
  }

  /**
   * Only a connection that is down is replaced. A tab that comes back to a
   * working one leaves it alone; rebuilding it would cause the very glitch
   * this is meant to avoid.
   */
  async repairUnhealthy(): Promise<void> {
    const state = this.session?.link.connection.connectionState;
    if (state === "failed" || state === "disconnected") this.replace();
  }

  async rebuild(link: Link): Promise<void> {
    if (this.session?.link === link) this.replace();
  }

  transport(): TransportReport {
    return this.health.transport(this.condition());
  }

  async sample(): Promise<StatsRound> {
    const session = this.session;
    if (!session) {
      return {
        readings: [],
        settle: () => {
          this.deps.transportChanged();
          this.health.report(this.condition());
        },
      };
    }
    const reading = await readLink(session.link);
    return {
      readings: [reading],
      settle: (selfMuted) => {
        if (this.session !== session) return;
        this.health.settle(session.link, reading, selfMuted, (person) =>
          session.directory.microphoneMids(person),
        );
        // A connection that proves media ends the run of replacements.
        if (session.link.flow.verified) this.attempts = 0;
        this.deps.transportChanged();
        this.health.report(this.condition());
      },
    };
  }

  // -------------------------------------------------------------- sessions

  /**
   * Open a media session for the seat and build its connection: the bootstrap
   * ICE servers, BUNDLE, everything this side publishes, then the offer.
   * Opening a session always means a new connection; the hub closes the
   * session this connection had before.
   */
  private async open(): Promise<void> {
    const channelId = this.deps.channelId();
    if (channelId === null || this.deps.activeSession() === null) return;
    const generation = ++this.generation;
    this.closeSession();
    this.deps.transportChanged();

    let sessionId: string;
    try {
      sessionId = await this.deps.realtime.openMediaSession(channelId);
    } catch {
      // The media server could not be reached, or the seat moved on.
      if (generation === this.generation) this.replace();
      return;
    }
    if (generation !== this.generation || this.deps.channelId() !== channelId) return;

    let session: MediaSessionLink;
    try {
      session = this.build(sessionId, channelId);
    } catch {
      // The browser would not even make the connection; another attempt is
      // all there is to try.
      this.replace();
      return;
    }
    this.deps.transportChanged();
    await this.requestOffer(session);
  }

  private build(id: string, channelId: ChannelId): MediaSessionLink {
    const link = createLink({
      // Read now, not at join: a replacement late in a long call must not
      // reach for credentials that have expired.
      iceServers: this.deps.ice.current(),
      recovery: this.recovery,
      crypto: this.deps.crypto(),
    });
    const session: MediaSessionLink = {
      id,
      channelId,
      link,
      directory: new TrackDirectory({
        people: this.people,
        personMedia: this.deps.personMedia,
        remoteAudio: this.deps.remoteAudio,
      }),
      chain: Promise.resolve(),
      outbox: Promise.resolve(),
      // A new media session forwards nothing until it is asked to, so the
      // empty list is what the media server already has.
      desiredSubscriptions: NO_SUBSCRIPTIONS,
      queuedSubscriptions: NO_SUBSCRIPTIONS,
      sentSubscriptions: NO_SUBSCRIPTIONS,
      subscriptionsPace: null,
      wantsOffer: false,
      answering: null,
      upAt: null,
      closed: false,
    };
    this.session = session;

    const { connection } = link;
    connection.ontrack = (event) => {
      if (this.session !== session) return;
      const mid = event.transceiver?.mid ?? null;
      // The directory comes with the offer, ahead of its tracks, and names
      // whose frames the receiver opens; one it does not name opens nothing.
      link.crypto?.attachReceiver(
        event.receiver,
        event.track.kind === "video" ? "video" : "audio",
        mid,
        mid === null ? null : session.directory.publisherOf(mid),
      );
      session.directory.arrive(mid, event.track);
      this.syncSubscriptions(session);
    };
    connection.onicecandidate = (event) => {
      void this.post(
        session,
        "candidate",
        encodeMediaCandidate(event.candidate ? event.candidate.toJSON() : null),
        false,
      );
    };
    connection.onnegotiationneeded = () => {
      if (this.session === session) void this.requestOffer(session);
    };
    connection.onconnectionstatechange = () => this.connectionChanged(session);

    // The captures outlive any one connection; a new one carries them all.
    this.deps.microphone.addTo(link);
    this.negotiator.tuneMicSender(link);
    this.deps.camera.addTo(link);
    this.deps.screen.addTo(link);
    if (!link.micSender && !link.cameraSender && !link.screenVideoSender) {
      // An offer must have a section, and the media server offers nothing
      // before it has answered one.
      connection.addTransceiver("audio", { direction: "recvonly" });
    }
    return session;
  }

  /** Close the current connection. People stay; only what arrived on it goes. */
  private closeSession(): void {
    const session = this.session;
    if (!session) return;
    this.session = null;
    session.closed = true;
    if (session.subscriptionsPace !== null) clearTimeout(session.subscriptionsPace);
    session.subscriptionsPace = null;
    session.directory.clear();
    this.health.forget();

    const { link } = session;
    link.crypto?.forgetTransforms();
    link.watchdog.dispose();
    const { connection } = link;
    connection.ontrack = null;
    connection.onicecandidate = null;
    connection.onnegotiationneeded = null;
    connection.onconnectionstatechange = null;
    try {
      connection.close();
    } catch {
      // Already closed.
    }
  }

  /**
   * The connection is lost: close it now, so nothing of it plays, and open a
   * new session after a pause that doubles each time. The captures stay open.
   */
  private replace(): void {
    if (this.deps.activeSession() === null) return;
    this.generation += 1;
    this.closeSession();

    if (this.replacementTimer === null && !this.exhausted) {
      if (this.deps.realtime.getStatus() !== "connected") {
        // The hub is away, and its reconnect ends the seat and the session
        // anyway; the rejoin opens the next one without spending the budget.
        this.waitingForHub = true;
      } else if (this.attempts >= MAX_REPLACEMENTS) {
        this.fail();
      } else {
        this.attempts += 1;
        const pause =
          (this.deps.replacementPauseMs ?? DEFAULT_REPLACEMENT_PAUSE_MS) *
          2 ** (this.attempts - 1) *
          (1 + (this.deps.random ?? Math.random)() / 2);
        this.replacementTimer = setTimeout(() => {
          this.replacementTimer = null;
          void this.open();
        }, pause);
      }
    }
    this.deps.transportChanged();
  }

  private fail(): void {
    this.exhausted = true;
    this.deps.store.set({ error: mediaServerLostError(this.deps.translate) });
  }

  private cancelReplacement(): void {
    if (this.replacementTimer === null) return;
    clearTimeout(this.replacementTimer);
    this.replacementTimer = null;
  }

  private connectionChanged(session: MediaSessionLink): void {
    if (this.session !== session) return;
    const { link } = session;
    const state = link.connection.connectionState;

    if (state === "connected") {
      link.watchdog.disarm();
      session.upAt = Date.now();
      this.deps.probe();
    } else if (state === "disconnected") {
      session.upAt = null;
      // Transient by design: a Wi-Fi handover produces this routinely and ICE
      // usually recovers unaided. The watchdog replaces it after the grace.
      expectFreshMedia(link.flow);
      link.watchdog.arm();
    } else if (state === "failed" || state === "closed") {
      this.replace();
      return;
    }
    this.deps.transportChanged();
  }

  private condition() {
    return {
      link: this.session?.link ?? null,
      recovering: this.attempts > 0 || this.replacementTimer !== null || this.waitingForHub,
      exhausted: this.exhausted,
      upAt: this.session?.upAt ?? null,
    };
  }

  // ----------------------------------------------------------- negotiation

  /** Run `step` after everything already queued on the session's negotiation. */
  private enqueue(session: MediaSessionLink, step: () => Promise<void>): Promise<void> {
    const run = session.chain.then(() => (session.closed ? undefined : step()));
    session.chain = run.catch(() => undefined);
    return session.chain;
  }

  /**
   * Ask for an offer of what this side publishes. Requests made while an
   * offer is unanswered are not lost: the offer is made once the answer lands.
   */
  private requestOffer(session: MediaSessionLink): Promise<void> {
    session.wantsOffer = true;
    return this.enqueue(session, () => this.offerIfWanted(session));
  }

  private async offerIfWanted(session: MediaSessionLink): Promise<void> {
    const { link } = session;
    if (!session.wantsOffer || link.connection.signalingState !== "stable") return;
    session.wantsOffer = false;
    await this.negotiator.offer(link);
    // `offer` never rejects. A connection that could not even describe itself
    // would never publish again.
    if (!session.closed && link.connection.signalingState === "stable") this.lose(session);
  }

  private async apply(
    session: MediaSessionLink,
    signal: MediaSignal<MediaServerSignalKind>,
  ): Promise<void> {
    const { link } = session;
    const { connection } = link;

    if (signal.kind === "offer") {
      const offer = decodeMediaServerOffer(signal.payload);
      if (!offer) return;
      // Impolite: an offer that meets one of ours being made or unanswered is
      // ignored, directory and all. The media server rolls it back, answers
      // ours and offers again.
      if (link.makingOffer || connection.signalingState !== "stable") return;
      // In hand before the description is applied, so every track the offer
      // brings finds its section named, and the list that asks for them
      // leaves ahead of the answer that makes them live.
      session.directory.update(offer.tracks);
      this.bindReceivers(session);
      this.syncSubscriptions(session);
      session.answering = offer.negotiationId;
      try {
        await this.negotiator.receive(link, { type: "offer", sdp: offer.sdp });
      } catch {
        // A description the browser refuses leaves the two sides apart for
        // good. Waiting for the media server's third strike only adds silence.
        this.lose(session);
        return;
      } finally {
        session.answering = null;
      }
    } else if (signal.kind === "answer") {
      const answer = decodeMediaServerAnswer(signal.payload);
      // Nothing of ours is waiting for it: a late answer.
      if (!answer || connection.signalingState !== "have-local-offer") return;
      try {
        await this.negotiator.receive(link, answer);
      } catch {
        this.lose(session);
        return;
      }
      link.crypto?.noteCodecs();
    } else {
      const tracks = decodeMediaTracks(signal.payload);
      if (tracks) {
        session.directory.update(tracks);
        this.bindReceivers(session);
        this.syncSubscriptions(session);
      }
    }

    if (session.wantsOffer) await this.offerIfWanted(session);
  }

  /** The negotiator finished a description: hand it to the media server. */
  private async sendDescription(link: Link, type: "offer" | "answer", sdp: string): Promise<void> {
    const session = this.session;
    if (!session || session.link !== link) return;
    if (type === "offer") {
      await this.post(session, "offer", encodeBrowserOffer(sdp), true);
      return;
    }
    // Read before the first await: the number belongs to the offer being answered now.
    const negotiationId = session.answering;
    if (negotiationId === null) return;
    await this.post(session, "answer", encodeBrowserAnswer(sdp, negotiationId), true);
  }

  /**
   * Send one signal after every signal made before it. A description the hub
   * refuses leaves this side waiting for an answer that never comes, so that
   * session is replaced; a lost candidate costs nothing, since the media
   * server learns this side's address from its checks. A payload given as a
   * function is read when the signal's turn comes, and null sends nothing.
   */
  private async post(
    session: MediaSessionLink,
    kind: BrowserMediaSignalKind,
    payload: string | (() => string | null),
    vital: boolean,
  ): Promise<void> {
    const sent = session.outbox.then(() => {
      if (session.closed) return undefined;
      const body = typeof payload === "string" ? payload : payload();
      if (body === null) return undefined;
      const signal: MediaSignal<BrowserMediaSignalKind> = {
        protocolVersion: RTC_PROTOCOL_VERSION,
        channelId: session.channelId,
        sessionId: session.id,
        kind,
        payload: body,
      };
      return this.deps.realtime.sendMediaSignal(signal);
    });
    session.outbox = sent.catch(() => undefined);
    try {
      await sent;
    } catch {
      if (vital) this.lose(session);
    }
  }

  /**
   * Ask the media server for what this side wants now: every voice, the
   * cameras unless they are turned off, and the one share being watched.
   * What is not asked for stops playing here at once, before the list has
   * reached the media server.
   */
  private syncSubscriptions(session: MediaSessionLink): void {
    const mids = session.directory.subscribedMids(this.receivePreferences, (person) => this.held(person));
    session.directory.applySubscriptions(mids);
    let payload: string;
    try {
      payload = encodeMediaSubscriptions(mids);
    } catch {
      // Only a directory the media server never makes could name such a section.
      this.lose(session);
      return;
    }
    session.desiredSubscriptions = payload;
    this.flushSubscriptions(session);
  }

  /**
   * Put a list in the outbox, unless the wanted one is in it already or one
   * went in less than the pace ago; then one goes in when the pace is up. A
   * list replaces the last one whole, so the one in the outbox is read when
   * its turn comes: it is the list wanted by then, and nothing when that is
   * what the media server has already. It shares the outbox with the
   * descriptions, and a refused one replaces the session like they do.
   */
  private flushSubscriptions(session: MediaSessionLink): void {
    if (
      session.closed ||
      session.subscriptionsPace !== null ||
      session.desiredSubscriptions === session.queuedSubscriptions
    ) {
      return;
    }
    session.queuedSubscriptions = session.desiredSubscriptions;
    session.subscriptionsPace = setTimeout(() => {
      session.subscriptionsPace = null;
      this.flushSubscriptions(session);
    }, SUBSCRIPTIONS_PACE_MS);
    void this.post(
      session,
      "subscriptions",
      () => {
        const latest = session.desiredSubscriptions;
        // What goes now is what the outbox holds, so the pace does not send it again.
        session.queuedSubscriptions = latest;
        if (latest === session.sentSubscriptions) return null;
        session.sentSubscriptions = latest;
        return latest;
      },
      true,
    );
  }

  /**
   * Somebody whose frames this side may not be able to open: nothing of them
   * is asked for, so nothing of them plays (`SfuDeps.holds`).
   */
  private held(person: Participant): boolean {
    return this.deps.holds(person);
  }

  /** Receivers that arrived before the directory named their section learn whose they are. */
  private bindReceivers(session: MediaSessionLink): void {
    session.link.crypto?.bind((mid) => session.directory.publisherOf(mid));
  }

  /** Replace `session` if it is still the current one. */
  private lose(session: MediaSessionLink): void {
    if (this.session === session) this.replace();
  }

  /**
   * What an audio section carries, for the Opus parameters of this side's
   * descriptions: the media server repeats a publisher's own parameters back
   * to it, so they set this side's encoders, and the parameters of a section
   * it receives set its decoder.
   */
  private audioUse(
    link: Link,
    { mid, streamId }: { mid: string | null; streamId: string | null },
  ): AudioSectionUse | null {
    const session = this.session;
    if (!session || session.link !== link) return null;
    const received = mid === null ? null : session.directory.useOf(mid);
    if (received) return received;
    if (streamId === null) return null;
    if (streamId === this.deps.screen.streamId) return "shared-sound";
    if (streamId === this.deps.voiceStreamId()) return "speech";
    return null;
  }
}
