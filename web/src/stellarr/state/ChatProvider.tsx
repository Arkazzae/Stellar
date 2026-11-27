import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useReducer,
  useRef,
  useSyncExternalStore,
  type ReactNode,
} from "react";

import type { PinnedMessageVM } from "@/components/room/panels/channelPanelModel";
import type { ComposerChip } from "@/components/room/composer/ComposerChips";
import type { MessageAction } from "@/components/room/transcript/MessageToolbar";
import type { TranscriptSectionVM } from "@/components/room/transcript/Transcript";
import { useStableHandler } from "@/hooks/useStableHandler";
import { buildThreadTranscript, buildTranscript } from "@/lib/format/format";
import { findFirstUnreadGroup } from "@/lib/messages/unread";
import type {
  Message,
  MessageReference,
  ThreadSummary,
} from "@/ports";

import { useAccountContext } from "./AccountProvider";
import { chatReducer, EMPTY_CHAT_DATA } from "./chat/chatStore";
import {
  createTranscriptPresenter,
  memberFaces,
  mentionTargets,
  pinnedRows,
  type PresentationContext,
} from "./chat/present";
import type { PendingIds, RestoredWindows, SyncCursors } from "./chat/shared";
import { NO_MESSAGES } from "./chat/timeline";
import { draftScope } from "@/lib/platform/drafts";
import { useAttachmentQueue } from "./chat/useAttachmentQueue";
import { useChannelPages } from "./chat/useChannelPages";
import { useChatComposer } from "./chat/useChatComposer";
import { useChatFeed } from "./chat/useChatFeed";
import { useMentionNotifications } from "./chat/useMentionNotifications";
import { useMessageActions } from "./chat/useMessageActions";
import { useMessageStore } from "./chat/useMessageStore";
import { useMessageEdit } from "./chat/useMessageEdit";
import { useReadMarks } from "./chat/useReadMarks";
import { useReconnectSync } from "./chat/useReconnectSync";
import { useThreadCatalogue } from "./chat/useThreadCatalogue";
import { useThreadPanel } from "./chat/useThreadPanel";
import { useDirectoryContext } from "./DirectoryProvider";
import { useEmoji } from "./EmojiProvider";
import { useLanguage } from "./LanguageProvider";
import { usePorts } from "./PortsProvider";
import { useSessionContext } from "./SessionProvider";
import { useUiContext } from "./UiProvider";

export interface ChatValue {
  channelId: string | null;
  sections: TranscriptSectionVM[];
  /** The thread beside the transcript, or null; its messages are `threadSections`. */
  thread: {
    rootId: string;
    /** The room applies this once for each new open revision. */
    initialExpanded: boolean;
    openRevision: number;
  } | null;
  /** The open thread's messages, empty while no thread is open. */
  threadSections: TranscriptSectionVM[];
  /** Complete per-channel catalogues used by the dropdowns in the left rail. */
  threadsByChannel: Readonly<Record<string, ThreadSummary[]>>;
  /** The rail row waiting for its complete conversation read, or null. */
  openingThreadId: string | null;
  /** Changes channel when needed, then opens the complete selected thread. */
  openThread: (thread: ThreadSummary) => void;
  closeThread: () => void;
  /**
   * How many messages have arrived on each channel since you last read it.
   * The channel you are on is always zero: you are reading it now.
   */
  unreadByChannel: Readonly<Record<string, number>>;
  /**
   * Clears one channel's badge without opening it.
   *
   * The same mark that leaving a channel writes, moved forward by hand: reading
   * is how a channel normally becomes read, and this is for the one you have
   * decided not to read.
   */
  markChannelRead: (channelId: string) => void;
  /**
   * The first block of the open channel that arrived after you last left it,
   * or null when there is nothing new. The transcript draws its "new messages"
   * line above this block, and the line stays put while you read: the mark
   * only moves when you leave the channel.
   */
  firstUnreadGroupId: string | null;
  /** True while the first page of a channel is still in flight. */
  loading: boolean;
  /** True while an older page is being prepended. */
  loadingOlder: boolean;
  /** False once the beginning of this channel has been reached. */
  hasOlder: boolean;
  chips: ComposerChip[];
  /**
   * The open channel's pinned messages, newest pin first.
   *
   * Read from its own endpoint rather than filtered out of `sections`: a pin is
   * normally far above the loaded window, and a list that filled in as you
   * scrolled would not be a list of the pinned messages.
   */
  pinned: PinnedMessageVM[];
  /** True while that list is in flight for the first time on this channel. */
  pinnedLoading: boolean;
  /** Why the last pin or unpin did not happen, or null. */
  pinError: string | null;
  /** The message the composer is answering, or null when it is not. */
  replyingTo: MessageReference | null;
  /**
   * The message the composer is rewriting, with the text it started with.
   * `send` saves it instead of posting a new message while this is set.
   */
  editing: { id: string; original: string } | null;
  /** Called by the editor in the message. Rejects if the server refuses. */
  saveEdit: (messageId: string, text: string) => Promise<void>;
  cancelEdit: () => void;
  /** Raised by the toolbar's reply action; the composer then names it. */
  startReply: (messageId: string) => void;
  cancelReply: () => void;

  /**
   * Bumped the moment one of your own messages enters the list.
   *
   * The transcript follows arrivals only for a reader already at the end, which
   * is right for someone else's message and wrong for your own: you just
   * acted, so you are owed the result. A token rather than a boolean, because
   * two sends in a row have to be two distinct events.
   */
  ownSendToken: number;
  send: (text: string) => Promise<void>;
  /** Uploads and publishes one standalone recording from the composer. */
  sendVoice: (file: File, durationSeconds: number) => Promise<void>;
  /** Sends one canonical GIPHY page as a standalone message. */
  sendGif: (pageUrl: string) => Promise<void>;
  /** Hands the message, with its files and its answer, over to go out at `sendAt`. */
  schedule: (text: string, sendAt: Date) => Promise<void>;
  restoringFiles: boolean;
  attach: (files: File[]) => void;
  retryChip: (chipId: string) => void;
  removeChip: (chipId: string) => void;
  reorderChips: (from: number, to: number) => void;
  notifyTyping: () => void;
  loadOlder: () => void;
  react: (messageId: string, emoji: string) => void;
  runAction: (messageId: string, action: MessageAction) => Promise<void> | void;
  /** Adds an indexed result to the transcript window so it can be revealed. */
  revealMessage: (message: Message) => void;
}

/*
 * The chat reaches three audiences, each through a value of its own that
 * changes only with what that audience draws: a message in the open channel
 * changes the transcript's, one elsewhere the rail's badges, and a reply bar
 * or a queued file the room's.
 */

type RailKey =
  | "threadsByChannel"
  | "openingThreadId"
  | "openThread"
  | "closeThread"
  | "unreadByChannel"
  | "markChannelRead";

type TranscriptKey =
  | "sections"
  | "threadSections"
  | "firstUnreadGroupId"
  | "loading"
  | "loadingOlder"
  | "hasOlder"
  | "loadOlder";

/** What the channel rail draws and does: the badges, the thread lists and the thread on screen. */
export type ChatRailValue = Pick<ChatValue, RailKey> & {
  /** The root of the thread beside the transcript, or null. */
  activeThreadId: string | null;
};

/** The messages of the open channel and of the open thread, with their paging. */
export type ChatTranscriptValue = Pick<ChatValue, TranscriptKey>;

/** Everything the room draws around its transcripts, and every action on a message. */
export type ChatRoomValue = Omit<ChatValue, Exclude<RailKey, "closeThread"> | TranscriptKey>;

const ChatRailContext = createContext<ChatRailValue | null>(null);
const ChatRoomContext = createContext<ChatRoomValue | null>(null);
const ChatTranscriptContext = createContext<ChatTranscriptValue | null>(null);

const NO_PINS: PinnedMessageVM[] = [];
const NO_SECTIONS: TranscriptSectionVM[] = [];

export function ChatProvider({ children }: { children: ReactNode }) {
  const ports = usePorts();
  const { preferences } = useAccountContext();
  const { user, hasEnded } = useSessionContext();
  const { members, server, textChannels } = useDirectoryContext();
  const { byShortcode: customEmoji } = useEmoji();
  const { locale, t } = useLanguage();
  const {
    activeTextChannelId: channelId,
    selectTextChannel,
    showChannelPanelTab,
    closeChannelPanel,
  } = useUiContext();
  const connection = useSyncExternalStore(
    ports.connection.subscribe,
    ports.connection.getStatus,
    ports.connection.getStatus,
  );

  const [data, dispatch] = useReducer(chatReducer, EMPTY_CHAT_DATA);
  const pendingIds: PendingIds = useRef(new Set<string>());
  const syncCursors: SyncCursors = useRef<Record<string, string>>({});
  const restored: RestoredWindows = useRef({});

  // Keyed by content, because a new list rejoins and replays every channel
  // (`useReconnectSync`). The directory hands over new channel objects for
  // changes that leave the ids alone, such as a rename or an edit to a voice
  // channel.
  const textChannelKey = textChannels.map((channel) => channel.id).join("\n");
  const textChannelIds = useMemo(
    () => (textChannelKey === "" ? [] : textChannelKey.split("\n")),
    [textChannelKey],
  );
  const messages =
    channelId === null ? NO_MESSAGES : (data.messages[channelId] ?? NO_MESSAGES);

  const panel = useThreadPanel({
    chat: ports.chat,
    channelId,
    locale,
    messages,
    dispatch,
    selectTextChannel,
    closeChannelPanel,
  });
  const queueScope = user && channelId ? draftScope(user.id, server.id, channelId, panel.threadRootId ?? "") : undefined;
  const queue = useAttachmentQueue(ports.chat.uploadAttachment, t, queueScope);
  const edit = useMessageEdit(ports.messageActions, data.messages, dispatch);
  const composer = useChatComposer({
    chat: ports.chat,
    scheduled: ports.scheduled,
    channelId,
    threadRootId: panel.threadRootId,
    user,
    dispatch,
    pendingIds,
    pending: queue.pending,
    discardChips: queue.discard,
    replyTarget: panel.replyTarget,
    settleReply: panel.settleReply,
  });

  const { forgetReply, forgetThread } = panel;
  const { forgetEdit } = edit;
  const onRemoved = useCallback(
    (messageId: string) => {
      forgetReply(messageId);
      forgetEdit(messageId);
      forgetThread(messageId);
    },
    [forgetReply, forgetEdit, forgetThread],
  );

  const { reactionEpoch } = useChatFeed({
    ports,
    dispatch,
    locale,
    onRemoved,
    selfId: user?.id ?? null,
    selfUsername: user?.username ?? null,
    openChannelId: channelId,
    preferences,
  });
  useMentionNotifications({
    chat: ports.chat,
    selfId: user?.id ?? null,
    selfUsername: user?.username ?? null,
    preferences,
    channelName: (id) => textChannels.find((channel) => channel.id === id)?.name ?? null,
    openChannel: selectTextChannel,
    t,
  });
  useThreadCatalogue(ports.chat, textChannelIds, data.threadsLoaded, dispatch);
  const { loadOlder } = useChannelPages({
    chat: ports.chat,
    channelId,
    data,
    dispatch,
    pendingIds,
    syncCursors,
    restored,
  });
  useMessageStore({
    userId: user?.id ?? null,
    serverId: server.id,
    channelId,
    channelIds: textChannelIds,
    data,
    dispatch,
    pendingIds,
    syncCursors,
    restored,
  });
  useReconnectSync({
    connected: connection === "connected",
    channelIds: textChannelIds,
    chat: ports.chat,
    dispatch,
    pendingIds,
    syncCursors,
    restored,
    onRemoved,
  });
  const reads = useReadMarks({
    chat: ports.chat,
    channelId,
    messages: data.messages,
    selfId: user?.id,
    sessionEnded: hasEnded,
  });

  const pinnedMessages =
    channelId === null ? NO_MESSAGES : (data.pins[channelId] ?? NO_MESSAGES);
  const pinned = useMemo(
    () => pinnedRows(pinnedMessages, locale, server.name),
    [pinnedMessages, locale, server.name],
  );
  /** Which individual messages carry the pin marker and unpin action. */
  const pinnedIds = useMemo(
    () => new Set(pinnedMessages.map((message) => message.id)),
    [pinnedMessages],
  );

  const faces = useMemo(() => memberFaces(members), [members]);
  const mentions = useMemo(
    () => mentionTargets(members, user?.id),
    [members, user?.id],
  );
  const view = useMemo<PresentationContext>(() => {
    void reactionEpoch; // rebuilt when a reaction changes; read per message
    return {
      selfId: user?.id,
      server,
      faces,
      customEmoji,
      mentions,
      pinnedIds,
      reactionsOf: (messageId) => ports.messageActions.getReactions(messageId),
      extFallback: t("attachment.extFallback"),
    };
  }, [ports, reactionEpoch, user?.id, server, faces, customEmoji, mentions, pinnedIds, t]);

  const present = useMemo(() => createTranscriptPresenter(), []);
  const sections = useMemo(
    () => present(buildTranscript(messages, new Date(), locale), view),
    [messages, locale, present, view],
  );

  const { threadRootId, threadInitialExpanded, threadOpenRevision } = panel;
  const thread = useMemo(
    () =>
      threadRootId === null
        ? null
        : {
            rootId: threadRootId,
            initialExpanded: threadInitialExpanded,
            openRevision: threadOpenRevision,
          },
    [threadRootId, threadInitialExpanded, threadOpenRevision],
  );
  // A presenter of its own, as the channel has: a message outside the thread
  // hands its sections back as they were, and a reply reuses the rows above it.
  const presentThread = useMemo(() => createTranscriptPresenter(), []);
  const threadSections = useMemo(
    () =>
      threadRootId === null
        ? NO_SECTIONS
        : presentThread(buildThreadTranscript(messages, threadRootId, new Date(), locale), view),
    [messages, locale, view, threadRootId, presentThread],
  );

  const firstUnreadGroupId = useMemo(
    () =>
      reads.openMark === null
        ? null
        : findFirstUnreadGroup(
            sections.flatMap((section) => section.groups),
            reads.openMark,
            user?.id,
          ),
    [sections, reads.openMark, user?.id],
  );

  const actions = useMessageActions({
    actions: ports.messageActions,
    t,
    dispatch,
    messages,
    pinnedIds,
    showChannelPanelTab,
    startReply: panel.startReply,
    cancelReply: panel.cancelReply,
    forgetReply,
    showThread: panel.showThread,
    beginEdit: edit.beginEdit,
    forgetEdit,
  });

  const revealMessage = useCallback(
    (message: Message) => dispatch({ type: "shown", message }),
    [],
  );

  // Each of these finds its message among the loaded ones, so each is made
  // anew with every arrival; the room holds one function for its life.
  const saveEdit = useStableHandler(edit.saveEdit);
  const startReply = useStableHandler(panel.startReply);
  const runAction = useStableHandler(actions.runAction);
  const notifyTyping = useCallback(() => {
    if (channelId !== null) ports.typing.notifyTyping(channelId);
  }, [ports, channelId]);

  const { openingThreadId, openThread, closeThread } = panel;
  const { unreadByChannel, markChannelRead } = reads;
  const rail = useMemo<ChatRailValue>(
    () => ({
      threadsByChannel: data.threads,
      openingThreadId,
      openThread,
      closeThread,
      unreadByChannel,
      markChannelRead,
      activeThreadId: threadRootId,
    }),
    [
      data.threads,
      openingThreadId,
      openThread,
      closeThread,
      unreadByChannel,
      markChannelRead,
      threadRootId,
    ],
  );

  const pinnedLoading = channelId !== null && data.pinsLoaded[channelId] !== true;
  const room = useMemo<ChatRoomValue>(
    () => ({
      channelId,
      thread,
      closeThread,
      chips: queue.chips,
      pinned: channelId === null ? NO_PINS : pinned,
      pinnedLoading,
      pinError: actions.pinError,
      replyingTo: panel.replyingTo,
      editing: edit.editing,
      saveEdit,
      cancelEdit: edit.cancelEdit,
      startReply,
      cancelReply: panel.cancelReply,
      ownSendToken: composer.ownSendToken,
      send: composer.send,
      sendVoice: composer.sendVoice,
      sendGif: composer.sendGif,
      schedule: composer.schedule,
      restoringFiles: queue.restoringFiles,
      attach: queue.attach,
      retryChip: queue.retryChip,
      removeChip: queue.removeChip,
      reorderChips: queue.reorderChips,
      notifyTyping,
      react: actions.react,
      runAction,
      revealMessage,
    }),
    [
      channelId,
      thread,
      closeThread,
      queue,
      pinned,
      pinnedLoading,
      actions.pinError,
      actions.react,
      panel.replyingTo,
      panel.cancelReply,
      edit.editing,
      edit.cancelEdit,
      saveEdit,
      startReply,
      composer,
      notifyTyping,
      runAction,
      revealMessage,
    ],
  );

  const loading =
    channelId !== null && data.loaded[channelId] !== true && data.restored[channelId] !== true;
  const loadingOlder = channelId !== null && data.loadingOlder[channelId] === true;
  const hasOlder = channelId !== null && typeof data.nextCursor[channelId] === "string";
  const transcript = useMemo<ChatTranscriptValue>(
    () => ({ sections, threadSections, firstUnreadGroupId, loading, loadingOlder, hasOlder, loadOlder }),
    [sections, threadSections, firstUnreadGroupId, loading, loadingOlder, hasOlder, loadOlder],
  );

  return (
    <ChatRailContext.Provider value={rail}>
      <ChatRoomContext.Provider value={room}>
        <ChatTranscriptContext.Provider value={transcript}>
          {children}
        </ChatTranscriptContext.Provider>
      </ChatRoomContext.Provider>
    </ChatRailContext.Provider>
  );
}

function provided<T>(value: T | null): T {
  if (value === null) {
    throw new Error("useChat must be used inside <ChatProvider>");
  }
  return value;
}

export function useChatRailContext(): ChatRailValue {
  return provided(useContext(ChatRailContext));
}

export function useChatRoomContext(): ChatRoomValue {
  return provided(useContext(ChatRoomContext));
}

export function useChatTranscriptContext(): ChatTranscriptValue {
  return provided(useContext(ChatTranscriptContext));
}

/** The whole chat, for a reader that needs all of it: it renders with any change to any part. */
export function useChatContext(): ChatValue {
  const rail = useChatRailContext();
  const room = useChatRoomContext();
  const transcript = useChatTranscriptContext();
  return useMemo(() => ({ ...rail, ...room, ...transcript }), [rail, room, transcript]);
}
