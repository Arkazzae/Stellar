import type { RtcConfig } from "@/lib/api/types";

import type { RealtimeClient, RealtimeEventMap, RealtimeEventName, RealtimeStatus } from "./types";

/** One subscription made through the wrapper, which the real client takes over once it exists. */
interface Subscription {
  attach(client: RealtimeClient): () => void;
  /** Cancels the subscription on the real client; null until it has one. */
  detach: (() => void) | null;
}

/**
 * A realtime client whose implementation is fetched on first use. The sign-in
 * pages build the runtime but never open the socket, so the SignalR library
 * stays out of the files they wait for and arrives with the signed-in app.
 *
 * Everything the session does before the implementation is there is kept and
 * replayed on it: event subscriptions, status listeners and the RTC contract.
 * A call that needs the socket waits for the implementation and then goes to
 * it in the order it was made, so a `stop()` after a `start()` still comes
 * after it. When the implementation cannot be loaded those calls reject with
 * the load's error and the status reads "disconnected"; the next call tries
 * the load again.
 */
export class DeferredRealtimeClient implements RealtimeClient {
  private client: RealtimeClient | null = null;
  private loading: Promise<RealtimeClient> | null = null;
  private failed = false;
  private rtc: { value: Pick<RtcConfig, "signalingHub"> | undefined } | null = null;
  private readonly subscriptions = new Set<Subscription>();
  private readonly statusListeners = new Set<() => void>();

  constructor(private readonly load: () => Promise<RealtimeClient>) {}

  configureRtc(rtc: Pick<RtcConfig, "signalingHub"> | undefined): void {
    this.rtc = { value: rtc };
    this.client?.configureRtc?.(rtc);
  }

  /** The implementation, once loaded. Calls made while it loads share the one request. */
  private ready(): Promise<RealtimeClient> {
    if (this.client !== null) return Promise.resolve(this.client);
    this.loading ??= this.load().then(
      (client) => {
        this.client = client;
        this.failed = false;
        if (this.rtc !== null) client.configureRtc?.(this.rtc.value);
        for (const subscription of this.subscriptions) subscription.detach ??= subscription.attach(client);
        return client;
      },
      (cause: unknown) => {
        this.loading = null;
        this.failed = true;
        for (const listener of [...this.statusListeners]) listener();
        throw cause;
      },
    );
    return this.loading;
  }

  start(): Promise<void> {
    return this.ready().then((client) => client.start());
  }

  stop(): Promise<void> {
    // Nothing was ever started, and loading the library to stop it is a wasted request.
    if (this.client === null && this.loading === null) return Promise.resolve();
    return this.ready().then((client) => client.stop());
  }

  getStatus(): RealtimeStatus {
    return this.client?.getStatus() ?? (this.failed ? "disconnected" : "idle");
  }

  subscribeStatus(listener: () => void): () => void {
    this.statusListeners.add(listener);
    const unsubscribe = this.subscribe({ detach: null, attach: (client) => client.subscribeStatus(listener) });
    return () => {
      this.statusListeners.delete(listener);
      unsubscribe();
    };
  }

  on<E extends RealtimeEventName>(event: E, handler: (payload: RealtimeEventMap[E]) => void): () => void {
    return this.subscribe({ detach: null, attach: (client) => client.on(event, handler) });
  }

  private subscribe(subscription: Subscription): () => void {
    this.subscriptions.add(subscription);
    if (this.client !== null) subscription.detach = subscription.attach(this.client);
    return () => {
      this.subscriptions.delete(subscription);
      subscription.detach?.();
    };
  }

  setPresence: RealtimeClient["setPresence"] = (presence) => this.ready().then((client) => client.setPresence(presence));
  joinChannel: RealtimeClient["joinChannel"] = (channelId) => this.ready().then((client) => client.joinChannel(channelId));
  sendTyping: RealtimeClient["sendTyping"] = (channelId) => this.ready().then((client) => client.sendTyping(channelId));
  joinVoice: RealtimeClient["joinVoice"] = (channelId) => this.ready().then((client) => client.joinVoice(channelId));
  leaveVoice: RealtimeClient["leaveVoice"] = () => this.ready().then((client) => client.leaveVoice());
  declineDirectCall: RealtimeClient["declineDirectCall"] = (id) => this.ready().then((client) => client.declineDirectCall(id));
  sendVoiceSignal: RealtimeClient["sendVoiceSignal"] = (signal) => this.ready().then((client) => client.sendVoiceSignal(signal));
  openMediaSession: RealtimeClient["openMediaSession"] = (channelId) =>
    this.ready().then((client) => client.openMediaSession(channelId));
  sendMediaSignal: RealtimeClient["sendMediaSignal"] = (signal) => this.ready().then((client) => client.sendMediaSignal(signal));
  playSoundboard: RealtimeClient["playSoundboard"] = (channelId, soundId) =>
    this.ready().then((client) => client.playSoundboard(channelId, soundId));
  getRtcClock: RealtimeClient["getRtcClock"] = () => this.ready().then((client) => client.getRtcClock());
}
