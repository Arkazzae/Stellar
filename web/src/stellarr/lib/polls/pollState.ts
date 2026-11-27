import type { Message } from "@/lib/api/types";
import type { Poll, PollUpdate, PollVoteChange } from "@/ports/polls";

/*
 * The rules of a poll that do not depend on how it is drawn: when it is over,
 * what a pick does, how a result reads, and how the changes that arrive from
 * the server are put together without an older one undoing a newer one.
 */

/** Over once it was ended or its time has run out, whichever came first. */
export function isPollClosed(poll: Poll, now: Date): boolean {
  return poll.endedAt !== undefined || Date.parse(poll.endsAt) <= now.getTime();
}

/** When it ended: the moment it was ended, or the end of its time. */
export function pollEndedAt(poll: Poll): string {
  return poll.endedAt ?? poll.endsAt;
}

/** The reader's answers as far as they are known here. */
export function myAnswers(poll: Poll): readonly number[] {
  return poll.mine ?? [];
}

/** One answer chosen in a pick: a single-answer poll swaps, as a radio does; another toggles. */
export function pickAnswer(picked: readonly number[], answerId: number, multiple: boolean): number[] {
  if (!multiple) return [answerId];
  return picked.includes(answerId)
    ? picked.filter((id) => id !== answerId)
    : [...picked, answerId].sort((a, b) => a - b);
}

/**
 * An answer's share of the people who voted. With several answers allowed the
 * shares add up to more than a hundred, as each person counts once per answer.
 */
export function shareOf(votes: number, voters: number): number {
  return voters === 0 ? 0 : Math.round((votes / voters) * 100);
}

/** The answers with the most votes; none while nobody has voted, and all of a tie. */
export function leadingAnswers(poll: Poll): ReadonlySet<number> {
  const most = Math.max(0, ...poll.answers.map((answer) => answer.votes));
  if (most === 0) return new Set();
  return new Set(poll.answers.filter((answer) => answer.votes === most).map((answer) => answer.id));
}

/** Absolute counts from everyone's channel; one older than what is shown is dropped. */
export function applyPollUpdate(poll: Poll, update: PollUpdate): Poll {
  if (poll.sequence !== undefined && update.sequence < poll.sequence) return poll;
  const votes = new Map(update.answers.map((answer) => [answer.id, answer.votes]));
  const { endedAt: _endedAt, endedBy: _endedBy, ...rest } = poll;
  const endedAt = update.endedAt ?? poll.endedAt;
  const endedBy = update.endedBy ?? poll.endedBy;
  return {
    ...rest,
    answers: poll.answers.map((answer) => ({ ...answer, votes: votes.get(answer.id) ?? answer.votes })),
    voters: update.voters,
    ...(endedAt === undefined ? {} : { endedAt }),
    ...(endedBy === undefined ? {} : { endedBy }),
    sequence: update.sequence,
  };
}

/** The reader's own vote, from one of their sessions; ordered on its own. */
export function applyVoteChange(poll: Poll, change: PollVoteChange): Poll {
  if (poll.mineSequence !== undefined && change.sequence < poll.mineSequence) return poll;
  return { ...poll, mine: [...change.mine], mineSequence: change.sequence };
}

/** What was known of a poll when a vote or an end was asked for. */
export interface PollSeen {
  sequence: number | undefined;
  mineSequence: number | undefined;
}

export function seenOf(poll: Poll): PollSeen {
  return { sequence: poll.sequence, mineSequence: poll.mineSequence };
}

/**
 * The poll a vote or an end answered with. It carries no order of its own, so
 * its counts are taken only when nothing newer arrived while it was asked for,
 * and the same for the reader's vote. An end is never undone.
 */
export function applyPollAnswer(poll: Poll, answered: Poll, seen: PollSeen): Poll {
  const countsCurrent = poll.sequence === seen.sequence;
  const mineCurrent = poll.mineSequence === seen.mineSequence;
  const endedAt = poll.endedAt ?? answered.endedAt;
  const endedBy = poll.endedBy ?? answered.endedBy;
  const { endedAt: _endedAt, endedBy: _endedBy, ...rest } = poll;
  return {
    ...rest,
    ...(countsCurrent ? { answers: answered.answers, voters: answered.voters } : {}),
    ...(mineCurrent && answered.mine !== undefined ? { mine: answered.mine } : {}),
    ...(endedAt === undefined ? {} : { endedAt }),
    ...(endedBy === undefined ? {} : { endedBy }),
  };
}

/** Something that changes a poll already on screen. */
export type PollChange =
  | { kind: "updated"; update: PollUpdate }
  | { kind: "vote"; change: PollVoteChange }
  | { kind: "answered"; poll: Poll; seen: PollSeen };

export function applyPollChange(poll: Poll, change: PollChange): Poll {
  switch (change.kind) {
    case "updated":
      return applyPollUpdate(poll, change.update);
    case "vote":
      return applyVoteChange(poll, change.change);
    case "answered":
      return applyPollAnswer(poll, change.poll, change.seen);
  }
}

/**
 * A message pushed to everyone carries no reader's vote, and knows nothing of
 * the changes already applied here: what was known of the poll stays.
 */
export function keepLoadedPoll(incoming: Message, existing: Message | undefined): Message {
  const known = existing?.poll;
  const poll = incoming.poll;
  if (known === undefined || poll === undefined) return incoming;
  const mine = poll.mine ?? known.mine;
  return {
    ...incoming,
    poll: {
      ...poll,
      ...(mine === undefined ? {} : { mine }),
      ...(known.sequence === undefined ? {} : { sequence: known.sequence }),
      ...(known.mineSequence === undefined ? {} : { mineSequence: known.mineSequence }),
    },
  };
}
