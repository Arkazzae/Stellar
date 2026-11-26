import { isRecord } from "@/lib/api/expect";
import { readActiveServerId } from "@/lib/api/transport";
import type { ChannelId, Presence, RtcConfig } from "@/lib/api/types";
import { RTC_HUB, type BrowserMediaSignalKind } from "@/lib/rtc/protocol";
import { ChangeNotifier, Emitter } from "./emitter";
import { HUB_DECODERS, parseMediaSessionOpened } from "./hubPayloads";
import type {
  MediaSignal,
  RealtimeClient,
  RealtimeEventMap,
  RealtimeEventName,
  RealtimeStatus,
  VoiceSignal,
} from "./types";

const SUBPROTOCOL = "stellarr.v1";
const READY_TIMEOUT_MS = 20_000;
const CALL_TIMEOUT_MS = 15_000;
const SERVER_SILENCE_MS = 45_000;
const MAX_PENDING_CALLS = 128;
const MAX_FRAME_BYTES = 160 * 1024;
const MAX_BUFFERED_BYTES = 1024 * 1024;
const RETRY_DELAYS_MS = [1_000, 2_000, 5_000, 10_000, 20_000, 30_000] as const;
const SESSION_ENDED = 4401;
const INCOMPATIBLE = 4406;
const PROTOCOL_ERROR = 1002;
const TERMINAL_CLOSE_CODES = new Set([1000, PROTOCOL_ERROR, 1007, 1009, SESSION_ENDED, INCOMPATIBLE]);
const utf8 = new TextEncoder();

type Timer = ReturnType<typeof setTimeout>;

interface PendingCall {
  resolve(value: unknown): void;
  reject(error: unknown): void;
  timer: Timer;
}

type Socket = Pick<WebSocket, "bufferedAmount" | "readyState" | "protocol" | "onmessage" | "onclose" | "send" | "close">;
type SocketFactory = (url: string, protocol: string) => Socket;

interface Connection {
  socket: Socket;
  url: string;
  serverId: string | undefined;
  ready: boolean;
  started: Promise<void>;
  resolve(): void;
  reject(error: unknown): void;
  timer: Timer;
  lastActivity: number;
  nextId: number;
  pending: Map<number, PendingCall>;
  voiceSequences: Map<ChannelId, number>;
}

function signalingHubPath(candidate: unknown): string {
  // The cookie authenticates the socket, so bootstrap may only name a local path.
  return typeof candidate === "string" &&
    candidate.startsWith("/") &&
    !candidate.startsWith("//") &&
    !candidate.includes("\\") &&
    !candidate.includes("#") &&
    candidate.trim() === candidate &&
    ![...candidate].some((character) => character.charCodeAt(0) < 32)
    ? candidate
    : RTC_HUB.url;
}

function retryDelay(attempt: number): number {
  const base = RETRY_DELAYS_MS[attempt] ?? 30_000;
  return Math.round(base * (1 + Math.random() * 0.25));
}

function parseFrame(data: unknown): Record<string, unknown> | null {
  if (typeof data !== "string") return null;
  try {
    const value: unknown = JSON.parse(data);
    return isRecord(value) ? value : null;
  } catch {
    return null;
  }
}

export class WebSocketRealtimeClient implements RealtimeClient {
  private connection: Connection | null = null;
  private hubPath: string = RTC_HUB.url;
  private status: RealtimeStatus = "idle";
  private retryTimer: Timer | null = null;
  private retryAttempts = 0;
  private target: string | null = null;
  private terminalError: Error | null = null;
  private sessionEnded = false;
  private readonly events = new Emitter<RealtimeEventMap>();
  private readonly statusNotifier = new ChangeNotifier();

  constructor(
    private readonly createSocket: SocketFactory = (url, protocol) => new WebSocket(url, protocol),
  ) {}

  configureRtc(rtc: Pick<RtcConfig, "signalingHub"> | undefined): void {
    if (this.connection === null) this.hubPath = signalingHubPath(rtc?.signalingHub);
  }

  getStatus(): RealtimeStatus {
    return this.status;
  }

  hasSessionEnded(): boolean {
    return this.sessionEnded;
  }

  subscribeStatus = (listener: () => void): (() => void) =>
    this.statusNotifier.subscribe(listener);

  on<E extends RealtimeEventName>(
    event: E,
    handler: (payload: RealtimeEventMap[E]) => void,
  ): () => void {
    return this.events.on(event, handler);
  }

  async start(): Promise<void> {
    const url = this.hubUrl();
    if (url !== this.target) {
      this.reset(new DOMException("Realtime target changed", "AbortError"));
      this.target = url;
      return this.connect(url, "connecting");
    }
    if (this.terminalError !== null) throw this.terminalError;
    if (this.connection !== null) return this.connection.started;
    this.clearRetryTimer();
    return this.connect(url, this.status);
  }

  async stop(): Promise<void> {
    this.reset(new DOMException("Realtime connection stopped", "AbortError"));
    this.setStatus("idle");
  }

  setPresence(presence: Presence): Promise<void> {
    return this.command("SetPresence", presence);
  }

  joinChannel(channelId: ChannelId): Promise<void> {
    return this.command("JoinChannel", channelId);
  }

  async sendTyping(channelId: ChannelId): Promise<void> {
    const connection = this.openConnection("SendTyping");
    this.send(connection, { type: "send", method: "SendTyping", args: [channelId] });
  }

  joinVoice(channelId: ChannelId): Promise<void> {
    return this.command(RTC_HUB.methods.join, channelId);
  }

  leaveVoice(): Promise<void> {
    return this.command(RTC_HUB.methods.leave);
  }

  declineDirectCall(conversationId: ChannelId): Promise<void> {
    return this.command(RTC_HUB.methods.declineDirectCall, conversationId);
  }

  sendVoiceSignal(signal: VoiceSignal): Promise<void> {
    return this.command(RTC_HUB.methods.signal, signal);
  }

  async openMediaSession(channelId: ChannelId): Promise<string> {
    const result = await this.invoke(RTC_HUB.methods.openMediaSession, channelId);
    const sessionId = parseMediaSessionOpened(result);
    if (sessionId === null) {
      throw new Error(`${RTC_HUB.methods.openMediaSession} returned no session id`);
    }
    return sessionId;
  }

  sendMediaSignal(signal: MediaSignal<BrowserMediaSignalKind>): Promise<void> {
    return this.command(RTC_HUB.methods.mediaSignal, signal);
  }

  playSoundboard(channelId: ChannelId, soundId: string): Promise<void> {
    return this.command(RTC_HUB.methods.playSoundboard, channelId, soundId);
  }

  async getRtcClock(): Promise<number> {
    const value = await this.invoke(RTC_HUB.methods.clock);
    if (typeof value !== "number" || !Number.isFinite(value)) {
      throw new Error(`${RTC_HUB.methods.clock} returned no clock value`);
    }
    return value;
  }

  activityCamera(place: string, input: Record<string, unknown>): Promise<Record<string, unknown>> {
    return this.activityRequest("ActivityCamera", place, input);
  }

  activityRoom(place: string, input: Record<string, unknown>): Promise<Record<string, unknown>> {
    return this.activityRequest("ActivityRoom", place, input);
  }

  private async activityRequest(method: string, place: string, input: Record<string, unknown>): Promise<Record<string, unknown>> {
    const reply = await this.invoke(method, place, input);
    if (!isRecord(reply)) throw new Error("Invalid activity room response");
    if (typeof reply.error === "string") throw Object.assign(new Error(reply.error), { status: reply.status });
    if (!isRecord(reply.value)) throw new Error("Missing activity room response");
    return reply.value;
  }

  private async command(method: string, ...args: unknown[]): Promise<void> {
    await this.invoke(method, ...args);
  }

  private async invoke(method: string, ...args: unknown[]): Promise<unknown> {
    const connection = this.openConnection(method);
    if (connection.pending.size >= MAX_PENDING_CALLS) {
      throw new Error("Too many realtime calls in progress");
    }
    if (connection.nextId >= Number.MAX_SAFE_INTEGER) {
      throw new Error("Realtime call ids exhausted");
    }
    const id = ++connection.nextId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        connection.pending.delete(id);
        reject(new Error(`Realtime call ${method} timed out`));
      }, CALL_TIMEOUT_MS);
      connection.pending.set(id, { resolve, reject, timer });
      try {
        this.send(connection, { type: "call", id, method, args });
      } catch (cause) {
        clearTimeout(timer);
        connection.pending.delete(id);
        reject(cause);
      }
    });
  }

  private openConnection(method: string): Connection {
    const connection = this.connection;
    if (connection === null || connection.serverId !== readActiveServerId() ||
        !connection.ready || connection.socket.readyState !== WebSocket.OPEN) {
      throw new Error(`Cannot call ${method}: realtime connection is not open`);
    }
    return connection;
  }

  private send(connection: Connection, frame: object): void {
    const data = JSON.stringify(frame);
    const byteLength = utf8.encode(data).length;
    if (byteLength > MAX_FRAME_BYTES) throw Object.assign(new Error("Realtime message is too large"), { code: "frame_too_large" });
    if (connection.socket.bufferedAmount + byteLength > MAX_BUFFERED_BYTES) {
      throw new Error("Realtime send buffer is full");
    }
    connection.socket.send(data);
  }

  private hubUrl(): string {
    const url = new URL(this.hubPath, window.location.origin);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    const serverId = readActiveServerId();
    if (serverId !== undefined) url.searchParams.set("serverId", serverId);
    return url.href;
  }

  private async connect(url: string, status: RealtimeStatus): Promise<void> {
    let socket: Socket;
    try {
      socket = this.createSocket(url, SUBPROTOCOL);
    } catch (cause) {
      this.scheduleRetry(url);
      if (status !== "reconnecting") this.setStatus("disconnected");
      throw cause;
    }

    let resolve!: () => void;
    let reject!: (error: unknown) => void;
    const started = new Promise<void>((yes, no) => {
      resolve = yes;
      reject = no;
    });
    const timer = setTimeout(() => {
      this.lose(connection, 1011, new Error("Timed out waiting for realtime readiness"), 4000);
    }, READY_TIMEOUT_MS);
    const connection: Connection = {
      socket,
      url,
      serverId: readActiveServerId(),
      ready: false,
      started,
      resolve,
      reject,
      timer,
      lastActivity: 0,
      nextId: 0,
      pending: new Map(),
      voiceSequences: new Map(),
    };
    this.connection = connection;
    socket.onmessage = ({ data }) => this.receive(connection, data);
    // Browsers report an opaque error followed by close for failed upgrades.
    // SessionProvider checks the session over HTTP when this changes status.
    socket.onclose = ({ code }) => this.lose(connection, code, new Error(`Realtime socket closed (${code})`));
    this.setStatus(status);
    return started;
  }

  private receive(connection: Connection, data: unknown): void {
    if (connection !== this.connection || connection.serverId !== readActiveServerId()) return;
    const frame = parseFrame(data);
    if (frame !== null && !connection.ready && frame["type"] === "ready") {
      if (connection.socket.protocol !== SUBPROTOCOL) {
        this.lose(connection, INCOMPATIBLE, new Error("Incompatible realtime subprotocol"), INCOMPATIBLE);
        return;
      }
      clearTimeout(connection.timer);
      connection.ready = true;
      connection.lastActivity = Date.now();
      connection.timer = setTimeout(() => this.checkActivity(connection), SERVER_SILENCE_MS);
      this.retryAttempts = 0;
      connection.resolve();
      this.setStatus("connected");
      return;
    }
    if (frame !== null && connection.ready && this.dispatch(connection, frame)) {
      connection.lastActivity = Date.now();
      return;
    }
    // WebSocket.close() only accepts 1000 or application codes in the browser.
    this.lose(connection, PROTOCOL_ERROR, new Error("Malformed realtime message"), 4002);
  }

  private dispatch(connection: Connection, frame: Record<string, unknown>): boolean {
    const type = frame["type"];
    if (type === "ping") return true;
    if (type === "event") {
      const name = frame["name"];
      if (typeof name !== "string" || !Object.hasOwn(frame, "data")) return false;
      if (Object.hasOwn(HUB_DECODERS, name)) {
        this.relay(connection, name as RealtimeEventName, frame["data"]);
      }
      return true;
    }
    if (type !== "result" && type !== "error") return false;
    const id = frame["id"];
    if (typeof id !== "number" || !Number.isSafeInteger(id) || id < 1) return false;
    if (type === "result" && !Object.hasOwn(frame, "value")) return false;
    const code = frame["code"];
    const message = frame["message"];
    if (type === "error" && (typeof code !== "string" || (message !== undefined && typeof message !== "string"))) {
      return false;
    }

    // A response may arrive after its deadline. Ids are never reused on this socket.
    const pending = connection.pending.get(id);
    if (pending === undefined) return true;
    clearTimeout(pending.timer);
    connection.pending.delete(id);
    if (type === "result") pending.resolve(frame["value"]);
    else pending.reject(Object.assign(new Error(typeof message === "string" ? message : String(code)), { code }));
    return true;
  }

  private relay<E extends RealtimeEventName>(connection: Connection, event: E, payload: unknown): void {
    const decoded = HUB_DECODERS[event](payload);
    if (decoded === null) return;
    if (event === "VoiceParticipantChanged") {
      const { channelId, sequence } = decoded as RealtimeEventMap["VoiceParticipantChanged"];
      if (sequence !== undefined) {
        const previous = connection.voiceSequences.get(channelId);
        if (previous !== undefined && sequence <= previous) return;
        connection.voiceSequences.set(channelId, sequence);
      }
    }
    this.events.emit(event, decoded);
  }

  private checkActivity(connection: Connection): void {
    if (connection !== this.connection) return;
    // A suspended browser can deliver this timer late. Measure the silence
    // against wall time so a laptop that wakes on a broken link reconnects.
    const elapsed = Math.max(0, Date.now() - connection.lastActivity);
    if (elapsed >= SERVER_SILENCE_MS) {
      this.lose(connection, 1011, new Error("Timed out waiting for realtime traffic"), 4000);
      return;
    }
    connection.timer = setTimeout(
      () => this.checkActivity(connection),
      SERVER_SILENCE_MS - elapsed,
    );
  }

  private lose(connection: Connection, code: number, error: Error, closeCode?: number): void {
    if (connection !== this.connection) return;
    this.release(connection, error, closeCode);
    if (connection.url !== this.hubUrl()) {
      this.setStatus("disconnected");
      return;
    }
    if (TERMINAL_CLOSE_CODES.has(code)) {
      this.terminalError = error;
      this.sessionEnded = code === SESSION_ENDED;
      this.setStatus("disconnected", this.sessionEnded);
      return;
    }
    this.scheduleRetry(connection.url);
    this.setStatus(connection.ready || this.status === "reconnecting" ? "reconnecting" : "disconnected");
  }

  private release(connection: Connection, error: Error, closeCode?: number): void {
    this.connection = null;
    clearTimeout(connection.timer);
    connection.socket.onmessage = null;
    connection.socket.onclose = null;
    connection.reject(error);
    for (const pending of connection.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    connection.pending.clear();
    if (closeCode !== undefined) connection.socket.close(closeCode);
  }

  private reset(error: Error): void {
    this.target = null;
    this.terminalError = null;
    this.sessionEnded = false;
    this.clearRetryTimer();
    this.retryAttempts = 0;
    if (this.connection !== null) this.release(this.connection, error, 1000);
  }

  private scheduleRetry(url: string): void {
    const delay = retryDelay(this.retryAttempts);
    this.retryAttempts = Math.min(this.retryAttempts + 1, RETRY_DELAYS_MS.length - 1);
    this.retryTimer = setTimeout(() => {
      this.retryTimer = null;
      if (url !== this.target || url !== this.hubUrl()) return;
      void this.connect(url, this.status).catch(() => undefined);
    }, delay);
  }

  private clearRetryTimer(): void {
    if (this.retryTimer === null) return;
    clearTimeout(this.retryTimer);
    this.retryTimer = null;
  }

  private setStatus(status: RealtimeStatus, notify = false): void {
    if (this.status === status && !notify) return;
    this.status = status;
    this.statusNotifier.notify();
  }
}
