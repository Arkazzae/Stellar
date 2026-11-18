import type {
  Attachment,
  ConversationEncryption,
  Message,
  User,
  UserId,
  VoiceOccupant,
} from "@/lib/api/types";

export type {
  ConversationEncryption,
  ConversationEncryptionState,
  SealedMessages,
  SealedReason,
  TimelineLine,
  TimelineLineKind,
} from "@/lib/api/types";

import type { Unsubscribe } from "./chat";

/** Friends and direct messages, backed by the social API and SignalR. */

/** A conversation between exactly two people; groups are not modelled. */
export type DmId = string;

/** Where a pending request came from; the two are answered differently. */
export type RequestDirection = "incoming" | "outgoing";

export interface Friend {
  user: User;
  /** ISO 8601. The "Wszyscy" list falls back to the display name for order. */
  since: string;
}

export interface FriendRequest {
  id: string;
  user: User;
  direction: RequestDirection;
  /** ISO 8601, shown as "sent 3 days ago". */
  sentAt: string;
}

export interface DmConversation {
  id: DmId;
  /** The other person; self is never in here. */
  peer: User;
  /**
   * Newest message, for the rail's second line. Absent on an empty thread.
   * Already decrypted, or sealed; never a line about someone's keys.
   */
  lastMessage?: Message;
  unread: number;
  /** ISO 8601 of `lastMessage`, or of the moment the thread was opened. */
  updatedAt: string;
  /** Where the end-to-end switch stands; absent means off. */
  encryption?: ConversationEncryption;
  /** Ephemeral RTC occupancy; absent or empty means there is no call to join. */
  callParticipants?: readonly VoiceOccupant[];
  /** Stable for one occupancy lifetime, through mute and speaking updates. */
  callId?: string;
}

/** What "Dodaj znajomego" can answer; each maps to one sentence in the UI. */
export type AddFriendResult =
  | "sent"
  | "unknown-user"
  | "already-friends"
  | "already-sent"
  | "self"
  | "blocked";

export interface FriendsPort {
  /** Loads the persisted snapshot once the session has bootstrapped. */
  load(): Promise<void>;
  listFriends(): Friend[];
  listRequests(): FriendRequest[];
  listBlocked(): User[];
  listConversations(): DmConversation[];

  /** Oldest first, the order a transcript reads in. */
  listDmMessages(id: DmId): Message[];
  hasOlderDmMessages(id: DmId): boolean;
  isLoadingOlderDmMessages(id: DmId): boolean;
  loadOlderDmMessages(id: DmId): Promise<void>;
  /**
   * Loads the newest page of a conversation that is not in memory yet, so
   * that opening one from the rail does not show an empty transcript. A
   * conversation that has already been read is left alone; realtime keeps it
   * current and a reconnect refreshes it.
   */
  loadConversation(id: DmId): Promise<void>;

  /** Opens (or finds) the conversation with `userId` and returns its id. */
  openConversation(userId: UserId): Promise<DmId>;
  /**
   * Sends plain or encrypted, as the conversation's switch stands. The
   * attachments are uploads `uploadAttachment` made for this conversation.
   */
  sendDm(
    id: DmId,
    input: { content: string; attachmentIds: string[]; clientMessageId: string },
  ): Promise<Message>;
  /**
   * Uploads a file for a message in this conversation: encrypted in the
   * browser first while the conversation is encrypted.
   */
  uploadAttachment(id: DmId, file: File): Promise<Attachment>;
  /**
   * An encrypted file that is shown as a link (a document, a video), decrypted
   * on demand. The link is the attachment's `url`, which starts with `e2ee:`.
   */
  openAttachment?(url: string): Promise<{ url: string; type: string } | null>;
  markRead(id: DmId): void;
  /** Rejects the current private RTC attempt and releases its caller. */
  declineCall(id: DmId): Promise<void>;

  /** `username` as typed, with or without a leading `@`. */
  addFriend(username: string): Promise<AddFriendResult>;
  acceptRequest(id: string): Promise<void>;
  declineRequest(id: string): Promise<void>;
  /** Withdraws an outgoing request. */
  cancelRequest(id: string): Promise<void>;
  removeFriend(userId: UserId): Promise<void>;
  block(userId: UserId): Promise<void>;
  unblock(userId: UserId): Promise<void>;

  /** Fires after every mutation above and on incoming realtime events. */
  onChanged(handler: () => void): Unsubscribe;

  /** Releases the realtime listeners an adapter owns. */
  dispose?(): void;
}
