import { useMemo } from "react";

import type { PollCallbacks } from "@/components/room/poll/PollCard";
import { useStableHandler } from "@/hooks/useStableHandler";
import { seenOf } from "@/lib/polls/pollState";
import type { Message, Poll, PollsPort } from "@/ports";

import type { ChatDispatch } from "./shared";

/** What a poll on screen can ask for. One object for the life of the chat. */
export type PollActions = PollCallbacks;

interface Options {
  polls: PollsPort;
  dispatch: ChatDispatch;
  /** The open channel; its threads' messages are kept with it. */
  channelId: string | null;
  messages: Readonly<Record<string, readonly Message[]>>;
  /** Pictures of the members, by id. */
  faces: ReadonlyMap<string, string>;
}

/**
 * Votes and ends. What they answer is merged into the poll's message, but
 * only where nothing newer arrived while they were asked for.
 */
export function usePollActions({ polls, dispatch, channelId, messages, faces }: Options): PollActions {
  const answered = async (messageId: string, ask: () => Promise<Poll>) => {
    const loaded = channelId === null ? undefined : messages[channelId]?.find((message) => message.id === messageId);
    const seen = loaded?.poll === undefined ? null : seenOf(loaded.poll);
    const poll = await ask();
    if (channelId === null || seen === null) return;
    dispatch({ type: "poll", channelId, messageId, change: { kind: "answered", poll, seen } });
  };

  const vote = useStableHandler((messageId: string, answers: readonly number[]) =>
    answered(messageId, () => polls.vote(messageId, answers)),
  );
  const end = useStableHandler((messageId: string) => answered(messageId, () => polls.end(messageId)));
  const faceOf = useStableHandler((userId: string) => faces.get(userId));

  return useMemo(
    () => ({
      vote,
      end,
      voters: (messageId, answerId, cursor) => polls.voters(messageId, answerId, cursor),
      faceOf,
    }),
    [vote, end, faceOf, polls],
  );
}
