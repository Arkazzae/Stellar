import { POLL_EVENTS, type PollApiClient } from "@/lib/api/calls/polls";
import type { RealtimeClient } from "@/lib/realtime/types";
import type { PollsPort } from "@/ports";

/**
 * `PollsPort` over HTTP and the hub. A poll lives on its message, so nothing
 * is kept here: the chat state merges what arrives into the message it is on.
 */
export function createPollsAdapter(api: PollApiClient, realtime: RealtimeClient): PollsPort {
  return {
    vote: (messageId, answers) => api.votePoll(messageId, answers),
    end: (messageId) => api.endPoll(messageId),
    voters: (messageId, answerId, cursor) => api.pollVoters(messageId, answerId, cursor),
    onPollUpdated: (handler) => realtime.on(POLL_EVENTS.updated, handler),
    onPollVoteChanged: (handler) => realtime.on(POLL_EVENTS.voteChanged, handler),
  };
}
