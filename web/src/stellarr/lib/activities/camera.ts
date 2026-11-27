import type { ActivityCamera } from "../../../../../sdk/host/media-host.js";
import { sessionCalls } from "@/lib/api/calls/session";
import { iceServers } from "@/lib/platform/env";
import type { MediaSignal, RealtimeClient } from "@/lib/realtime/types";
import {
  decodeMediaServerAnswer, decodeMediaServerOffer, decodeMediaTracks, encodeBrowserAnswer,
  encodeBrowserOffer, encodeMediaCandidate, encodeMediaSubscriptions, RTC_PROTOCOL_VERSION,
  type BrowserMediaSignalKind, type MediaServerSignalKind, type MediaTrack,
} from "@/lib/rtc/protocol";
import { resolveIceServers } from "@/lib/voice/rtc/iceConfig";

const NO_SUBSCRIPTIONS = encodeMediaSubscriptions([]);

async function cameraIce(): Promise<RTCIceServer[]> {
  const { rtc } = await sessionCalls().bootstrap();
  return resolveIceServers({ runtime: rtc?.iceServers, env: iceServers(), isProduction: import.meta.env.PROD });
}

type Watch = { user: string; show: (stream: MediaStream | null) => void };
type Grant = { channelId: string; sessionId: string; grant: string };

interface Link {
  pc: RTCPeerConnection;
  grant: Grant;
  publisher: RTCRtpTransceiver;
  directory: MediaTrack[];
  streams: Map<string, MediaStream>;
  negotiation: Promise<void>;
  outbox: Promise<void>;
  subscriptions: string;
  subscriptionTimer?: ReturnType<typeof setTimeout>;
  needsOffer: boolean;
}

/** One WebRTC connection to the activity's SFU room, owned by the trusted page. */
export class ActivityCameraTransport implements ActivityCamera {
  private readonly watches = new Set<Watch>();
  private readonly off: (() => void)[] = [];
  private local: MediaStream | null = null;
  private ended?: () => void;
  private link: Link | null = null;
  private opening?: Promise<void>;
  private generation = 0;
  private disposed = false;
  private retry?: ReturnType<typeof setTimeout>;
  private attempts = 0;

  constructor(
    private readonly hub: RealtimeClient | null,
    private readonly self: string,
    private readonly place: string,
    private readonly lease: { install: string; access: string },
    private readonly getIce = cameraIce,
  ) {
    if (!hub) return;
    this.off.push(
      hub.on("MediaSignal", event => {
        const link = this.link;
        if (link && event.sessionId === link.grant.sessionId && event.channelId === link.grant.channelId)
          void this.enqueue(link, () => this.receive(link, event)).catch(() => {});
      }),
      hub.on("MediaSessionClosed", event => {
        if (event.sessionId === this.link?.grant.sessionId) this.failed();
      }),
      hub.subscribeStatus(() => {
        if (hub.getStatus() !== "connected") this.failed();
        else if (this.hasRemoteWatch()) { this.attempts = 0; this.view(); }
      }),
    );
  }

  private hasRemoteWatch() {
    return [...this.watches].some(watch => watch.user !== this.self);
  }

  watch(user: string, show: Watch["show"]) {
    const watch = { user, show };
    this.watches.add(watch);
    this.render();
    if (user !== this.self) this.view();
    return () => {
      this.watches.delete(watch);
      show(null);
      this.render();
    };
  }

  async publish(stream: MediaStream, signal: AbortSignal, ended: () => void) {
    if (signal.aborted || this.disposed) throw new Error("Camera stopped.");
    this.local = stream;
    this.ended = ended;
    signal.addEventListener("abort", () => {
      if (this.local !== stream) return;
      this.local = null;
      this.ended = undefined;
      this.render();
      const link = this.link;
      if (link) void this.enqueue(link, () => this.offer(link)).catch(() => {});
    }, { once: true });
    this.render();
    await this.connect();
    const link = this.link;
    if (!link || signal.aborted) throw new Error("Camera stopped.");
    await this.enqueue(link, () => this.offer(link));
  }

  private view() {
    if (this.retry === undefined && this.hub?.getStatus() === "connected")
      void this.connect().catch(() => {});
  }

  private connect(): Promise<void> {
    if (this.opening) return this.opening;
    if (this.link) return Promise.resolve();
    const generation = ++this.generation;
    const opening = this.start(generation).catch(error => {
      if (generation === this.generation) this.failed();
      throw error;
    }).finally(() => {
      if (generation === this.generation) this.opening = undefined;
    });
    return this.opening = opening;
  }

  private command(action: Record<string, unknown>) {
    if (!this.hub?.activityCamera) throw new Error("Activity camera transport unavailable.");
    return this.hub.activityCamera(this.place, { ...this.lease, action });
  }

  private release(grant: Grant) {
    void this.command({ op: "close", grant: grant.grant }).catch(() => {});
  }

  private async start(generation: number) {
    const iceServers = await this.getIce();
    if (generation !== this.generation) return;
    const reply = await this.command({ op: "open" });
    const { grant, sessionId, channelId } = reply;
    if (typeof grant !== "string" || typeof sessionId !== "string" || typeof channelId !== "string")
      throw new Error("Invalid activity camera session.");
    const credentials = { grant, sessionId, channelId };
    if (generation !== this.generation) { this.release(credentials); return; }
    let pc: RTCPeerConnection | undefined;
    try {
      pc = new RTCPeerConnection({ iceServers, bundlePolicy: "max-bundle" });
      const publisher = pc.addTransceiver("video", { direction: "inactive", streams: [new MediaStream()] });
      const link: Link = {
        pc, grant: credentials, publisher, directory: [], streams: new Map(),
        negotiation: Promise.resolve(), outbox: Promise.resolve(), subscriptions: NO_SUBSCRIPTIONS,
        needsOffer: true,
      };
      this.link = link;
      pc.ontrack = ({ transceiver, track }) => {
        if (this.link !== link || transceiver.mid === null || track.kind !== "video") return;
        link.streams.set(transceiver.mid, new MediaStream([track]));
        this.render();
      };
      pc.onicecandidate = ({ candidate }) => {
        void this.send(link, "candidate", encodeMediaCandidate(candidate?.toJSON() ?? null)).catch(() => {});
      };
      pc.onconnectionstatechange = () => {
        if (this.link !== link) return;
        if (link.pc.connectionState === "failed") this.failed();
        if (link.pc.connectionState === "connected") this.attempts = 0;
      };
      await this.enqueue(link, () => this.offer(link));
    } catch (error) {
      if (!this.link && generation === this.generation) { pc?.close(); this.release(credentials); }
      throw error;
    }
  }

  private enqueue(link: Link, step: () => Promise<void>) {
    const next = link.negotiation.then(() => this.link === link ? step() : undefined);
    link.negotiation = next.catch(() => { if (this.link === link) this.failed(); });
    return next;
  }

  private send(link: Link, kind: BrowserMediaSignalKind, payload: string) {
    const next = link.outbox.then(() => {
      if (this.link !== link) return;
      return this.hub!.sendMediaSignal({ protocolVersion: RTC_PROTOCOL_VERSION, channelId: link.grant.channelId,
        sessionId: link.grant.sessionId, kind, payload });
    });
    link.outbox = next.catch(() => { if (this.link === link) this.failed(); });
    return next;
  }

  private async offer(link: Link) {
    const { pc, publisher } = link;
    const stream = this.local;
    const track = stream?.getVideoTracks()[0] ?? null;
    if (publisher.sender.track !== track) {
      await publisher.sender.replaceTrack(track);
      if (this.link !== link) return;
      publisher.direction = track ? "sendonly" : "inactive";
      link.needsOffer = true;
    }
    if (pc.signalingState !== "stable" || !link.needsOffer) return;
    link.needsOffer = false;
    const offer = await pc.createOffer();
    await pc.setLocalDescription(offer);
    await this.send(link, "offer", encodeBrowserOffer(offer.sdp!));
  }

  private async receive(link: Link, event: MediaSignal<MediaServerSignalKind>) {
    const { pc } = link;
    if (event.kind === "offer") {
      const offer = decodeMediaServerOffer(event.payload);
      if (!offer || pc.signalingState !== "stable") return;
      link.directory = offer.tracks;
      // The media server forgets what was asked for on a section it hands to
      // another person's camera, so the list goes out again with every offer,
      // the same as the last or not. An empty one has nothing to forget.
      if (link.subscriptions !== NO_SUBSCRIPTIONS) link.subscriptions = "";
      this.render();
      await pc.setRemoteDescription({ type: "offer", sdp: offer.sdp });
      const answer = await pc.createAnswer();
      await pc.setLocalDescription(answer);
      await this.send(link, "answer", encodeBrowserAnswer(answer.sdp!, offer.negotiationId));
    } else if (event.kind === "answer") {
      const answer = decodeMediaServerAnswer(event.payload);
      if (!answer || pc.signalingState !== "have-local-offer") return;
      await pc.setRemoteDescription(answer);
    } else {
      const tracks = decodeMediaTracks(event.payload);
      if (tracks) { link.directory = tracks; this.render(); }
    }
    await this.offer(link);
  }

  private render() {
    const link = this.link;
    const tracks = link?.directory.filter(track => track.kind === "video" &&
      [...this.watches].some(watch => watch.user === track.userId)) ?? [];
    const subscriptions = encodeMediaSubscriptions(tracks.map(track => track.mid));
    if (link && subscriptions !== link.subscriptions && link.subscriptionTimer === undefined) {
      link.subscriptions = subscriptions;
      void this.send(link, "subscriptions", subscriptions).catch(() => {});
      // A game can move or replace tiles on every animation frame. Coalesce
      // its changes so subscriptions leave signalling capacity for the call.
      link.subscriptionTimer = setTimeout(() => {
        link.subscriptionTimer = undefined;
        if (this.link === link) this.render();
      }, 250);
    }
    for (const { user, show } of this.watches) {
      const track = tracks.find(track => track.userId === user && !track.heldBack);
      show(user === this.self ? this.local : (track && link?.streams.get(track.mid)) || null);
    }
  }

  private close() {
    this.generation++;
    this.opening = undefined;
    const link = this.link;
    this.link = null;
    if (link) {
      clearTimeout(link.subscriptionTimer);
      link.pc.close();
      this.release(link.grant);
    }
    clearTimeout(this.retry);
    this.retry = undefined;
    this.render();
  }

  private failed() {
    this.close();
    this.ended?.();
    if (!this.disposed && this.hasRemoteWatch() && this.hub?.getStatus() === "connected" && this.attempts < 3) {
      this.retry = setTimeout(() => {
        this.retry = undefined;
        if (this.hasRemoteWatch()) this.view();
      }, 2000 * 2 ** this.attempts++);
    }
  }

  dispose() {
    this.disposed = true;
    for (const off of this.off) off();
    this.ended?.();
    this.local = null;
    this.close();
    this.watches.clear();
  }
}
