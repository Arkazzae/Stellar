import { ChannelButton } from "@/components/shell/rail/ChannelButton";
import { ChannelThreads } from "@/components/shell/rail/ChannelThreads";
import type {
  ChannelMenuRaiser,
  RailSortRefs,
  RailThreads,
} from "@/components/shell/rail/railTypes";
import type { TextChannelVM } from "@/components/view-models";

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
        const channelThreads = threads.byChannel[channel.id] ?? [];
        const holdsActive = channelThreads.some(
          (thread) => thread.id === threads.activeId,
        );
        const open = threads.expanded.has(channel.id) || holdsActive;
        const threadsId = `channel-threads-${channel.id}`;
        const unread = channel.unread ?? 0;
        return (
          <div key={channel.id} ref={item(channel.id)} className="text-channel-node">
            <ChannelButton
              kind="text"
              name={channel.name}
              active={channel.id === activeChannelId && threads.activeId === null}
              unread={unread}
              muted={isMuted?.(channel.id) ?? false}
              threadCount={channelThreads.length}
              threadsOpen={open}
              threadsId={threadsId}
              onToggleThreads={
                channelThreads.length === 0
                  ? undefined
                  : () => threads.onToggle(channel.id)
              }
              onOpenSettings={
                onOpenSettings === undefined
                  ? undefined
                  : () => onOpenSettings(channel.id)
              }
              onSelect={() => onSelect(channel.id)}
              onContextMenu={menuFor?.({
                id: channel.id,
                kind: "text",
                name: channel.name,
                unread,
                sectionId: channel.sectionId,
              })}
              drag={handle?.(channel.id)}
            />
            {channelThreads.length === 0 ? null : (
              <ChannelThreads
                id={threadsId}
                channelName={channel.name}
                threads={channelThreads}
                open={open}
                channelActive={channel.id === activeChannelId}
                activeThreadId={threads.activeId}
                openingThreadId={threads.openingId}
                onOpen={threads.onOpen}
                register={threads.register}
              />
            )}
          </div>
        );
      })}
    </div>
  );
}
