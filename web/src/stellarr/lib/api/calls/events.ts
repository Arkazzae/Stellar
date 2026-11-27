/*
 * Server events on the wire: the event and its current date, what a new one
 * is sent as, the routes, the live events and how a refusal reads.
 * Everything that depends on the API's contract for events is in this file.
 */

import {
  EventDraftRefusedError,
  EventRefusedError,
  type EventDraft,
  type EventDraftField,
  type EventPeoplePage,
  type EventPerson,
  type EventPlace,
  type EventRefusalReason,
  type EventRepeat,
  type EventSignup,
  type EventSignupChange,
  type EventState,
  type ServerEvent,
} from "@/ports/events";

import { ApiError } from "../errors";
import { definedField, expectArray, expectNumber, expectRecord, expectString, isRecord, optionalNumber, optionalString } from "../expect";
import { deleteResource, getJson, route, sendJson, withQuery, type Parse } from "./http";

/** The live events. */
export const EVENT_EVENTS = Object.freeze({
  /** The event as it is now, to everyone who sees it; the reader's own place is null in it. */
  changed: "EventChanged",
  /** The reader's own place, to their own connections only. */
  signup: "EventSignupChanged",
} as const);

/** How many sign-ups one read takes: enough for the card's comparison with the channel. */
const PEOPLE_PAGE = 100;

const REPEATS: readonly EventRepeat[] = ["none", "weekly", "biweekly", "monthly"];
const STATES: readonly EventState[] = ["upcoming", "live", "ended", "cancelled"];
const SIGNUPS: readonly EventSignup[] = ["none", "signed", "waiting"];

const oneOf = <T extends string>(values: readonly T[], value: unknown): T | undefined =>
  values.find((candidate) => candidate === value);

const parsePerson: Parse<EventPerson> = (value, path) => {
  const record = expectRecord(value, path);
  const id = expectString(record, "id", path);
  return { id, displayName: optionalString(record, "displayName") ?? id };
};

/** An event with its current date. Every missing value comes as null; the domain leaves it out. */
export const parseEvent: Parse<ServerEvent> = (value, path) => {
  const record = expectRecord(value, path);
  const place = expectRecord(record["place"], `${path}.place`);
  const state = oneOf(STATES, record["state"]);
  if (state === undefined) throw new ApiError("malformed", `${path}.state is not a state`);
  // Null in a copy shared with everyone: not this reader's to know.
  const mine = oneOf(SIGNUPS, record["mine"]);
  return {
    id: expectString(record, "id", path),
    serverId: expectString(record, "serverId", path),
    name: expectString(record, "name", path),
    ...definedField("description", optionalString(record, "description")),
    place:
      place["kind"] === "voice"
        ? { kind: "voice", channelId: expectString(place, "channelId", `${path}.place`) }
        : { kind: "elsewhere", text: expectString(place, "text", `${path}.place`) },
    timeZone: expectString(record, "timeZone", path),
    repeat: oneOf(REPEATS, record["repeat"]) ?? "none",
    ...definedField("seats", optionalNumber(record, "seats")),
    createdBy: expectString(record, "createdBy", path),
    ...definedField("announceMessageId", optionalString(record, "announceMessageId")),
    occurrenceId: expectString(record, "occurrenceId", path),
    startsAt: expectString(record, "startsAt", path),
    ...definedField("endsAt", optionalString(record, "endsAt")),
    state,
    ...definedField("liveSince", optionalString(record, "liveSince")),
    ...definedField("endedAt", optionalString(record, "endedAt")),
    signedUp: expectNumber(record, "signedUp", path),
    waiting: expectNumber(record, "waiting", path),
    faces: Array.isArray(record["faces"])
      ? record["faces"].flatMap((face, index) => {
          try {
            return [parsePerson(face, `${path}.faces[${index}]`)];
          } catch {
            return [];
          }
        })
      : [],
    ...definedField("mine", mine),
    ...(mine === "waiting" ? definedField("myPosition", optionalNumber(record, "myPosition")) : {}),
    revision: expectNumber(record, "revision", path),
  };
};

/**
 * The event an announcement carries. One this client cannot read is left off
 * its message, which still shows, rather than losing the whole page.
 */
export function readEvent(value: unknown, path: string): ServerEvent | undefined {
  if (value === null || value === undefined) return undefined;
  try {
    return parseEvent(value, path);
  } catch {
    return undefined;
  }
}

/** `EventChanged`; null for a payload this client cannot read. */
export function parseEventChanged(value: unknown): ServerEvent | null {
  return readEvent(value, "EventChanged") ?? null;
}

/** `EventSignupChanged`; null for a payload this client cannot read. */
export function parseEventSignupChanged(value: unknown): EventSignupChange | null {
  if (!isRecord(value)) return null;
  const { eventId, occurrenceId, revision } = value;
  const mine = oneOf(SIGNUPS, value["mine"]);
  if (typeof eventId !== "string" || typeof occurrenceId !== "string") return null;
  if (typeof revision !== "number" || mine === undefined) return null;
  const position = optionalNumber(value, "myPosition");
  return { eventId, occurrenceId, revision, mine, ...(mine === "waiting" && position !== undefined ? { myPosition: position } : {}) };
}

/** A new or changed event, as the routes take it: every member, absent ones as null. */
export interface EventBody {
  name: string;
  description: string | null;
  place: EventPlace;
  startsAt: string;
  endsAt: string | null;
  timeZone: string;
  seats: number | null;
  repeat: EventRepeat;
  announceChannelId: string | null;
}

export function eventBody(draft: EventDraft): EventBody {
  return {
    name: draft.name.trim(),
    description: draft.description?.trim() || null,
    place:
      draft.place.kind === "voice"
        ? { kind: "voice", channelId: draft.place.channelId }
        : { kind: "elsewhere", text: draft.place.text.trim() },
    startsAt: draft.startsAt.toISOString(),
    endsAt: draft.endsAt?.toISOString() ?? null,
    timeZone: draft.timeZone,
    seats: draft.seats ?? null,
    repeat: draft.repeat,
    announceChannelId: draft.announceChannelId ?? null,
  };
}

/**
 * Only what the sheet changed. A running event refuses a new time or place,
 * so a change of its name must not carry the time it already has.
 */
export function changedBody(event: ServerEvent, draft: EventDraft): Partial<EventBody> {
  const body = eventBody(draft);
  const samePlace =
    event.place.kind === body.place.kind &&
    (event.place.kind === "voice"
      ? body.place.kind === "voice" && event.place.channelId === body.place.channelId
      : body.place.kind === "elsewhere" && event.place.text === body.place.text);
  const sameTime =
    Date.parse(event.startsAt) === Date.parse(body.startsAt) &&
    (event.endsAt === undefined ? body.endsAt === null : body.endsAt !== null && Date.parse(event.endsAt) === Date.parse(body.endsAt));
  return {
    ...(body.name === event.name ? {} : { name: body.name }),
    ...(body.description === (event.description ?? null) ? {} : { description: body.description }),
    ...(samePlace ? {} : { place: body.place }),
    ...(sameTime ? {} : { startsAt: body.startsAt, endsAt: body.endsAt, timeZone: body.timeZone }),
    ...(body.seats === (event.seats ?? null) ? {} : { seats: body.seats }),
    ...(body.repeat === event.repeat ? {} : { repeat: body.repeat }),
    ...(body.announceChannelId === null ? {} : { announceChannelId: body.announceChannelId }),
  };
}

const parseEvents: Parse<ServerEvent[]> = (value, path) =>
  expectArray(value, path).flatMap((item, index) => {
    const event = readEvent(item, `${path}[${index}]`);
    return event === undefined ? [] : [event];
  });

/** A created event comes alone, or beside the announcement it was posted with. */
const parseCreated: Parse<ServerEvent> = (value, path) =>
  isRecord(value) && isRecord(value["event"]) ? parseEvent(value["event"], `${path}.event`) : parseEvent(value, path);

const parsePeople: Parse<EventPeoplePage> = (value, path) => {
  const record = expectRecord(value, path);
  return {
    people: expectArray(record["people"], `${path}.people`).map((item, index) => {
      const at = `${path}.people[${index}]`;
      const entry = expectRecord(item, at);
      const state = entry["state"] === "waiting" ? "waiting" : "signed";
      return {
        ...parsePerson(entry["user"], `${at}.user`),
        state,
        ...(state === "waiting" ? definedField("position", optionalNumber(entry, "position")) : {}),
      };
    }),
    nextCursor: optionalString(record, "nextCursor") ?? null,
  };
};

export interface EventApiClient {
  listEvents(): Promise<ServerEvent[]>;
  getEvent(eventId: string): Promise<ServerEvent>;
  createEvent(draft: EventDraft): Promise<ServerEvent>;
  editEvent(event: ServerEvent, draft: EventDraft): Promise<ServerEvent>;
  cancelEvent(eventId: string): Promise<void>;
  signUp(eventId: string, occurrenceId: string): Promise<ServerEvent>;
  signOut(eventId: string, occurrenceId: string): Promise<ServerEvent>;
  people(eventId: string, occurrenceId: string, cursor?: string): Promise<EventPeoplePage>;
}

const CODES: Record<string, EventRefusalReason> = {
  "event-over": "over",
  "event-live": "live",
  "event-occurrence-gone": "moved",
  "event-limit": "limit",
};

/** A refusal as a reason the card or the sheet can say. */
function refused(error: unknown): never {
  if (!(error instanceof ApiError)) throw error;
  const reason =
    (error.code === undefined ? undefined : CODES[error.code]) ??
    (error.status === 403 ? "forbidden" : error.status === 404 ? "gone" : "other");
  throw new EventRefusedError(reason, { cause: error });
}

/** A sheet's refusal: its fields where the server named them, else as any other refusal. */
function sheetRefused(error: unknown): never {
  if (error instanceof ApiError && error.status === 400 && error.fields !== undefined) {
    const fields = new Set<EventDraftField>();
    for (const key of Object.keys(error.fields)) {
      const name = key.toLowerCase();
      if (name === "name") fields.add("name");
      else if (name === "description") fields.add("description");
      else if (name.startsWith("place")) fields.add("place");
      else if (name === "startsat" || name === "timezone") fields.add("startsAt");
      else if (name === "endsat") fields.add("endsAt");
      else if (name === "seats") fields.add("seats");
      else if (name === "announcechannelid") fields.add("announce");
    }
    if (fields.size > 0) throw new EventDraftRefusedError([...fields], { cause: error });
  }
  refused(error);
}

const occurrenceRoute = (eventId: string, occurrenceId: string) =>
  route`/api/events/${eventId}/occurrences/${occurrenceId}/signup`;

export function eventCalls(): EventApiClient {
  return {
    // The server is the one every `/api/` call names in its header.
    listEvents: () => getJson("/api/events", parseEvents),
    getEvent: (eventId) => getJson(route`/api/events/${eventId}`, parseEvent).catch(refused),
    createEvent: (draft) => sendJson("POST", "/api/events", eventBody(draft), parseCreated).catch(sheetRefused),
    editEvent: (event, draft) =>
      sendJson("PATCH", route`/api/events/${event.id}`, changedBody(event, draft), parseEvent).catch(sheetRefused),
    cancelEvent: (eventId) => deleteResource(route`/api/events/${eventId}`).catch(refused),
    signUp: (eventId, occurrenceId) =>
      sendJson("PUT", occurrenceRoute(eventId, occurrenceId), undefined, parseEvent).catch(refused),
    signOut: (eventId, occurrenceId) =>
      sendJson("DELETE", occurrenceRoute(eventId, occurrenceId), undefined, parseEvent).catch(refused),
    people: (eventId, occurrenceId, cursor) =>
      getJson(
        withQuery(route`/api/events/${eventId}/occurrences/${occurrenceId}/signups`, { limit: PEOPLE_PAGE, cursor }),
        parsePeople,
      ),
  };
}
