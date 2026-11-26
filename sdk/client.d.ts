import type { ActivityMedia } from "./media.js";

export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };
export type ActivityState = Record<string, JsonValue>;
export type Unsubscribe = () => void;
export interface Observable<T> {
  get(): T;
  subscribe(listener: (value: T) => void): Unsubscribe;
}
export interface Participant {
  id: string;
  name: string;
  avatar: string | null;
  connected: boolean;
}
export interface ActivityContext {
  standalone: boolean;
  launchMode: "voice" | "personal";
  sessionId: string;
  me: Participant;
  place: { id: string } | null;
  locale: string;
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
  media: ActivityMedia;
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
