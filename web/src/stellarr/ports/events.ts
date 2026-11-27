/*
 * A server's events: something that happens at a time, on one of its voice
 * channels or somewhere else, that people sign up for. An event has one
 * current date at a time; a repeating one moves on to its next date when the
 * current one ends. Sign-ups and seats belong to that date.
 */

export type EventRepeat = "none" | "weekly" | "biweekly" | "monthly";

export const EVENT_REPEATS: readonly EventRepeat[] = ["none", "weekly", "biweekly", "monthly"];

/** Where it happens: a voice channel of the server, named from the reader's own list, or a place in words. */
export type EventPlace = { kind: "voice"; channelId: string } | { kind: "elsewhere"; text: string };

/** The reader's own place: not signed up, holding a seat, or waiting for one. */
export type EventSignup = "none" | "signed" | "waiting";

export type EventState = "upcoming" | "live" | "ended" | "cancelled";

export interface EventPerson {
  id: string;
  displayName: string;
}

/** An event with its current date, as the reader sees it. */
export interface ServerEvent {
  id: string;
  serverId: string;
  name: string;
  description?: string;
  place: EventPlace;
  /** The creator's zone, which its dates repeat in. */
  timeZone: string;
  repeat: EventRepeat;
  /** How many may hold a seat; absent when anyone may. */
  seats?: number;
  createdBy: string;
  /** The message on a text channel that announced it. */
  announceMessageId?: string;
  /** The current date: what is signed up for. */
  occurrenceId: string;
  startsAt: string;
  endsAt?: string;
  state: EventState;
  /** When it really began: when the first person came into its channel. */
  liveSince?: string;
  endedAt?: string;
  /** Seats taken, and people waiting for one. */
  signedUp: number;
  waiting: number;
  /** The first few holding a seat, in sign-up order. */
  faces: EventPerson[];
  /** The reader's own place; absent when a copy shared with everyone could not say. */
  mine?: EventSignup;
  /** The reader's place on the waitlist, from 1, while they wait. */
  myPosition?: number;
  /** Grows with every change, so an older copy arriving late is dropped. */
  revision: number;
  /** The same order for the reader's own place, which arrives on its own. */
  mineRevision?: number;
}

/** The reader's own place, changed by them or by others leaving the queue ahead of them. */
export interface EventSignupChange {
  eventId: string;
  occurrenceId: string;
  revision: number;
  mine: EventSignup;
  myPosition?: number;
}

/** An event as it is written in its sheet, before it is created or changed. */
export interface EventDraft {
  name: string;
  place: EventPlace;
  startsAt: Date;
  endsAt?: Date;
  /** The zone the times were written in. */
  timeZone: string;
  description?: string;
  seats?: number;
  repeat: EventRepeat;
  /** A text channel to announce it on. */
  announceChannelId?: string;
}

/** The limits an event is written within; lengths count as `String.length` does, after trimming. */
export const EVENT_LIMITS = Object.freeze({
  name: 100,
  place: 200,
  description: 1000,
  maxSeats: 999,
  /** How far ahead the first date may be. */
  maxDaysAhead: 365,
  /** How long one date may last. */
  maxDays: 7,
  /** How long ago a start may be, for a sheet left open a moment. */
  startGraceMs: 5 * 60_000,
});

export interface EventSignupPerson extends EventPerson {
  state: "signed" | "waiting";
  position?: number;
}

export interface EventPeoplePage {
  people: EventSignupPerson[];
  nextCursor: string | null;
}

/** Why a sign-up, a change or a cancellation did not happen. */
export type EventRefusalReason =
  /** It ended or was cancelled. */
  | "over"
  /** Its time and place cannot change while it runs. */
  | "live"
  /** The date signed up for is no longer the current one: read it again. */
  | "moved"
  /** It is not there for this reader any more. */
  | "gone"
  | "forbidden"
  /** The server already has as many coming events as it may. */
  | "limit"
  | "other";

export class EventRefusedError extends Error {
  readonly reason: EventRefusalReason;

  constructor(reason: EventRefusalReason, options?: ErrorOptions) {
    super(`Event action refused: ${reason}`, options);
    this.name = "EventRefusedError";
    this.reason = reason;
  }
}

/** Which part of a sheet the server refused. */
export type EventDraftField = "name" | "description" | "place" | "startsAt" | "endsAt" | "seats" | "announce";

/** An event the server would not save, with the fields it named. */
export class EventDraftRefusedError extends Error {
  readonly fields: readonly EventDraftField[];

  constructor(fields: readonly EventDraftField[], options?: ErrorOptions) {
    super("Event refused", options);
    this.name = "EventDraftRefusedError";
    this.fields = fields;
  }
}

export interface EventsPort {
  /** The server's coming and running events, soonest first; read once it is looked at. */
  list(serverId: string): readonly ServerEvent[];
  /** The newest copy heard of one event, wherever it came from. */
  get(eventId: string): ServerEvent | undefined;
  subscribe(listener: () => void): () => void;
  load(serverId: string): Promise<void>;
  /** Takes in a copy that came with a message, as any other copy. */
  remember(event: ServerEvent): void;
  create(draft: EventDraft): Promise<ServerEvent>;
  /** Sends what the sheet changed: a running event keeps its time and place. */
  edit(event: ServerEvent, draft: EventDraft): Promise<ServerEvent>;
  /** Cancels the whole series; those signed up or waiting are told. */
  cancel(eventId: string): Promise<void>;
  /** Signs up for the current date, or joins its waitlist when it is full. */
  signUp(event: ServerEvent): Promise<void>;
  /** Signs out, or leaves the waitlist. */
  signOut(event: ServerEvent): Promise<void>;
  /** A page of who signed up, seated first, then the waitlist. */
  people(event: ServerEvent, cursor?: string): Promise<EventPeoplePage>;
}
