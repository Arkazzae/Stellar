/*
 * Polls: a question with answers to choose from, sent as a message in a
 * server's channel or thread. The poll rides on its message; this port is
 * what can be done with one after it went out, and how its changes arrive.
 */

/** How long a poll stays open, from a fixed list. */
export type PollDuration = "1h" | "4h" | "8h" | "1d" | "3d" | "1w";

export const POLL_DURATIONS: readonly PollDuration[] = ["1h", "4h", "8h", "1d", "3d", "1w"];

export const DEFAULT_POLL_DURATION: PollDuration = "1d";

/** The limits a poll is sent within; lengths count as `String.length` does, after trimming. */
export const POLL_LIMITS = Object.freeze({
  question: 300,
  answer: 55,
  minAnswers: 2,
  maxAnswers: 10,
});

export interface PollAnswer {
  /** Its place in the poll, from 1. */
  id: number;
  text: string;
  /** Named as a reaction is: the character, or `:shortcode:` for a server emoji. */
  emoji?: string;
  votes: number;
}

export interface Poll {
  question: string;
  answers: PollAnswer[];
  /** More than one answer may be chosen. */
  multiple: boolean;
  /** ISO 8601. The poll ends then, whatever has been heard since. */
  endsAt: string;
  /** ISO 8601, once it ended, on time or early. */
  endedAt?: string;
  /** Who ended it early; absent when it ended on time. */
  endedBy?: string;
  /** How many people voted. */
  voters: number;
  /**
   * The reader's own answers; empty when they have not voted. Absent when a
   * copy shared with everyone could not say, so the one already known stays.
   */
  mine?: number[];
  /** The last change to the counts applied here, so an older one arriving late is dropped. */
  sequence?: number;
  /** The same for the reader's own vote, which arrives on its own. */
  mineSequence?: number;
}

/** A poll as it is written, before it is sent. */
export interface PollDraft {
  question: string;
  answers: { text: string; emoji?: string }[];
  multiple: boolean;
  duration: PollDuration;
}

/** The counts after a vote or an end, as everyone who sees the channel hears them. */
export interface PollUpdate {
  channelId: string;
  messageId: string;
  sequence: number;
  answers: { id: number; votes: number }[];
  voters: number;
  endedAt?: string;
  endedBy?: string;
}

/** The reader's own vote, changed in this or another of their sessions. */
export interface PollVoteChange {
  channelId: string;
  messageId: string;
  sequence: number;
  mine: number[];
}

export interface PollVoter {
  id: string;
  displayName: string;
}

export interface PollVoterPage {
  voters: PollVoter[];
  nextCursor: string | null;
}

/** Why a vote or an end did not happen. */
export type PollRefusalReason = "ended" | "selection" | "forbidden" | "other";

export class PollRefusedError extends Error {
  readonly reason: PollRefusalReason;

  constructor(reason: PollRefusalReason, options?: ErrorOptions) {
    super(`Poll action refused: ${reason}`, options);
    this.name = "PollRefusedError";
    this.reason = reason;
  }
}

/** Which part of a draft the server refused. */
export type PollDraftField = "question" | "answers" | "duration" | "attachments" | { answer: number };

/** A poll the server would not post, with the fields it named. */
export class PollDraftRefusedError extends Error {
  readonly fields: readonly PollDraftField[];

  constructor(fields: readonly PollDraftField[], options?: ErrorOptions) {
    super("Poll refused", options);
    this.name = "PollDraftRefusedError";
    this.fields = fields;
  }
}

export interface PollsPort {
  /** Sets the reader's whole vote; an empty list takes it back. Answers with the poll. */
  vote(messageId: string, answers: readonly number[]): Promise<Poll>;
  /** Ends the poll now; for its author and whoever may manage messages. */
  end(messageId: string): Promise<Poll>;
  /** A page of the people who chose `answerId`, earliest first. */
  voters(messageId: string, answerId: number, cursor?: string): Promise<PollVoterPage>;
  onPollUpdated(handler: (update: PollUpdate) => void): () => void;
  onPollVoteChanged(handler: (change: PollVoteChange) => void): () => void;
}
