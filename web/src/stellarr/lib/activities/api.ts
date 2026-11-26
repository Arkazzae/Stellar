import { detectDevice, type DeviceClass } from "../../../../../sdk/client.js";
import { isRecord, stringArray } from "@/lib/api/expect";
import { problemCode, request } from "@/lib/api/transport";
import { parseActivitySummary } from "@/lib/realtime/hubPayloads";
import type { VoiceActivitySummary } from "@/lib/realtime/types";

/*
 * The activities API, in one place: the requests, what their answers say, and
 * why a refused one was refused. Every screen that starts, joins or lists an
 * activity goes through here, so a field the server renames is renamed once.
 */

export type ActivitySummary = VoiceActivitySummary;

/** The path segment of a session's place: a voice channel's id, or this one. */
export const PERSONAL = "personal";

export type { DeviceClass };

/** What kind of device this is, as an activity's manifest names devices. */
export const deviceClass: () => DeviceClass = detectDevice;

/** One installed activity this place may run. */
export interface ActivityEntry {
  install: string;
  name: string;
  version: string;
  description: string;
  icon: string | null;
  cover: string | null;
  running: boolean;
  /** Whether it runs on this device; `devices` says where it does. */
  supported: boolean;
  devices: string[] | null;
  seats: number | null;
  /** Manifest permissions, such as `media.cameraTile`. */
  permissions: string[];
}

export interface ActivityOwner {
  id: string;
  name: string;
  avatar: string | null;
}

/** What a place may run, and who installs more. */
export interface ActivityLibrary {
  activities: ActivityEntry[];
  /** What runs on the voice channel right now; always null for yourself. */
  live: ActivitySummary | null;
  /** The instance's owner, who installs activities; null when unknown. */
  owner: ActivityOwner | null;
  viewerIsOwner: boolean;
  /** For yourself: the names of activities that run only on a voice channel. */
  voiceOnly: string[];
}

/** A seat in a running activity: the frame to open and the lease that keeps it. */
export interface ActivityLaunch {
  install: string;
  access: string;
  src: string;
  title: string;
  permissions: string[];
  /** Who hosts it after this launch; null when the server did not say. */
  host: string | null;
  /** The activity plays YouTube, which the host page draws for it. */
  youtube: boolean;
}

/** Why the server refused, by the `code` of its problem body. */
export type ActivityRefusal =
  | "activity-running"
  | "not-host"
  | "full"
  | "unsupported-device"
  | "not-found";

const REFUSALS: readonly ActivityRefusal[] = [
  "activity-running",
  "not-host",
  "full",
  "unsupported-device",
  "not-found",
];

/**
 * A refused or failed activities request. `status` is 0 when no answer came;
 * `message` is the body's text, which a frame's own requests hand on to the
 * activity.
 */
export class ActivityRequestError extends Error {
  readonly status: number;
  readonly code: ActivityRefusal | null;

  constructor(status: number, code: ActivityRefusal | null, message: string) {
    super(message);
    this.name = "ActivityRequestError";
    this.status = status;
    this.code = code;
  }
}

function refusalOf(code: string | null): ActivityRefusal | null {
  return REFUSALS.find((candidate) => candidate === code) ?? null;
}

async function failure(response: Response): Promise<ActivityRequestError> {
  const code = refusalOf(await problemCode(response));
  let text = "";
  try {
    text = await response.text();
  } catch {
    // The status and the code say enough.
  }
  return new ActivityRequestError(response.status, code, text || `Activity request failed (${response.status}).`);
}

async function send(path: string, init: RequestInit, signal?: AbortSignal): Promise<Response> {
  let response: Response;
  try {
    response = await request(path, init, signal);
  } catch (cause) {
    if (isRecord(cause) && cause["name"] === "AbortError") throw cause;
    throw new ActivityRequestError(0, null, "The activities request got no answer.");
  }
  if (!response.ok) throw await failure(response);
  return response;
}

async function post(place: string, action: string, input: object | undefined, signal?: AbortSignal): Promise<unknown> {
  const response = await send(
    `/api/activities/${encodeURIComponent(place)}/${action}`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(input ?? {}),
    },
    signal,
  );
  try {
    return (await response.json()) as unknown;
  } catch {
    return null;
  }
}

const text = (value: unknown, fallback = ""): string => (typeof value === "string" ? value : fallback);
const nullableText = (value: unknown): string | null => (typeof value === "string" && value !== "" ? value : null);
const count = (value: unknown): number | null =>
  typeof value === "number" && Number.isFinite(value) && value > 0 ? value : null;

function parseEntry(value: unknown): ActivityEntry | null {
  if (!isRecord(value) || typeof value["install"] !== "string" || typeof value["name"] !== "string") return null;
  const artwork = isRecord(value["artwork"]) ? value["artwork"] : {};
  const devices = value["devices"];
  return {
    install: value["install"],
    name: value["name"],
    version: text(value["version"]),
    description: text(value["description"]),
    icon: nullableText(artwork["icon"]),
    cover: nullableText(artwork["cover"]),
    running: value["running"] === true,
    supported: value["supported"] !== false,
    devices: Array.isArray(devices) ? stringArray(devices) : null,
    seats: count(value["seats"]),
    permissions: stringArray(value["permissions"]),
  };
}

function parseOwner(value: unknown): ActivityOwner | null {
  if (!isRecord(value) || typeof value["id"] !== "string" || value["id"] === "") return null;
  return { id: value["id"], name: text(value["name"]), avatar: nullableText(value["avatar"]) };
}

export function parseLibrary(value: unknown): ActivityLibrary {
  const record = isRecord(value) ? value : {};
  const entries = Array.isArray(record["activities"]) ? record["activities"] : [];
  return {
    activities: entries.map(parseEntry).filter((entry): entry is ActivityEntry => entry !== null),
    live: parseActivitySummary(record["live"]),
    owner: parseOwner(record["owner"]),
    viewerIsOwner: record["viewerIsOwner"] === true,
    voiceOnly: stringArray(record["voiceOnly"]),
  };
}

function parseLaunch(value: unknown, install: string | undefined): ActivityLaunch {
  const record = isRecord(value) ? value : {};
  const resolved = text(record["install"], install ?? "");
  const access = text(record["access"]);
  const src = text(record["src"]);
  if (resolved === "" || access === "" || src === "") {
    throw new ActivityRequestError(200, null, "The launch answer is missing its frame.");
  }
  return {
    install: resolved,
    access,
    src,
    title: text(record["title"]),
    permissions: stringArray(record["permissions"]),
    host: nullableText(record["host"]),
    youtube: record["youtube"] === true,
  };
}

export interface Lease {
  install: string;
  access: string;
}

export const activitiesApi = {
  /** What the place may run. With a lease it also keeps that seat alive. */
  async list(place: string, input: { device: DeviceClass; lease?: Lease }, signal?: AbortSignal) {
    return parseLibrary(await post(place, "list", input, signal));
  },

  /** Starts one, or with no install joins what runs on the channel. */
  async launch(
    place: string,
    input: { install?: string; device: DeviceClass; replace?: boolean },
    signal?: AbortSignal,
  ): Promise<ActivityLaunch> {
    return parseLaunch(await post(place, "launch", input, signal), input.install);
  },

  async leave(place: string, lease: Lease): Promise<void> {
    await post(place, "leave", lease);
  },

  /** Ends it for everyone; on a voice channel only its host may. */
  async end(place: string, input?: { install: string }): Promise<void> {
    await post(place, "end", input);
  },

  /** A request a frame makes of its own activity. */
  async act(
    place: string,
    kind: "room" | "tile",
    input: Lease & { action: Record<string, unknown> },
    signal?: AbortSignal,
  ): Promise<Record<string, unknown>> {
    const value = await post(place, kind, input, signal);
    return isRecord(value) ? value : {};
  },

  /** Every running activity on one server's voice channels, by channel. */
  async live(serverId: string, signal?: AbortSignal): Promise<Map<string, ActivitySummary>> {
    const response = await send(
      `/api/activities/live?server=${encodeURIComponent(serverId)}`,
      { method: "GET", headers: { Accept: "application/json" } },
      signal,
    );
    let body: unknown = null;
    try {
      body = await response.json();
    } catch {
      // An unreadable answer is an empty one.
    }
    const running = new Map<string, ActivitySummary>();
    for (const row of Array.isArray(body) ? body : []) {
      if (!isRecord(row) || typeof row["channelId"] !== "string") continue;
      const activity = parseActivitySummary(row["activity"]);
      if (activity !== null) running.set(row["channelId"], activity);
    }
    return running;
  },
};

/** Whether a summary has no seat left for somebody outside it. */
export function isFull(summary: ActivitySummary | null | undefined): boolean {
  return summary?.seats != null && summary.participants.length >= summary.seats;
}
