import type { ActivityMedia } from "./media.js";
export type { GifError, GifErrorCode, GifImage, GifItem, GifPage } from "./media.js";

export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };
export type ActivityState = Record<string, JsonValue>;
export type Unsubscribe = () => void;
export interface Observable<T> {
  get(): T;
  subscribe(listener: (value: T) => void): Unsubscribe;
}
export interface Participant {
  id: string;
  username?: string;
  name: string;
  avatar: string | null;
  connected: boolean;
}
export type DeviceClass = "desktop" | "tablet" | "phone";
export function detectDevice(): DeviceClass;
export interface ActivityContext {
  standalone: boolean;
  launchMode: "voice" | "personal";
  sessionId: string;
  me: Participant;
  place: { id: string } | null;
  locale: string;
  device: DeviceClass;
}
export interface ActivityTheme {
  mode: "light" | "dark";
  colors: Record<"background" | "surface" | "text" | "muted" | "accent" | "border", string>;
}
export interface ActivityLimits {
  stateBytes: number;
  messageBytes: number;
  writesPerSecond: number;
  messagesPerSecond: number;
  graceSeconds: number;
  emptySeconds: number;
  maxParticipants: number;
}
/** Stellarr's dock over a full-screen activity, as this frame hears it. */
export interface DockState {
  /** The activity has the screen. The dock exists only then. */
  fullscreen: boolean;
  /** The dock is showing over the activity's bottom edge. */
  visible: boolean;
  /** A `hold()` is in force and the player allows it, so the edge no longer calls the dock up. */
  held: boolean;
  /** CSS pixels the visible dock takes from the bottom of the frame; 0 while hidden. */
  height: number;
}
export interface ShellDock extends Observable<DockState> {
  /**
   * Keeps the bottom edge for the activity while it is full screen: moving the
   * pointer there no longer brings the dock up, and the dock does not show on
   * entering full screen. The player keeps a small handle, the keyboard and
   * Esc, and may refuse holds altogether. Holds count; the last release ends it.
   */
  hold(): { release(): void };
  /** Shows the dock for a moment, for example after a round. Repeated calls are throttled. */
  peek(): void;
}
export interface CommandDecision<S> {
  patch?: Partial<S>;
  result?: JsonValue;
  assignments?: Record<string, JsonValue>;
}
export interface DevSession<S extends ActivityState> {
  context: ActivityContext;
  limits: ActivityLimits;
  capabilities: string[];
  now(): number;
  close(): Promise<void>;
  connection: Observable<"connected" | "reconnecting" | "closed">;
  theme: Observable<ActivityTheme>;
  locale: Observable<string>;
  device: Observable<DeviceClass>;
  progress: {
    load(): Promise<{ revision: number; data: ActivityState | null }>;
    save(data: ActivityState, expectedRevision: number): Promise<number>;
  };
  media: ActivityMedia;
  /** The page around the activity. Without a host that draws a dock, it stays hidden and `hold()` does nothing. */
  shell: { dock: ShellDock };
  participants: Observable<Participant[]>;
  send(kind: string, data: JsonValue | ArrayBuffer | ArrayBufferView, options?: { to?: string; latest?: boolean }): boolean;
  on(kind: string, listener: (data: JsonValue | Uint8Array, meta: { from: string }) => void): Unsubscribe;
  state: {
    get(): S;
    subscribe(listener: (state: S, revision: number) => void): Unsubscribe;
    set(patch: Partial<S>): Promise<number>;
    set(change: (current: S) => Partial<S> | void): Promise<number | undefined>;
  };
  seats: Observable<string[]> & { claim(): Promise<void>; release(): Promise<void> };
  assignments: Observable<JsonValue> & {
    checkpoint(): Promise<{ revision: number; state: S; assignments: Record<string, JsonValue> }>;
  };
  host: Observable<string | null> & { transfer(id: string): Promise<void> };
  commands: {
    handle(kind: string, handler: (
      data: JsonValue, meta: { id: string; from: string }, currentState: S,
      privateState: { assignments: Record<string, JsonValue> },
    ) => CommandDecision<S> | void | Promise<CommandDecision<S> | void>): Unsubscribe;
    submit(kind: string, data: JsonValue, options?: { id?: string }): Promise<JsonValue | undefined>;
  };
}
export function connect<S extends ActivityState = ActivityState>(options?: { initialState?: S }): Promise<DevSession<S>>;
