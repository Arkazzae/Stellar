/*
 * Polls on the wire: the poll on a message, what a new one is sent as, the
 * vote, end and voters routes, the live events and how a refusal reads.
 * Everything that depends on the API's contract for polls is in this file.
 */

import {
  PollDraftRefusedError,
  PollRefusedError,
  type Poll,
  type PollDraft,
  type PollDraftField,
  type PollUpdate,
  type PollVoteChange,
  type PollVoter,
  type PollVoterPage,
} from "@/ports/polls";

import { ApiError } from "../errors";
import { definedField, expectArray, expectNumber, expectRecord, expectString, isRecord, optionalString } from "../expect";
import { getJson, route, sendJson, withQuery, type Parse } from "./http";

/** The live events. */
export const POLL_EVENTS = Object.freeze({
  /** The counts, to everyone who sees the channel. */
  updated: "PollUpdated",
  /** The reader's own vote, to their own connections only. */
  voteChanged: "PollVoteChanged",
} as const);

/** How many voters one hover asks for. */
const VOTERS_PAGE = 20;

const numbers = (value: unknown): number[] =>
  Array.isArray(value) ? value.filter((item): item is number => Number.isSafeInteger(item)) : [];

/** The poll a message carries. Every missing value comes as null; the domain leaves it out. */
export const parsePoll: Parse<Poll> = (value, path) => {
  const record = expectRecord(value, path);
  const answers = expectArray(record["answers"], `${path}.answers`).map((item, index) => {
    const at = `${path}.answers[${index}]`;
    const answer = expectRecord(item, at);
    return {
      id: expectNumber(answer, "id", at),
      text: expectString(answer, "text", at),
      ...definedField("emoji", optionalString(answer, "emoji")),
      votes: expectNumber(answer, "votes", at),
    };
  });
  // Null in a copy shared with everyone: not this reader's to know.
  const mine = record["mine"];
  return {
    question: expectString(record, "question", path),
    answers,
    multiple: record["multiple"] === true,
    endsAt: expectString(record, "endsAt", path),
    ...definedField("endedAt", optionalString(record, "endedAt")),
    ...definedField("endedBy", optionalString(record, "endedBy")),
    voters: expectNumber(record, "voters", path),
    ...(Array.isArray(mine) ? { mine: numbers(mine) } : {}),
  };
};

/**
 * The poll on a message, if it has one. A poll this client cannot read is
 * left off its message, which still shows, rather than losing the whole page.
 */
export function readPoll(value: unknown, path: string): Poll | undefined {
  if (value === null || value === undefined) return undefined;
  try {
    return parsePoll(value, path);
  } catch {
    return undefined;
  }
}

/** `PollUpdated`; null for a payload this client cannot read. */
export function parsePollUpdated(value: unknown): PollUpdate | null {
  if (!isRecord(value)) return null;
  const { channelId, messageId, sequence, voters, answers } = value;
  if (typeof channelId !== "string" || typeof messageId !== "string") return null;
  if (typeof sequence !== "number" || typeof voters !== "number" || !Array.isArray(answers)) return null;
  const counts = answers.flatMap((answer) =>
    isRecord(answer) && typeof answer["id"] === "number" && typeof answer["votes"] === "number"
      ? [{ id: answer["id"], votes: answer["votes"] }]
      : [],
  );
  return {
    channelId,
    messageId,
    sequence,
    answers: counts,
    voters,
    ...definedField("endedAt", optionalString(value, "endedAt")),
    ...definedField("endedBy", optionalString(value, "endedBy")),
  };
}

/** `PollVoteChanged`; null for a payload this client cannot read. */
export function parsePollVoteChanged(value: unknown): PollVoteChange | null {
  if (!isRecord(value)) return null;
  const { channelId, messageId, sequence, mine } = value;
  if (typeof channelId !== "string" || typeof messageId !== "string") return null;
  if (typeof sequence !== "number" || !Array.isArray(mine)) return null;
  return { channelId, messageId, sequence, mine: numbers(mine) };
}

/** The `poll` member of a new message. Lengths are trimmed, as the server counts them. */
export interface PollBody {
  question: string;
  answers: { text: string; emoji: string | null }[];
  multiple: boolean;
  duration: PollDraft["duration"];
}

export function pollBody(draft: PollDraft): PollBody {
  return {
    question: draft.question.trim(),
    answers: draft.answers.map((answer) => ({ text: answer.text.trim(), emoji: answer.emoji ?? null })),
    multiple: draft.multiple,
    duration: draft.duration,
  };
}

const parseVoter: Parse<PollVoter> = (value, path) => {
  const record = expectRecord(value, path);
  const id = expectString(record, "id", path);
  return { id, displayName: optionalString(record, "displayName") ?? id };
};

const parseVoterPage: Parse<PollVoterPage> = (value, path) => {
  const record = expectRecord(value, path);
  return {
    voters: expectArray(record["voters"], `${path}.voters`).map((item) => parseVoter(item, `${path}.voters[]`)),
    nextCursor: optionalString(record, "nextCursor") ?? null,
  };
};

export interface PollApiClient {
  votePoll(messageId: string, answers: readonly number[]): Promise<Poll>;
  endPoll(messageId: string): Promise<Poll>;
  pollVoters(messageId: string, answerId: number, cursor?: string): Promise<PollVoterPage>;
}

/** Reads a refused vote or end as a reason the card can say. */
function refused(error: unknown): never {
  if (!(error instanceof ApiError)) throw error;
  const reason =
    error.code === "poll-ended"
      ? "ended"
      : error.fields?.["answers"] !== undefined
        ? "selection"
        : error.status === 403 || error.status === 404
          ? "forbidden"
          : "other";
  throw new PollRefusedError(reason, { cause: error });
}

export function pollCalls(): PollApiClient {
  return {
    votePoll: (messageId, answers) =>
      sendJson("PUT", route`/api/messages/${messageId}/poll/vote`, { answers: [...answers] }, parsePoll).catch(refused),

    endPoll: (messageId) =>
      sendJson("POST", route`/api/messages/${messageId}/poll/end`, undefined, parsePoll).catch(refused),

    pollVoters: (messageId, answerId, cursor) =>
      getJson(
        withQuery(route`/api/messages/${messageId}/poll/answers/${String(answerId)}/voters`, {
          limit: VOTERS_PAGE,
          cursor,
        }),
        parseVoterPage,
      ),
  };
}

/**
 * The fields of a new poll the server refused, read from its validation
 * problem, or null when the refusal was about something else.
 */
export function pollDraftRefusal(error: unknown): PollDraftRefusedError | null {
  if (!(error instanceof ApiError) || error.status !== 400 || error.fields === undefined) return null;
  const fields: PollDraftField[] = [];
  for (const key of Object.keys(error.fields)) {
    const answer = /^poll\.answers\[(\d+)\]$/.exec(key);
    if (answer !== null) fields.push({ answer: Number(answer[1]) });
    else if (key === "poll.question") fields.push("question");
    else if (key === "poll.answers") fields.push("answers");
    else if (key === "poll.duration") fields.push("duration");
    else if (key === "attachmentIds") fields.push("attachments");
  }
  return fields.length === 0 ? null : new PollDraftRefusedError(fields, { cause: error });
}
