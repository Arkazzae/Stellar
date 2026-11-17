import type { ApiClient } from "@/lib/api/client";
import type {
  Message,
  MessageId,
  MessageReaction,
  MessageReactionChanged,
  UserId,
} from "@/lib/api/types";
import type { RealtimeClient } from "@/lib/realtime/types";
import type { MessageActionsPort, Reaction, Unsubscribe } from "@/ports";

type ReactionHandler = (messageId: MessageId, reactions: Reaction[]) => void;

const copyAll = (reactions: MessageReaction[]): MessageReaction[] =>
  reactions.map((reaction) => ({ ...reaction }));

/**
 * The reactions every message currently has, fed by history pages and by hub
 * aggregates. HTTP reads repair missed events without replacing newer live
 * changes or a snapshot from a more recent request.
 */
export class ReactionStore {
  readonly #byMessage = new Map<MessageId, MessageReaction[]>();
  readonly #handlers = new Set<ReactionHandler>();
  #selfUserId: UserId | undefined;
  #revision = 0;
  readonly #changed = new Map<MessageId, Map<string, number>>();
  readonly #snapshots = new Map<MessageId, number>();

  /** Capture before starting an HTTP read; newer live aggregates still win. */
  checkpoint(): number {
    return ++this.#revision;
  }

  setSelfUserId(userId: UserId): void {
    this.#selfUserId = userId;
  }

  /**
   * HTTP snapshots repair missed events, including removed reactions. Live
   * changes after the request began win per emoji, rather than preventing the
   * rest of the snapshot from repairing stale counts.
   */
  ingest(messages: Message[], checkpoint?: number): void {
    for (const message of messages) {
      if (checkpoint === undefined && this.#byMessage.has(message.id)) continue;
      if (checkpoint !== undefined) {
        if ((this.#snapshots.get(message.id) ?? -1) > checkpoint) continue;
        this.#snapshots.set(message.id, checkpoint);
      }
      const current = this.#byMessage.get(message.id) ?? [];
      const next = new Map(
        (message.reactions ?? []).map((reaction) => [reaction.emoji, { ...reaction }]),
      );
      if (checkpoint !== undefined) {
        for (const [emoji, revision] of this.#changed.get(message.id) ?? []) {
          if (revision <= checkpoint) continue;
          const live = current.find((reaction) => reaction.emoji === emoji);
          if (live) next.set(emoji, { ...live });
          else next.delete(emoji);
        }
      }
      const result = [...next.values()];
      const changed = current.length !== result.length || current.some((reaction) => {
        const replacement = next.get(reaction.emoji);
        return replacement?.count !== reaction.count || replacement.mine !== reaction.mine;
      });
      this.#byMessage.set(message.id, result);
      if (changed) {
        for (const handler of this.#handlers) handler(message.id, copyAll(result));
      }
    }
  }

  get(messageId: MessageId): MessageReaction[] {
    return copyAll(this.#byMessage.get(messageId) ?? []);
  }

  subscribe(handler: ReactionHandler): Unsubscribe {
    this.#handlers.add(handler);
    return () => this.#handlers.delete(handler);
  }

  /**
   * Applies a per-emoji aggregate. The hub says who changed it but not who else
   * reacted, so `mine` follows the update only when it is about the signed-in
   * user. An aggregate that changes nothing is not announced.
   */
  apply(update: MessageReactionChanged): void {
    const changes = this.#changed.get(update.messageId) ?? new Map<string, number>();
    changes.set(update.emoji, ++this.#revision);
    this.#changed.set(update.messageId, changes);
    const current = this.#byMessage.get(update.messageId) ?? [];
    const existing = current.find((reaction) => reaction.emoji === update.emoji);
    const mine =
      update.userId === this.#selfUserId ? update.added : (existing?.mine ?? false);

    const unchanged =
      existing === undefined
        ? update.count === 0
        : existing.count === update.count && existing.mine === mine;
    if (unchanged) return;

    const others = current.filter((reaction) => reaction.emoji !== update.emoji);
    const next =
      update.count > 0
        ? [...others, { emoji: update.emoji, count: update.count, mine }]
        : others;
    this.#byMessage.set(update.messageId, next);
    for (const handler of this.#handlers) handler(update.messageId, copyAll(next));
  }

  reset(): void {
    this.#byMessage.clear();
    this.#changed.clear();
    this.#snapshots.clear();
    this.#revision = 0;
    this.#selfUserId = undefined;
  }
}

/**
 * `MessageActionsPort` over HTTP, with reaction state shared with the hub.
 *
 * Edits, deletions and pins need no local state: the hub echoes them to every
 * client through the message events the chat adapter already forwards.
 */
export function createMessageActionsAdapter(
  api: ApiClient,
  realtime: RealtimeClient,
  store: ReactionStore,
): MessageActionsPort {
  const stopListening = realtime.on("MessageReactionChanged", (update) => {
    store.apply(update);
  });

  return {
    getReactions: (messageId) => store.get(messageId),
    onReactionsChanged: (handler) => store.subscribe(handler),

    async react(messageId, emoji) {
      store.apply(await api.setMessageReaction(messageId, emoji, true));
    },

    async unreact(messageId, emoji) {
      store.apply(await api.setMessageReaction(messageId, emoji, false));
    },

    async edit(messageId, content) {
      await api.editMessage(messageId, content);
    },

    async remove(messageId) {
      await api.deleteMessage(messageId);
    },

    async setPinned(messageId, pinned) {
      await api.setMessagePinned(messageId, pinned);
    },

    dispose: stopListening,
  };
}
