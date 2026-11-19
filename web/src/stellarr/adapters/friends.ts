import type { SocialApiClient } from "@/lib/api/client";
import { ApiError } from "@/lib/api/errors";
import type { Attachment, VoiceOccupant } from "@/lib/api/types";
import { LAZY_ATTACHMENT_PREFIX, type E2eeEvent, type EncryptedUpload } from "@/lib/e2ee/client";
import { measureUpload } from "@/lib/media/measureUpload";
import type { UploadMediaMetadata } from "@/lib/media/probe";
import type { RealtimeClient } from "@/lib/realtime/types";
import type {
  DmConversation,
  DmId,
  Friend,
  FriendRequest,
  FriendsPort,
  Message,
  User,
} from "@/ports";

const NO_MESSAGES: Message[] = [];
const PAGE_SIZE = 50;

/**
 * What the friends adapter needs of the encryption client: turning a direct
 * message into what the reader sees, and sending one encrypted. Without it
 * every message is taken as it came, which is what a plain conversation is.
 */
export interface DmCodec {
  decode(message: Message): Promise<Message>;
  send(
    conversationId: string,
    input: { text: string; uploads: readonly EncryptedUpload[]; clientMessageId: string },
  ): Promise<Message>;
  uploadFile(file: File, metadata: UploadMediaMetadata): Promise<EncryptedUpload>;
  openAttachment(uploadId: string): Promise<{ url: string; type: string } | null>;
  subscribe(listener: (event: E2eeEvent) => void): () => void;
}

const PLAIN: DmCodec = {
  decode: async (message) => message,
  send: () => Promise.reject(new Error("This client cannot encrypt.")),
  uploadFile: () => Promise.reject(new Error("This client cannot encrypt.")),
  openAttachment: async () => null,
  subscribe: () => () => undefined,
};

/**
 * Lines about someone's keys stay in the history but are not news: they do not
 * become the rail's last message, move the conversation, or count as unread.
 */
function isQuiet(message: Message): boolean {
  return message.line?.kind === "device_added" || message.line?.kind === "keys_reset";
}

/** The peer's messages count as unread, and of the lines only a request waiting for an answer. */
function countsAsUnread(message: Message, peerId: string): boolean {
  if (message.author.id !== peerId) return false;
  return message.line === undefined || message.line.kind === "encryption_requested";
}

/** A decoded message that may read differently once keys or files arrive. */
function unfinished(message: Message): boolean {
  return message.sealed !== undefined || message.attachments.some((attachment) => attachment.url === "");
}

/** Thrown before sending a plain upload into an encrypted conversation, or the reverse. */
function kindMismatch(): ApiError {
  return new ApiError("server", "attachments do not match the conversation's encryption", 409, {
    code: "attachment-kind-mismatch",
  });
}

interface DirectCall {
  /** Stable while the call lasts, through mute and speaking updates. */
  id: string;
  participants: readonly VoiceOccupant[];
}

/** `next` itself, or `previous` when both hold the very same items in the same order. */
function sameItems<T>(previous: T[], next: T[]): T[] {
  return previous.length === next.length && next.every((item, index) => item === previous[index])
    ? previous
    : next;
}

/** A room's frame that says what the last one did, mute and names included. */
function sameParticipants(a: readonly VoiceOccupant[], b: readonly VoiceOccupant[]): boolean {
  return (
    a.length === b.length &&
    a.every((participant, index) => {
      const other = b[index];
      return (
        other !== undefined &&
        participant.userId === other.userId &&
        participant.displayName === other.displayName &&
        participant.muted === other.muted
      );
    })
  );
}

function mergeMessages(
  current: readonly Message[],
  incoming: readonly Message[],
): Message[] {
  const byId = new Map(current.map((message) => [message.id, message]));
  for (const message of incoming) byId.set(message.id, message);
  return [...byId.values()].sort(
    (left, right) =>
      left.createdAt.localeCompare(right.createdAt) || left.id.localeCompare(right.id),
  );
}

/** Persisted friends and direct messages over HTTP, kept fresh through SignalR. */
export function createFriendsAdapter(
  api: SocialApiClient,
  realtime: RealtimeClient,
  codec: DmCodec = PLAIN,
): FriendsPort {
  const listeners = new Set<() => void>();
  /** The wire form of every decoded message that may read differently later. */
  const pending = new Map<string, Message>();
  /** This session's encrypted uploads by upload id, waiting to be sent. */
  const encryptedUploads = new Map<string, EncryptedUpload>();
  const transcripts = new Map<DmId, Message[]>();
  /** Conversations whose newest page has been fetched at least once. */
  const loadedConversations = new Set<DmId>();
  /** Where the next older page starts; null once the transcript is complete. */
  const olderCursors = new Map<DmId, string | null>();
  const olderLoads = new Map<DmId, Promise<void>>();
  /** Newest-page fetches in flight, so opening one twice asks the server once. */
  const newestLoads = new Map<DmId, Promise<void>>();
  /** Per conversation, the newest message already reported as read. */
  const reportedRead = new Map<DmId, string>();
  const calls = new Map<DmId, DirectCall>();
  let callCount = 0;

  let friends: Friend[] = [];
  let requests: FriendRequest[] = [];
  let blocked: User[] = [];
  let conversations: DmConversation[] = [];

  const emit = (): void => {
    for (const listener of listeners) listener();
  };

  const remembered = (wire: Message, decoded: Message): Message => {
    if (unfinished(decoded)) pending.set(wire.id, wire);
    else pending.delete(wire.id);
    return decoded;
  };

  /** Plain messages and lines need no work and are taken as they are. */
  const decode = async (message: Message): Promise<Message> =>
    message.encrypted === undefined ? message : remembered(message, await codec.decode(message));

  const decodeAll = (messages: readonly Message[]): Promise<Message[]> => Promise.all(messages.map(decode));

  const replaceMessage = (decoded: Message): void => {
    const transcript = transcripts.get(decoded.channelId);
    if (transcript !== undefined && transcript.some((item) => item.id === decoded.id)) {
      transcripts.set(
        decoded.channelId,
        transcript.map((item) => (item.id === decoded.id ? decoded : item)),
      );
    }
    conversations = conversations.map((conversation) =>
      conversation.lastMessage?.id === decoded.id ? { ...conversation, lastMessage: decoded } : conversation,
    );
  };

  /** Keys or files arrived: whatever was sealed or still loading is decoded again. */
  const decodeAgain = async (event: E2eeEvent): Promise<void> => {
    const wires = [...pending.values()].filter((wire) =>
      event.type === "attachments"
        ? wire.id === event.messageId
        : event.type === "keys" && (event.conversationId === undefined || wire.channelId === event.conversationId),
    );
    if (wires.length === 0) return;
    const decoded = await Promise.all(wires.map((wire) => codec.decode(wire).then((out) => remembered(wire, out))));
    for (const message of decoded) replaceMessage(message);
    emit();
  };

  /**
   * Each conversation with the call it has now. One whose call did not change
   * is handed back as it was, and so is the list when none did.
   */
  const withCalls = (items: DmConversation[]): DmConversation[] =>
    sameItems(
      items,
      items.map((item) => {
        const call = calls.get(item.id);
        if (item.callId === call?.id && item.callParticipants === call?.participants) return item;
        const { callParticipants: _participants, callId: _id, ...conversation } = item;
        return call === undefined
          ? conversation
          : { ...conversation, callParticipants: call.participants, callId: call.id };
      }),
    );

  let inflight: Promise<void> | null = null;
  let reloadAfter = false;
  let pendingPresence: Map<string, User["presence"]> | null = null;

  /** Concurrent callers share one request; a request made meanwhile reruns it. */
  const load = (): Promise<void> => {
    if (inflight !== null) {
      reloadAfter = true;
      return inflight;
    }
    const updates = new Map<string, User["presence"]>();
    pendingPresence = updates;
    inflight = (async () => {
      const state = await api.getSocialState();
      const lastMessages = await decodeAll(
        state.conversations.flatMap((conversation) =>
          conversation.lastMessage === undefined ? [] : [conversation.lastMessage],
        ),
      );
      const decodedLast = new Map(lastMessages.map((message) => [message.id, message]));
      const fresh = (user: User): User => {
        const presence = updates.get(user.id);
        return presence === undefined ? user : { ...user, presence };
      };
      friends = state.friends.map((friend) => ({ ...friend, user: fresh(friend.user) }));
      requests = state.requests.map((request) => ({ ...request, user: fresh(request.user) }));
      blocked = state.blocked;
      conversations = withCalls(state.conversations.map((conversation) => {
        const last = conversation.lastMessage;
        return {
          ...conversation,
          peer: fresh(conversation.peer),
          ...(last === undefined ? {} : { lastMessage: decodedLast.get(last.id) ?? last }),
        };
      }));
      emit();
    })().finally(() => {
      inflight = null;
      pendingPresence = null;
      if (reloadAfter) {
        reloadAfter = false;
        void load().catch(() => undefined);
      }
    });
    return inflight;
  };

  const loadNewest = async (id: DmId): Promise<void> => {
    // Buffer pushes while the first page is in flight. Conversations that
    // have never been opened do not otherwise need a transcript cache.
    if (!transcripts.has(id)) transcripts.set(id, []);
    const page = await api.listDirectMessages(id, { limit: PAGE_SIZE });
    // A reconnect starts a new server-side snapshot. The messages already on
    // screen stay, but paging continues from the fresh cursor: keeping the old
    // one could skip a page that arrived while the connection was down.
    olderCursors.set(id, page.nextCursor);
    const messages = await decodeAll(page.messages);
    const loaded = transcripts.get(id) ?? NO_MESSAGES;
    transcripts.set(id, mergeMessages(messages, loaded));
    loadedConversations.add(id);
    emit();
  };

  const loadConversation = (id: DmId): Promise<void> => {
    if (loadedConversations.has(id)) return Promise.resolve();
    const running = newestLoads.get(id);
    if (running !== undefined) return running;

    const operation = loadNewest(id).finally(() => newestLoads.delete(id));
    newestLoads.set(id, operation);
    return operation;
  };

  const loadOlder = (id: DmId): Promise<void> => {
    const running = olderLoads.get(id);
    if (running !== undefined) return running;
    const cursor = olderCursors.get(id);
    if (cursor === undefined || cursor === null) return Promise.resolve();

    const operation = api
      .listDirectMessages(id, { cursor, limit: PAGE_SIZE })
      .then(async (page) => {
        const messages = await decodeAll(page.messages);
        transcripts.set(id, mergeMessages(messages, transcripts.get(id) ?? NO_MESSAGES));
        olderCursors.set(id, page.nextCursor);
      })
      .finally(() => {
        olderLoads.delete(id);
        emit();
      });
    olderLoads.set(id, operation);
    emit();
    return operation;
  };

  const changeThenReload = async (change: () => Promise<void>): Promise<void> => {
    await change();
    await load();
  };

  const arrived = (message: Message): void => {
    const id = message.channelId;
    const transcript = transcripts.get(id);
    if (transcript !== undefined && !transcript.some((item) => item.id === message.id)) {
      transcripts.set(id, mergeMessages(transcript, [message]));
    }
    if (!isQuiet(message)) {
      conversations = conversations.map((conversation) => {
        if (conversation.id !== id) return conversation;
        const newer =
          conversation.lastMessage === undefined || conversation.lastMessage.createdAt <= message.createdAt;
        return {
          ...conversation,
          ...(newer ? { lastMessage: message, updatedAt: message.createdAt } : {}),
          unread: countsAsUnread(message, conversation.peer.id) ? conversation.unread + 1 : conversation.unread,
        };
      });
    }
    emit();
  };

  let lastStatus = realtime.getStatus();

  const unsubscribers = [
    realtime.on("PresenceChanged", ({ userId, presence }) => {
      pendingPresence?.set(userId, presence);
      // Only the entries of the person whose presence moved are replaced; a
      // stranger's presence changes no list, and nobody is told.
      const moved = (user: User) => user.id === userId && user.presence !== presence;
      const nextFriends = sameItems(
        friends,
        friends.map((friend) =>
          moved(friend.user) ? { ...friend, user: { ...friend.user, presence } } : friend,
        ),
      );
      const nextRequests = sameItems(
        requests,
        requests.map((request) =>
          moved(request.user) ? { ...request, user: { ...request.user, presence } } : request,
        ),
      );
      const nextConversations = sameItems(
        conversations,
        conversations.map((conversation) =>
          moved(conversation.peer)
            ? { ...conversation, peer: { ...conversation.peer, presence } }
            : conversation,
        ),
      );
      if (
        nextFriends === friends &&
        nextRequests === requests &&
        nextConversations === conversations
      ) {
        return;
      }
      friends = nextFriends;
      requests = nextRequests;
      conversations = nextConversations;
      emit();
    }),
    realtime.on("SocialChanged", () => {
      void load().catch(() => undefined);
    }),

    realtime.on("DirectMessageCreated", (message) => {
      // A plain message is applied at once, an encrypted one once it is
      // decrypted; the sorted merge puts it in its place either way.
      if (message.encrypted === undefined) arrived(message);
      else void decode(message).then(arrived, () => undefined);
    }),

    realtime.on("DirectEncryptionChanged", ({ conversationId, encryption }) => {
      conversations = conversations.map((conversation) =>
        conversation.id === conversationId ? { ...conversation, encryption } : conversation,
      );
      emit();
    }),

    realtime.on("VoiceParticipantChanged", (event) => {
      // Public channels share this event. Remembering their occupancy costs
      // little and closes a race for a brand-new DM, whose call can be
      // announced before the SocialChanged that introduces the conversation.
      // `withCalls` only shows entries whose ids turn out to be DMs.
      const known = calls.get(event.channelId);
      if (event.participants.length === 0) {
        calls.delete(event.channelId);
      } else if (known === undefined || !sameParticipants(known.participants, event.participants)) {
        const id = known?.id ?? `${event.channelId}:${++callCount}`;
        calls.set(event.channelId, { id, participants: event.participants });
      }
      const next = withCalls(conversations);
      if (next === conversations) return;
      conversations = next;
      emit();
    }),

    realtime.subscribeStatus(() => {
      const status = realtime.getStatus();
      const reconnected = status === "connected" && lastStatus !== "connected";
      lastStatus = status;
      if (!reconnected) return;
      // SignalR events are not replayed, so a reconnect refreshes the summaries
      // and every thread that was opened: a message sent during the outage must
      // not leave a transcript behind for good.
      void Promise.all([
        load(),
        ...[...transcripts.keys()].map((id) => loadNewest(id)),
      ]).catch(() => undefined);
    }),
    codec.subscribe((event) => {
      if (event.type !== "status") void decodeAgain(event).catch(() => undefined);
    }),
  ];

  return {
    load,
    listFriends: () => friends,
    listRequests: () => requests,
    listBlocked: () => blocked,
    listConversations: () => conversations,
    listDmMessages: (id) => transcripts.get(id) ?? NO_MESSAGES,
    hasOlderDmMessages: (id) => olderCursors.get(id) != null,
    isLoadingOlderDmMessages: (id) => olderLoads.has(id),
    loadOlderDmMessages: loadOlder,
    loadConversation,

    async openConversation(userId) {
      const id = await api.openDirectConversation(userId);
      await Promise.all([load(), loadConversation(id)]);
      return id;
    },

    async sendDm(id, input) {
      const encrypted = conversations.find((item) => item.id === id)?.encryption?.state === "on";
      let message: Message;
      try {
        if (encrypted) {
          const uploads = input.attachmentIds.map((uploadId) => encryptedUploads.get(uploadId));
          if (uploads.some((upload) => upload === undefined)) throw kindMismatch();
          message = await codec.send(id, {
            text: input.content,
            uploads: uploads.filter((upload) => upload !== undefined),
            clientMessageId: input.clientMessageId,
          });
        } else {
          if (input.attachmentIds.some((uploadId) => encryptedUploads.has(uploadId))) throw kindMismatch();
          message = await api.sendDirectMessage(
            id,
            input.content,
            input.attachmentIds,
            input.clientMessageId,
          );
        }
      } catch (cause) {
        // The switch may have moved under this send; the list says where it is now.
        void load().catch(() => undefined);
        throw cause;
      }
      for (const uploadId of input.attachmentIds) encryptedUploads.delete(uploadId);
      const transcript = transcripts.get(id) ?? NO_MESSAGES;
      if (!transcript.some((item) => item.id === message.id)) {
        transcripts.set(id, [...transcript, message]);
        emit();
      }
      await load();
      return message;
    },

    async uploadAttachment(id, file) {
      const metadata = await measureUpload(file);
      if (conversations.find((item) => item.id === id)?.encryption?.state !== "on") {
        return api.uploadAttachment(file, metadata);
      }
      const upload = await codec.uploadFile(file, metadata);
      encryptedUploads.set(upload.payload.id, upload);
      const attachment: Attachment = {
        id: upload.payload.id,
        url: "",
        name: file.name,
        contentType: upload.payload.type,
        size: file.size,
      };
      return attachment;
    },

    openAttachment(url) {
      if (!url.startsWith(LAZY_ATTACHMENT_PREFIX)) return Promise.resolve(null);
      return codec.openAttachment(url.slice(LAZY_ATTACHMENT_PREFIX.length));
    },

    markRead(id) {
      const conversation = conversations.find((item) => item.id === id);
      // The server answers a read with SocialChanged, and the refresh that
      // follows would report the same read again, for as long as the thread
      // stays open. So a read is reported once per newest message.
      const newest = conversation?.lastMessage?.id ?? conversation?.updatedAt ?? "";
      const unread = conversation?.unread ?? 0;
      if (conversation !== undefined && unread === 0 && reportedRead.get(id) === newest) {
        return;
      }
      reportedRead.set(id, newest);
      if (unread > 0) {
        conversations = conversations.map((item) =>
          item.id === id ? { ...item, unread: 0 } : item,
        );
        emit();
      }
      void api.markDirectConversationRead(id).catch(() => {
        reportedRead.delete(id);
        void load().catch(() => undefined);
      });
    },

    declineCall: (id) => realtime.declineDirectCall(id),

    async addFriend(username) {
      const answer = await api.sendFriendRequest(username);
      await load();
      return answer;
    },
    acceptRequest: (id) => changeThenReload(() => api.acceptFriendRequest(id)),
    declineRequest: (id) => changeThenReload(() => api.deleteFriendRequest(id)),
    cancelRequest: (id) => changeThenReload(() => api.deleteFriendRequest(id)),
    removeFriend: (userId) => changeThenReload(() => api.removeFriend(userId)),
    block: (userId) => changeThenReload(() => api.blockUser(userId)),
    unblock: (userId) => changeThenReload(() => api.unblockUser(userId)),

    onChanged(handler) {
      listeners.add(handler);
      return () => listeners.delete(handler);
    },

    dispose() {
      for (const unsubscribe of unsubscribers) unsubscribe();
      listeners.clear();
      transcripts.clear();
      loadedConversations.clear();
      olderCursors.clear();
      olderLoads.clear();
      newestLoads.clear();
      calls.clear();
      pending.clear();
      encryptedUploads.clear();
    },
  };
}
