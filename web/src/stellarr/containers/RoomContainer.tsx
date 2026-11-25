import { memo, useCallback, useRef, useSyncExternalStore, type ComponentProps } from "react";

import { DropOverlay } from "@/components/room/DropOverlay";
import { MediaViewer } from "@/components/room/attachments/MediaViewer";
import { ThreadPanel } from "@/components/room/panels/ThreadPanel";
import {
  Transcript,
  type TranscriptProps,
  type TranscriptSectionVM,
} from "@/components/room/transcript/Transcript";
import { Typing } from "@/components/room/transcript/Typing";
import { RoomHeader } from "@/components/room/RoomHeader";
import {
  useAccount,
  useChatRoom,
  useChatTranscript,
  useDirectory,
  useEmoji,
  useLanguage,
  usePorts,
  useProfile,
  useUi,
  useVoice,
} from "@/state/hooks";

import { useJoinFromPopoutLink } from "./callPopout";
import { CallStage } from "./CallStage";
import { ChannelPanelContainer } from "./ChannelPanelContainer";
import { NotificationInboxButton } from "./NotificationInboxContainer";
import { RoomComposer } from "./RoomComposer";
import { useAttachmentViewer } from "./useAttachmentViewer";
import { useChatTranslation } from "./useChatTranslation";
import { useThreadLayout } from "./useThreadLayout";

const NOBODY: string[] = [];
const NO_SECTIONS: TranscriptSectionVM[] = [];

/**
 * Who is typing in the open channel, read where it is drawn. The port keeps
 * the list, so a notice, in this channel or in any other, renders this line
 * and nothing else.
 */
function RoomTyping({ channelId }: { channelId: string | null }) {
  const { typing } = usePorts();
  const read = () => (channelId === null ? NOBODY : typing.typingIn(channelId));
  return <Typing names={useSyncExternalStore(typing.onTypingChanged, read, read)} />;
}

/**
 * The open channel's messages. The transcripts are the parts of the room that
 * read the chat's messages, so a message arriving renders them and not the
 * room around them.
 */
function ChannelTranscript(
  props: Omit<
    TranscriptProps,
    "sections" | "loading" | "firstUnreadGroupId" | "onLoadOlder" | "hasOlder" | "loadingOlder"
  >,
) {
  const chat = useChatTranscript();
  return (
    <Transcript
      {...props}
      sections={chat.sections}
      loading={chat.loading}
      firstUnreadGroupId={chat.firstUnreadGroupId}
      onLoadOlder={chat.loadOlder}
      hasOlder={chat.hasOlder}
      loadingOlder={chat.loadingOlder}
    />
  );
}

const SettledThreadPanel = memo(ThreadPanel);

/**
 * The open thread's messages, in its panel. A message elsewhere in the channel
 * leaves the thread's sections as they were, and the panel with them.
 */
function OpenThreadPanel(props: Omit<ComponentProps<typeof ThreadPanel>, "sections">) {
  const { threadSections } = useChatTranscript();
  return <SettledThreadPanel {...props} sections={threadSections} />;
}

/**
 * The main column of the server view: the open channel's transcript and
 * composer, with its thread and its materials beside it, or the call's stage
 * when one is on screen.
 *
 * Both views stay mounted and swap `.active`, so the transcript keeps its
 * scroll position across a call.
 */
export function RoomContainer() {
  const { server, channels, textChannels, channelName, canManageMessages } = useDirectory();
  const ui = useUi();
  const voice = useVoice();
  const chat = useChatRoom();
  const { emoji: customEmoji } = useEmoji();
  const { t } = useLanguage();
  const { preferences } = useAccount();
  const profile = useProfile();
  const translation = useChatTranslation(chat.channelId);
  const viewer = useAttachmentViewer();
  const threadLayout = useThreadLayout(chat.thread);
  useJoinFromPopoutLink(ui.joinVoiceChannel, server.id, channels);

  const room = useRef<HTMLElement>(null);
  const fullscreenTarget = useCallback(() => room.current, []);

  /*
   * The transcript's "go to this message", lent to the channel panel. Only the
   * transcript can perform the scroll, since only it knows where its rows are,
   * so it hands the function up. A ref, because taking it must not re-render
   * the room; a message older than the loaded history scrolls nowhere.
   */
  const jump = useRef<((messageId: string) => void) | null>(null);
  const takeJump = useCallback((jumpTo: (messageId: string) => void) => {
    jump.current = jumpTo;
  }, []);
  const jumpToMessage = useCallback((messageId: string) => jump.current?.(messageId), []);

  const activeText = textChannels.find((channel) => channel.id === ui.activeTextChannelId);
  const textName = activeText?.name ?? "";

  // A call has the stage; the panels belong to the transcript.
  const inVoice = ui.roomView === "voice";
  const joinedVoiceChannel = channels.find(
    (channel) => channel.id === voice.channelId && channel.kind === "voice",
  );
  const title = inVoice
    ? (voice.call?.channelName ?? (channelName(voice.channelId) || textName))
    : textName;
  const topic = inVoice ? (joinedVoiceChannel?.topic ?? "") : (activeText?.topic ?? "");

  const channelPanelOpen = ui.channelPanel !== null && !inVoice;
  const threadPanelOpen = chat.thread !== null && !inVoice;
  const threadRootId = chat.thread?.rootId ?? null;
  const threadExpanded = threadPanelOpen && threadLayout.expanded;

  const composer = (
    <RoomComposer
      key={threadPanelOpen ? `thread-${threadRootId}` : "channel"}
      channelName={textName}
      threadRootId={threadPanelOpen ? threadRootId : null}
    />
  );

  // The two transcripts differ only in what they show; everything a reader can
  // do with a message is the same.
  const messageProps = {
    followOwnSend: chat.ownSendToken,
    translation,
    attachments: viewer.attachments,
    customEmoji,
    // Moderation is separate from channel administration; the server enforces
    // the same effective permission on deletion.
    canModerate: canManageMessages,
    developerMode: preferences.developerMode,
    editing: chat.editing,
    onEditSave: chat.saveEdit,
    onEditCancel: chat.cancelEdit,
    onReact: chat.react,
    onAction: chat.runAction,
    onOpenAuthor: profile.toggle,
  };
  const threadPanel = {
    ...messageProps,
    open: threadPanelOpen,
    expanded: threadExpanded,
    onExpandedChange: threadLayout.setExpanded,
    onClose: chat.closeThread,
    label: t("room.thread"),
    composer: threadPanelOpen ? composer : null,
  };

  return (
    <main
      className={[
        "room",
        inVoice && "in-voice",
        channelPanelOpen && "channel-panel-open",
        threadPanelOpen && "thread-panel-open",
        threadExpanded && "thread-panel-expanded",
      ]
        .filter(Boolean)
        .join(" ")}
      ref={room}
    >
      <RoomHeader
        view={ui.roomView}
        title={title}
        topic={topic}
        rosterOpen={ui.rosterOpen}
        onToggleRoster={ui.toggleRoster}
        railOpen={ui.railOpen}
        onToggleRail={ui.toggleRail}
        panelOpen={channelPanelOpen}
        onTogglePanel={() => {
          chat.closeThread();
          ui.toggleChannelPanel(ui.channelPanelTab);
        }}
        notifications={<NotificationInboxButton />}
        t={t}
      />

      <div className="room-body">
        <div className={inVoice ? "view" : "view active"}>
          <ChannelTranscript
            {...messageProps}
            key={chat.channelId ?? "no-channel"}
            label={t("room.messagesOn", { channel: textName })}
            onJumpReady={takeJump}
            covered={inVoice}
          />
          <RoomTyping channelId={chat.channelId} />
          {threadPanelOpen ? null : composer}
        </div>

        <div className={inVoice ? "view active" : "view"}>
          <CallStage fullscreenTarget={fullscreenTarget} />
        </div>
      </div>

      {/* The closed panel stays in the shell for its drawer geometry. A fresh
          transcript for each thread lets its first alignment to the bottom run
          against that thread's rows, not the empty list the closed panel held. */}
      {chat.thread === null ? (
        <ThreadPanel {...threadPanel} key="closed-thread" sections={NO_SECTIONS} />
      ) : (
        <OpenThreadPanel {...threadPanel} key={chat.thread.rootId} />
      )}

      <ChannelPanelContainer
        open={channelPanelOpen}
        callbacks={viewer.attachments}
        onJump={jumpToMessage}
      />

      <DropOverlay
        channelName={textName}
        // Armed only where a drop has a visible home: behind a call the
        // transcript is hidden, and a file dropped there would land unseen in
        // whichever text channel was last open.
        enabled={!inVoice && activeText !== undefined}
        onDropFiles={chat.attach}
      />
      <MediaViewer target={viewer.target} onClose={viewer.close} resolveOriginal={viewer.attachments.resolveOriginal} />
    </main>
  );
}
