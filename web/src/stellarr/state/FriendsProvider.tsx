import { draftScope } from "@/lib/platform/drafts";
import { useAttachmentQueue } from "./chat/useAttachmentQueue";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useSyncExternalStore,
  type ReactNode,
} from "react";

import type { ComposerChip } from "@/components/room/composer/ComposerChips";
import type { TranscriptSectionVM } from "@/components/room/transcript/Transcript";
import { translateMessage } from "@/i18n";
import { ApiError } from "@/lib/api/errors";
import { buildTranscript, initials } from "@/lib/format/format";
import type {
  AddFriendResult,
  DmConversation,
  Friend,
  FriendRequest,
  FriendsPort,
  Message,
  User,
} from "@/ports";

import { createSealedCollapser, encryptionLineFacts, type EncryptionLineFacts } from "./dmTimeline";
import { useEmoji } from "./EmojiProvider";
import { useLanguage } from "./LanguageProvider";
import { createTranscriptPresenter, type PresentationContext } from "./chat/present";
import { usePorts } from "./PortsProvider";
import { useSessionContext } from "./SessionProvider";
import { useUiContext } from "./UiProvider";
import { useVoiceContext } from "./VoiceProvider";

export interface FriendsValue {
  friends: Friend[];
  requests: FriendRequest[];
  blocked: User[];
  conversations: DmConversation[];

  /** Incoming requests only: what the pending badge counts. */
  incoming: number;
  /** Unread messages across every conversation. */
  unread: number;

  /** The open conversation, or null while the Friends view is showing. */
  active: DmConversation | null;
  /** The open conversation's transcript, grouped the way the room renders it. */
  sections: TranscriptSectionVM[];
  /** What the open conversation's encryption lines mean now. */
  encryptionLines: EncryptionLineFacts;
  /** Decrypts an encrypted file shown as a link; null when it cannot be read. */
  openAttachment: (url: string) => Promise<{ url: string; type: string } | null>;

  /**
   * Opens the conversation with this person, creating it if there is none, and
   * goes there. `text` is sent on the way in, so the mini profile's field can
   * write the first message and navigate in one action.
   */
  messagePerson: (userId: string, text?: string) => void;
  /** Opens a conversation if necessary and starts its private call. */
  callPerson: (userId: string) => void;
  /** Starts or joins the call attached to an existing conversation. */
  callConversation: (conversationId: string) => void;
  declineCall: (conversationId: string) => void;
  /**
   * You are in a private call the other person has not joined, so on their
   * side it rings. Read from the call's occupancy, the way the receiving side
   * reads an incoming call: the RTC layer lists the person before they come.
   */
  ringing: boolean;
  /** Sends into the open conversation. No-op when there is none. */
  send: (text: string) => Promise<void>;
  /**
   * Hands the message and its files over to go out at `sendAt` in the open
   * conversation. An encrypted conversation cannot hold one: the server only
   * takes what is sealed for the key in use now.
   */
  schedule: (text: string, sendAt: Date) => Promise<void>;
  hasOlderMessages: boolean;
  loadingOlderMessages: boolean;
  loadOlderMessages: () => void;
  chips: ComposerChip[];
  restoringFiles: boolean;
  attach: (files: File[]) => void;
  retryChip: (chipId: string) => void;
  removeChip: (chipId: string) => void;
  reorderChips: (from: number, to: number) => void;

  addFriend: (username: string) => Promise<AddFriendResult>;
  accept: (requestId: string) => void;
  decline: (requestId: string) => void;
  cancel: (requestId: string) => void;
  remove: (userId: string) => void;
  block: (userId: string) => void;
  unblock: (userId: string) => void;
}

const FriendsContext = createContext<FriendsValue | null>(null);

const NO_MESSAGES: Message[] = [];
const NO_SECTIONS: TranscriptSectionVM[] = [];
const NO_LINES: EncryptionLineFacts = { liveRequestId: null, declinedIds: new Set() };
const NO_MENTIONS: PresentationContext["mentions"] = new Map();
const NO_PINS: PresentationContext["pinnedIds"] = new Set();

/** Reads a value the port owns. The port hands back a stable reference until it changes. */
function useSocial<T>(port: FriendsPort, read: () => T): T {
  return useSyncExternalStore(port.onChanged, read, read);
}

/**
 * The friends graph and the private conversations.
 *
 * A provider of its own rather than a branch of `ChatProvider`: that one
 * follows the active channel and talks to `ports.chat`, and a conversation is
 * neither. What they share is how a transcript is presented, and that is the
 * same presenter function, called here rather than inherited.
 */
export function FriendsProvider({ children }: { children: ReactNode }) {
  const { locale, t } = useLanguage();
  const { friends: port, scheduled } = usePorts();
  const { byShortcode: customEmoji } = useEmoji();
  const { user } = useSessionContext();
  const { activeDmId, openDm, inConversationCall, callChatOpen, cinema } = useUiContext();
  const {
    join: joinVoice,
    leave: leaveVoice,
    privateCall,
    channelId: voiceRoomId,
  } = useVoiceContext();

  const uploadFile = useCallback((file: File) => port.uploadAttachment(activeDmId!, file), [port, activeDmId]);
  const {
    pending: chips,
    restoringFiles,
    chips: chipViews,
    attach,
    retryChip,
    removeChip,
    reorderChips,
    discard: discardChips,
    reupload: reuploadChips,
  } = useAttachmentQueue(uploadFile, t, activeDmId && user ? draftScope(user.id, "dm", activeDmId) : undefined, true);
  const failedSend = useRef<{ fingerprint: string; clientMessageId: string } | null>(
    null,
  );

  const friends = useSocial(port, port.listFriends);
  const requests = useSocial(port, port.listRequests);
  const blocked = useSocial(port, port.listBlocked);
  const conversations = useSocial(port, port.listConversations);
  // Paging and the transcript change without touching the lists above, so the
  // open conversation's own state is read from the port as well.
  const messages = useSocial(port, () =>
    activeDmId === null ? NO_MESSAGES : port.listDmMessages(activeDmId),
  );
  const hasOlderMessages = useSocial(
    port,
    () => activeDmId !== null && port.hasOlderDmMessages(activeDmId),
  );
  const loadingOlderMessages = useSocial(
    port,
    () => activeDmId !== null && port.isLoadingOlderDmMessages(activeDmId),
  );

  const active =
    conversations.find((conversation) => conversation.id === activeDmId) ?? null;

  // Opening a conversation is what reads it, so one reached any other way (a
  // reload, a message arriving while it is open) is not left unread. The port
  // reports each newest message once, so the refresh a read causes is not read
  // again.
  //
  // Read means seen. During the conversation's call the room shows the call,
  // and the messages are on screen only while the chat beside it is open (and
  // not stepped aside for cinema); until then they stay unread and the chat's
  // button counts them.
  const activeUnread = active?.unread ?? 0;
  const activeNewest = active?.lastMessage?.id ?? null;
  const transcriptShown = !inConversationCall || (callChatOpen && !cinema);
  useEffect(() => {
    if (activeDmId !== null && transcriptShown) port.markRead(activeDmId);
  }, [port, activeDmId, activeUnread, activeNewest, transcriptShown]);

  // Selecting a conversation in the rail only changes which one is open; the
  // port has no transcript for it yet unless somebody opened it through the
  // mini profile or a call in this session. Ask for the newest page here so
  // the room never shows an empty history for a conversation that has one.
  useEffect(() => {
    if (activeDmId !== null) void port.loadConversation(activeDmId).catch(() => undefined);
  }, [port, activeDmId]);

  // Blocking either person removes the conversation from the snapshot, but a
  // browser-to-browser connection would outlive that decision, so the private
  // call is closed as soon as its conversation disappears.
  useEffect(() => {
    if (!privateCall || voiceRoomId === null) return;
    if (conversations.some((item) => item.id === voiceRoomId)) return;
    void leaveVoice();
  }, [conversations, privateCall, voiceRoomId, leaveVoice]);

  const joinedCall = privateCall
    ? conversations.find((item) => item.id === voiceRoomId)
    : undefined;
  const ringing =
    joinedCall !== undefined &&
    !(joinedCall.callParticipants ?? []).some(
      (participant) => participant.userId === joinedCall.peer.id,
    );

  const present = useMemo(() => createTranscriptPresenter(), []);
  const presentation = useMemo<PresentationContext>(() => {
    // Both authors are already `User`s, so no directory lookup is needed.
    const avatars = new Map<string, string>();
    for (const person of [user, active?.peer]) {
      if (person?.avatarUrl !== undefined) avatars.set(person.id, person.avatarUrl);
    }
    return {
      selfId: user?.id,
      // A private conversation has no server notice to name.
      server: { name: "" },
      faces: avatars,
      customEmoji,
      mentions: NO_MENTIONS,
      pinnedIds: NO_PINS,
      reactionsOf: () => [],
      extFallback: t("attachment.extFallback"),
    };
  }, [user, active?.peer, customEmoji, t]);

  const collapseSealed = useMemo(() => createSealedCollapser(), []);
  const sections = useMemo<TranscriptSectionVM[]>(() => {
    if (activeDmId === null) {
      // Presenting nothing releases the conversation that was just closed.
      present([], presentation);
      return NO_SECTIONS;
    }
    const shown = collapseSealed(messages);
    return present(buildTranscript(shown, new Date(), locale), presentation);
  }, [activeDmId, messages, locale, present, presentation, collapseSealed]);

  const activeEncryption = active?.encryption;
  const encryptionLines = useMemo(
    () => (activeDmId === null ? NO_LINES : encryptionLineFacts(messages, activeEncryption)),
    [activeDmId, messages, activeEncryption],
  );

  const openAttachment = useCallback(
    (url: string) => port.openAttachment?.(url) ?? Promise.resolve(null),
    [port],
  );

  const messagePerson = useCallback(
    (userId: string, text?: string) => {
      void (async () => {
        const id = await port.openConversation(userId);
        // Sent against the id rather than through `send`, which follows the
        // open conversation, and that is still the previous one until the
        // navigation below has rendered.
        const content = text?.trim() ?? "";
        if (content.length > 0) {
          await port.sendDm(id, {
            content,
            attachmentIds: [],
            clientMessageId: crypto.randomUUID(),
          });
        }
        openDm(id);
      })().catch(() => undefined);
    },
    [port, openDm],
  );

  const send = useCallback(
    async (text: string) => {
      if (activeDmId === null) return;
      if (chips.some((chip) => chip.status !== "ready")) {
        throw new Error("Attachments are still uploading or need a retry.");
      }
      const attachmentIds = chips.flatMap((chip) =>
        chip.attachment === null ? [] : [chip.attachment.id],
      );
      if (text.trim().length === 0 && attachmentIds.length === 0) return;

      // A retry of the same message keeps its id, so the server can tell it
      // from a second message if the first attempt did arrive.
      const fingerprint = JSON.stringify([activeDmId, text, attachmentIds]);
      const previous = failedSend.current;
      const clientMessageId =
        previous?.fingerprint === fingerprint
          ? previous.clientMessageId
          : crypto.randomUUID();
      try {
        await port.sendDm(activeDmId, { content: text, attachmentIds, clientMessageId });
      } catch (cause) {
        failedSend.current = { fingerprint, clientMessageId };
        if (cause instanceof ApiError && cause.code === "attachment-kind-mismatch") reuploadChips();
        throw cause;
      }
      if (failedSend.current?.clientMessageId === clientMessageId) {
        failedSend.current = null;
      }
      discardChips(new Set(chips.map((chip) => chip.id)));
    },
    [port, activeDmId, chips, discardChips, reuploadChips],
  );

  const schedule = useCallback(
    async (text: string, sendAt: Date) => {
      if (activeDmId === null) return;
      if (chips.some((chip) => chip.status !== "ready")) {
        throw new Error("Attachments are still uploading or need a retry.");
      }
      const attachmentIds = chips.flatMap((chip) =>
        chip.attachment === null ? [] : [chip.attachment.id],
      );
      if (text.trim().length === 0 && attachmentIds.length === 0) return;
      await scheduled.schedule(
        { kind: "conversation", conversationId: activeDmId },
        { content: text, attachmentIds, sendAt },
      );
      discardChips(new Set(chips.map((chip) => chip.id)));
    },
    [scheduled, activeDmId, chips, discardChips],
  );

  const joinConversationCall = useCallback(
    (id: string, peer: User | undefined) => {
      joinVoice(id, {
        label:
          peer === undefined
            ? translateMessage(locale, "dm.callPrivate")
            : translateMessage(locale, "dm.callWith", { name: peer.displayName }),
        privateCall: true,
        ...(peer === undefined
          ? {}
          : {
              people: [
                {
                  id: peer.id,
                  displayName: peer.displayName,
                  initials: initials(peer.displayName),
                  presence: peer.presence,
                  ...(peer.avatarUrl === undefined ? {} : { avatarUrl: peer.avatarUrl }),
                },
              ],
            }),
      });
      openDm(id);
    },
    [joinVoice, openDm, locale],
  );

  const callConversation = useCallback(
    (conversationId: string) => {
      const conversation = conversations.find((item) => item.id === conversationId);
      if (conversation !== undefined) {
        joinConversationCall(conversation.id, conversation.peer);
      }
    },
    [conversations, joinConversationCall],
  );

  const callPerson = useCallback(
    (personId: string) => {
      const known =
        conversations.find((item) => item.peer.id === personId)?.peer ??
        friends.find((item) => item.user.id === personId)?.user;
      void port
        .openConversation(personId)
        .then((id) => joinConversationCall(id, known))
        .catch(() => undefined);
    },
    [port, conversations, friends, joinConversationCall],
  );

  const loadOlderMessages = useCallback(() => {
    if (activeDmId !== null) {
      void port.loadOlderDmMessages(activeDmId).catch(() => undefined);
    }
  }, [port, activeDmId]);

  const addFriend = useCallback(
    async (username: string) => {
      // Answered locally, so the form needs no round trip to refuse yourself.
      const wanted = username.trim().replace(/^@/, "").toLowerCase();
      if (user !== null && wanted === user.username.toLowerCase()) return "self";
      return port.addFriend(username);
    },
    [port, user],
  );

  // Failures leave the lists as they were; the next snapshot corrects the view.
  const commands = useMemo(() => {
    const quietly = (pending: Promise<unknown>) => {
      void pending.catch(() => undefined);
    };
    return {
      declineCall: (conversationId: string) => quietly(port.declineCall(conversationId)),
      accept: (requestId: string) => quietly(port.acceptRequest(requestId)),
      decline: (requestId: string) => quietly(port.declineRequest(requestId)),
      cancel: (requestId: string) => quietly(port.cancelRequest(requestId)),
      remove: (userId: string) => quietly(port.removeFriend(userId)),
      block: (userId: string) => quietly(port.block(userId)),
      unblock: (userId: string) => quietly(port.unblock(userId)),
    };
  }, [port]);

  const value = useMemo<FriendsValue>(
    () => ({
      friends,
      requests,
      blocked,
      conversations,
      incoming: requests.filter((request) => request.direction === "incoming").length,
      unread: conversations.reduce((total, item) => total + item.unread, 0),
      active,
      sections,
      encryptionLines,
      openAttachment,
      messagePerson,
      callPerson,
      callConversation,
      ringing,
      send,
      schedule,
      hasOlderMessages,
      loadingOlderMessages,
      loadOlderMessages,
      chips: chipViews,
      restoringFiles,
      attach,
      retryChip,
      removeChip,
      reorderChips,
      addFriend,
      ...commands,
    }),
    [
      friends,
      requests,
      blocked,
      conversations,
      active,
      sections,
      encryptionLines,
      openAttachment,
      messagePerson,
      callPerson,
      callConversation,
      ringing,
      send,
      schedule,
      hasOlderMessages,
      loadingOlderMessages,
      loadOlderMessages,
      chipViews,
      restoringFiles,
      attach,
      retryChip,
      removeChip,
      reorderChips,
      addFriend,
      commands,
    ],
  );

  return <FriendsContext.Provider value={value}>{children}</FriendsContext.Provider>;
}

export function useFriendsContext(): FriendsValue {
  const value = useContext(FriendsContext);
  if (value === null) {
    throw new Error("useFriends must be used inside <FriendsProvider>");
  }
  return value;
}
