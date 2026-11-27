/*
 * Scheduled messages on the wire: the routes, the shape the server answers
 * with, the live events and how a refusal reads. Everything that depends on
 * the API's contract for them is in this one file, so a change to that
 * contract is a change here.
 */

import type {
  ScheduledFailureCode,
  ScheduledMessage,
  ScheduledTarget,
  ScheduleRefusalReason,
} from "@/ports/scheduled";
import { SCHEDULED_FAILURE_CODES } from "@/ports/scheduled";

import { parseAttachment, parseMessage } from "../chatParsers";
import { ApiError, statusToKind } from "../errors";
import { expectRecord, expectString, isRecord, optionalList, optionalString } from "../expect";
import { JSON_HEADERS, readJson, request } from "../transport";
import type { Message } from "../types";
import { ACCEPT_JSON, route, type Parse } from "./http";

/** The live events, sent to the author's own connections only. */
export const SCHEDULED_EVENTS = Object.freeze({
  /** Created, edited or failed: the message itself. */
  changed: "ScheduledMessageChanged",
  /** Deleted, or sent: `{ id, sent }`; a sent one arrives as a message with the same id. */
  removed: "ScheduledMessageRemoved",
} as const);

export interface ScheduledBody {
  content: string;
  attachmentIds: string[];
  /** In a channel only; a message in a thread answers the thread's first message. */
  replyToMessageId?: string;
  /** UTC, ending in `Z`: an offset is refused. */
  sendAt: string;
}

export interface ScheduledApiClient {
  listScheduled(signal?: AbortSignal): Promise<ScheduledMessage[]>;
  createScheduled(target: ScheduledTarget, body: ScheduledBody): Promise<ScheduledMessage>;
  updateScheduled(id: string, patch: { content?: string; sendAt?: string }): Promise<ScheduledMessage>;
  deleteScheduled(id: string): Promise<void>;
  sendScheduled(id: string): Promise<Message>;
}

function failureCode(value: unknown): ScheduledFailureCode {
  return SCHEDULED_FAILURE_CODES.find((code) => code === value) ?? "other";
}

function parseTarget(value: unknown, path: string): ScheduledTarget {
  const record = expectRecord(value, path);
  if (record["kind"] === "conversation") {
    return { kind: "conversation", conversationId: expectString(record, "conversationId", path) };
  }
  const serverId = optionalString(record, "serverId");
  const serverName = optionalString(record, "serverName");
  const channelName = optionalString(record, "channelName");
  return {
    kind: "channel",
    channelId: expectString(record, "channelId", path),
    ...(serverId === undefined ? {} : { serverId }),
    ...(serverName === undefined ? {} : { serverName }),
    ...(channelName === undefined ? {} : { channelName }),
  };
}

/** A missing value comes as null; the domain leaves it out. */
export const parseScheduledMessage: Parse<ScheduledMessage> = (value, path) => {
  const record = expectRecord(value, path);
  const failure = record["failure"];
  const replyToMessageId = optionalString(record, "replyToMessageId");
  const sendAt = expectString(record, "sendAt", path);
  return {
    id: expectString(record, "id", path),
    target: parseTarget(record["target"], `${path}.target`),
    content: optionalString(record, "content") ?? "",
    attachments: optionalList(record["attachments"], `${path}.attachments[]`, parseAttachment),
    ...(replyToMessageId === undefined ? {} : { replyToMessageId }),
    sendAt,
    createdAt: optionalString(record, "createdAt") ?? sendAt,
    state: record["state"] === "failed" ? "failed" : "pending",
    ...(isRecord(failure)
      ? { failure: { code: failureCode(failure["code"]), at: optionalString(failure, "at") ?? sendAt } }
      : {}),
  };
};

/** `ScheduledMessageChanged` carries the message itself. */
export function parseScheduledChanged(value: unknown): ScheduledMessage | null {
  try {
    return parseScheduledMessage(value, SCHEDULED_EVENTS.changed);
  } catch {
    return null;
  }
}

export function parseScheduledRemoved(value: unknown): { id: string; sent: boolean } | null {
  if (!isRecord(value) || typeof value["id"] !== "string") return null;
  return { id: value["id"], sent: value["sent"] === true };
}

/**
 * Reads a refused answer. A refused field is a validation problem naming the
 * field in `errors` with no code; a refused state is a problem with a
 * kebab-case `code`; some refusals have no body at all.
 */
async function refusal(response: Response, path: string): Promise<never> {
  let code: string | undefined;
  let fields: Record<string, string> | undefined;
  const text = await response.text().catch(() => "");
  if (text !== "") {
    try {
      const body = JSON.parse(text) as unknown;
      if (isRecord(body)) {
        code = optionalString(body, "code");
        const errors = body["errors"];
        if (isRecord(errors)) {
          fields = Object.fromEntries(
            Object.entries(errors).map(([field, messages]) => [
              field,
              Array.isArray(messages) && typeof messages[0] === "string" ? messages[0] : "",
            ]),
          );
        }
      }
    } catch {
      // A body that is not JSON still has its status.
    }
  }
  throw new ApiError(statusToKind(response.status), `${path} responded ${response.status}`, response.status, {
    ...(code === undefined ? {} : { code }),
    ...(fields === undefined ? {} : { fields }),
    // An empty 409 from "send now" means the message changed at that moment.
    ...(text === "" ? { details: { empty: true } } : {}),
  });
}

async function call<T>(path: string, init: RequestInit, parse: Parse<T>, signal?: AbortSignal): Promise<T> {
  const response = await request(path, init, signal);
  if (!response.ok) return refusal(response, path);
  return parse(await readJson(response, path), path);
}

function json(method: "POST" | "PATCH", body: unknown): RequestInit {
  return { method, headers: JSON_HEADERS, body: JSON.stringify(body) };
}

const parseList: Parse<ScheduledMessage[]> = (value, path) => {
  if (!Array.isArray(value)) throw new ApiError("malformed", `${path} returned an unexpected shape`);
  return value.map((item) => parseScheduledMessage(item, `${path}[]`));
};

export function scheduledCalls(): ScheduledApiClient {
  return {
    listScheduled: (signal) =>
      call("/api/scheduled", { method: "GET", headers: ACCEPT_JSON }, parseList, signal),

    // A channel's route carries the server in the header every `/api/` call
    // gets; a conversation takes no answered message.
    createScheduled: (target, { replyToMessageId, ...body }) =>
      target.kind === "channel"
        ? call(
            route`/api/channels/${target.channelId}/scheduled`,
            json("POST", replyToMessageId === undefined ? body : { ...body, replyToMessageId }),
            parseScheduledMessage,
          )
        : call(
            route`/api/social/conversations/${target.conversationId}/scheduled`,
            json("POST", body),
            parseScheduledMessage,
          ),

    updateScheduled: (id, patch) =>
      call(route`/api/scheduled/${id}`, json("PATCH", patch), parseScheduledMessage),

    deleteScheduled: async (id) => {
      const path = route`/api/scheduled/${id}`;
      const response = await request(path, { method: "DELETE", headers: ACCEPT_JSON });
      // Already gone is what was asked for.
      if (!response.ok && response.status !== 404) await refusal(response, path);
    },

    // 201 for a new post, 200 when the worker had already sent it.
    sendScheduled: (id) =>
      call(route`/api/scheduled/${id}/send`, { method: "POST", headers: ACCEPT_JSON }, (value, path) =>
        parseMessage(value, path),
      ),
  };
}

const FIELD_REASONS: Record<string, ScheduleRefusalReason> = {
  sendat: "time",
  content: "content",
  attachmentids: "attachments",
};

const CODE_REASONS: Record<string, ScheduleRefusalReason> = {
  "scheduled-limit": "limit",
  "scheduled-encrypted": "encrypted",
  "scheduled-failed": "failed",
  "attachment-kind-mismatch": "attachments",
  "encryption-required": "encrypted",
};

/** What a refusal means for the person, from the problem the server answered with. */
export function scheduleRefusalReason(error: unknown): ScheduleRefusalReason {
  if (!(error instanceof ApiError)) return "other";
  const byCode = error.code === undefined ? undefined : CODE_REASONS[error.code];
  if (byCode !== undefined) return byCode;
  for (const field of Object.keys(error.fields ?? {})) {
    const byField = FIELD_REASONS[field.toLowerCase()];
    if (byField !== undefined) return byField;
  }
  if (error.status === 429) return "slowMode";
  if (error.status === 409 && error.details?.["empty"] === true) return "changed";
  if (error.status === 403 || error.status === 404) return "forbidden";
  return "other";
}
