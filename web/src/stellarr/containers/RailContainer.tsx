import { memo, useMemo, useState } from "react";

import type { ChannelMenuActions } from "@/components/shell/rail/ChannelMenu";
import { Rail } from "@/components/shell/rail/Rail";
import type { ChannelThreadVM, TextChannelVM } from "@/components/view-models";
import { useMediaQuery } from "@/hooks/useMediaQuery";
import {
  useAccount,
  useChat,
  useDirectory,
  usePorts,
  useSession,
  useUi,
} from "@/state/hooks";

import { RailAccount, RailVoiceDock } from "./RailFooter";
import { useMemberMenu } from "./useMemberMenu";

/**
 * Puts each text channel's unread count on it, handing back the same object
 * for a channel whose count did not move, so a message in one channel renders
 * that channel's row alone.
 */
function unreadCounter() {
  const shown = new WeakMap<TextChannelVM, TextChannelVM>();
  return (channel: TextChannelVM, unread: number): TextChannelVM => {
    const before = shown.get(channel);
    if (before?.unread === unread) return before;
    const next = { ...channel, unread };
    shown.set(channel, next);
    return next;
  };
}

/**
 * The server's rail. It takes no props, so the shell rendering around it does
 * not render it; it follows the directory, the chat and the account, and its
 * rows follow their own seats.
 */
export const RailContainer = memo(function RailContainer() {
  const {
    server,
    textChannels,
    voiceChannels,
    sections,
    moveChannel,
    moveChannelToSection,
    reorderSections,
    canAccessServerSettings,
    canManageServer,
    canManageChannels,
    canModerateMembers,
  } = useDirectory();
  const { user } = useSession();
  const ui = useUi();
  const { admin } = usePorts();
  const { preferences, setPreferences } = useAccount();
  const memberMenu = useMemberMenu();
  const {
    thread,
    threadsByChannel,
    openingThreadId,
    openThread,
    closeThread,
    unreadByChannel,
    markChannelRead,
  } = useChat();

  // Below 768 the rail is a drawer; shut, it is off screen and must be inert.
  const railIsDrawer = useMediaQuery("(max-width: 767px)", false);

  // The directory knows what the channels are; only the chat knows which of
  // them said something while you were reading a different one.
  const [withUnread] = useState(unreadCounter);
  const texts = useMemo<TextChannelVM[]>(
    () => textChannels.map((channel) => withUnread(channel, unreadByChannel[channel.id] ?? 0)),
    [textChannels, unreadByChannel, withUnread],
  );

  const threadRows = useMemo<Record<string, ChannelThreadVM[]>>(
    () =>
      Object.fromEntries(
        Object.entries(threadsByChannel).map(([channelId, threads]) => [
          channelId,
          threads.map(({ root, channelId: parent, replyCount }) => ({
            id: root.id,
            channelId: parent,
            authorName: root.authorName,
            preview: root.preview,
            replyCount,
          })),
        ]),
      ),
    [threadsByChannel],
  );

  const canSection = canManageChannels && admin.supported.sections;
  const canModerate = canModerateMembers && admin.supported.moderation;

  // Editing and deleting are both gated: the member has to hold the permission
  // and the adapter has to have an endpoint behind it.
  const channelMenu: ChannelMenuActions = {
    onMarkRead: markChannelRead,
    isMuted: (channelId) => preferences.mutedChannelIds.includes(channelId),
    onSetMuted: (channelId, muted) => {
      const others = preferences.mutedChannelIds.filter((id) => id !== channelId);
      void setPreferences({
        mutedChannelIds: muted ? [...others, channelId] : others,
      });
    },
    onOpenSettings: ui.openChannelSettings,
    canOpenSettings: (_channelId, kind) => canManageChannels || kind === "text",
    developerMode: preferences.developerMode,
    ...(canManageServer ? { onInvite: () => ui.openModal("invite") } : {}),
    ...(canSection
      ? {
          sections,
          onMoveToSection: moveChannelToSection,
        }
      : {}),
    ...(canManageChannels && admin.supported.channels
      ? {
          onDelete: (channelId: string) => admin.deleteChannel(channelId),
        }
      : {}),
  };

  return (
    <Rail
      serverName={server.name}
      textChannels={texts}
      threadsByChannel={threadRows}
      activeThreadId={thread?.rootId ?? null}
      openingThreadId={openingThreadId}
      onOpenThread={({ channelId, id }) => {
        const summary = threadsByChannel[channelId]?.find(
          (candidate) => candidate.root.id === id,
        );
        if (summary !== undefined) openThread(summary);
      }}
      // Speech is RTC state, not presence: it only exists for the call this
      // client is in, and each occupant row follows its own seat for it.
      voiceChannels={voiceChannels}
      sections={sections}
      activeChannelId={ui.activeChannelId}
      selfId={user?.id ?? ""}
      canAccessServerSettings={canAccessServerSettings}
      canManageServer={canManageServer}
      canManageChannels={canManageChannels}
      onSelectTextChannel={(channelId) => {
        // Selecting the parent is a real navigation target even though its id
        // is already active underneath the thread.
        closeThread();
        ui.selectTextChannel(channelId);
      }}
      onJoinVoiceChannel={ui.joinVoiceChannel}
      onEditServer={() => ui.openModal("settings")}
      onInvite={() => ui.openModal("invite")}
      onMoveChannel={moveChannel}
      onOpenChannelSettings={ui.openChannelSettings}
      onCreateChannel={ui.openCreateChannel}
      {...(canSection
        ? { onOpenSectionSettings: ui.openSectionSettings, onMoveSection: reorderSections }
        : {})}
      channelMenu={channelMenu}
      voiceMemberMenu={memberMenu}
      {...(canModerate && admin.moveVoiceMember !== undefined
        ? {
            onMoveVoiceMember: (
              userId: string,
              sourceChannelId: string,
              targetChannelId: string,
            ) =>
              admin.moveVoiceMember?.(sourceChannelId, targetChannelId, userId),
          }
        : {})}
      voiceDock={<RailVoiceDock />}
      account={<RailAccount />}
      hidden={railIsDrawer && !ui.railOpen}
    />
  );
});
