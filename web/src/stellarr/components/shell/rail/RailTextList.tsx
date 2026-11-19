import { memo, useCallback, useMemo, type RefCallback } from "react";

import { ChannelButton } from "@/components/shell/rail/ChannelButton";
import { ChannelThreads } from "@/components/shell/rail/ChannelThreads";
import type {
  ChannelMenuRaiser,
  RailSortRefs,
  RailThreads,
} from "@/components/shell/rail/railTypes";
import type { ChannelThreadVM, TextChannelVM } from "@/components/view-models";
import type { DragHandleProps } from "@/lib/drag/useRailDrag";

export interface RailTextListProps {
  channels: TextChannelVM[];
  activeChannelId: string | null;
  threads: RailThreads;
  isMuted?: (channelId: string) => boolean;
  onSelect: (channelId: string) => void;
  /** Already gated on who may edit; absent hides the cog. */
  onOpenSettings?: (channelId: string) => void;
  menuFor?: ChannelMenuRaiser;
  /** Where this list sits in the rail's sort. */
  sort: RailSortRefs;
}

const NO_THREADS: ChannelThreadVM[] = [];

interface TextChannelNodeProps {
  channel: TextChannelVM;
  /** The room shows this channel itself, not one of its threads. */
  active: boolean;
  /** The room shows this channel or one of its threads. */
  channelActive: boolean;
  muted: boolean;
  threads: ChannelThreadVM[];
  threadsOpen: boolean;
  /** The thread the room shows, when it is one of this channel's; null otherwise. */
  activeThreadId: string | null;
  /** The thread being fetched, when it is one of this channel's; null otherwise. */
  openingThreadId: string | null;
  onSelect: (channelId: string) => void;
  onToggleThreads: (channelId: string) => void;
  onOpenThread?: (thread: ChannelThreadVM) => void;
  registerThread: RailThreads["register"];
  onOpenSettings?: (channelId: string) => void;
  menuFor?: ChannelMenuRaiser;
  itemRef: RefCallback<HTMLElement>;
  drag?: DragHandleProps;
}

/**
 * One text channel with its open threads. Everything it takes stays the same
 * object while the channel does, so a message in another channel renders
 * that channel's row and not this one.
 */
const TextChannelNode = memo(function TextChannelNode({
  channel,
  active,
  channelActive,
  muted,
  threads,
  threadsOpen,
  activeThreadId,
  openingThreadId,
  onSelect,
  onToggleThreads,
  onOpenThread,
  registerThread,
  onOpenSettings,
  menuFor,
  itemRef,
  drag,
}: TextChannelNodeProps) {
  const { id, name, sectionId } = channel;
  const unread = channel.unread ?? 0;
  const threadsId = `channel-threads-${id}`;
  const select = useCallback(() => onSelect(id), [onSelect, id]);
  const hasThreads = threads.length > 0;
  const toggle = useMemo(
    () => (hasThreads ? () => onToggleThreads(id) : undefined),
    [hasThreads, onToggleThreads, id],
  );
  const openSettings = useMemo(
    () => (onOpenSettings === undefined ? undefined : () => onOpenSettings(id)),
    [onOpenSettings, id],
  );
  const contextMenu = useMemo(
    () => menuFor?.({ id, kind: "text", name, unread, sectionId }),
    [menuFor, id, name, unread, sectionId],
  );

  return (
    <div ref={itemRef} className="text-channel-node">
      <ChannelButton
        kind="text"
        name={name}
        active={active}
        unread={unread}
        muted={muted}
        threadCount={threads.length}
        threadsOpen={threadsOpen}
        threadsId={threadsId}
        onToggleThreads={toggle}
        onOpenSettings={openSettings}
        onSelect={select}
        onContextMenu={contextMenu}
        drag={drag}
      />
      {hasThreads ? (
        <ChannelThreads
          id={threadsId}
          channelName={name}
          threads={threads}
          open={threadsOpen}
          channelActive={channelActive}
          activeThreadId={activeThreadId}
          openingThreadId={openingThreadId}
          onOpen={onOpenThread}
          register={registerThread}
        />
      ) : null}
    </div>
  );
});

/**
 * The text channels of one rail group.
 *
 * The rail sorts every group as one; this list hands it its own element, each
 * row with its open threads as the thing that moves, and the row's main
 * button as the handle when the reader may sort.
 */
export function RailTextList({
  channels,
  activeChannelId,
  threads,
  isMuted,
  onSelect,
  onOpenSettings,
  menuFor,
  sort: { list, item, handle },
}: RailTextListProps) {
  return (
    <div className="rail-list" ref={list}>
      {channels.map((channel) => {
        const channelThreads = threads.byChannel[channel.id] ?? NO_THREADS;
        const holds = (threadId: string | null) =>
          threadId !== null && channelThreads.some((thread) => thread.id === threadId);
        const holdsActive = holds(threads.activeId);
        return (
          <TextChannelNode
            key={channel.id}
            channel={channel}
            active={channel.id === activeChannelId && threads.activeId === null}
            channelActive={channel.id === activeChannelId}
            muted={isMuted?.(channel.id) ?? false}
            threads={channelThreads}
            threadsOpen={threads.expanded.has(channel.id) || holdsActive}
            activeThreadId={holdsActive ? threads.activeId : null}
            openingThreadId={holds(threads.openingId) ? threads.openingId : null}
            onSelect={onSelect}
            onToggleThreads={threads.onToggle}
            onOpenThread={threads.onOpen}
            registerThread={threads.register}
            onOpenSettings={onOpenSettings}
            menuFor={menuFor}
            itemRef={item(channel.id)}
            drag={handle?.(channel.id)}
          />
        );
      })}
    </div>
  );
}
