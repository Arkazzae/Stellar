import { ChannelButton } from "@/components/shell/rail/ChannelButton";
import { ChannelThreads } from "@/components/shell/rail/ChannelThreads";
import type {
  ChannelMenuRaiser,
  RailThreads,
} from "@/components/shell/rail/railTypes";
import type { TextChannelVM } from "@/components/view-models";
import { DragAnnouncer } from "@/lib/drag/DragAnnouncer";
import { useSortableList } from "@/lib/drag/useSortableList";
import { useLanguage } from "@/state/hooks";

export interface RailTextListProps {
  channels: TextChannelVM[];
  activeChannelId: string | null;
  threads: RailThreads;
  isMuted?: (channelId: string) => boolean;
  onSelect: (channelId: string) => void;
  /** Already gated on who may edit; absent hides the cog. */
  onOpenSettings?: (channelId: string) => void;
  menuFor?: ChannelMenuRaiser;
  /** The list's complete id order after a drop that changed it, and the channel that moved. */
  onReorder: (order: string[], channelId: string) => void;
}

/**
 * The text channels of one rail group, sortable by dragging.
 *
 * The whole row is the handle, which only works because a press has to travel
 * 6 px before it counts as a drag, so a click still opens the channel. What
 * moves is the row together with its open threads, so they never part. The
 * keyboard stands in for a grip: Space picks a channel up, the arrows move it,
 * Space drops it (`ChannelButton` keeps Enter for "open") and Escape puts it
 * back.
 */
export function RailTextList({
  channels,
  activeChannelId,
  threads,
  isMuted,
  onSelect,
  onOpenSettings,
  menuFor,
  onReorder,
}: RailTextListProps) {
  const { t } = useLanguage();
  const nameOf = (id: string) =>
    channels.find((channel) => channel.id === id)?.name ?? "";
  const sortable = useSortableList({
    ids: channels.map((channel) => channel.id),
    onReorder,
    roleDescription: t("shell.reorderHandle"),
    messages: {
      lifted: (id, position, count) =>
        t("shell.reorderLifted", { name: nameOf(id), position, count }),
      moved: (id, position, count) =>
        t("shell.reorderMoved", { name: nameOf(id), position, count }),
      dropped: (id, position, count) =>
        t("shell.reorderDropped", { name: nameOf(id), position, count }),
      cancelled: () => t("shell.reorderCancelled"),
    },
  });

  return (
    <>
      {channels.map((channel) => {
        const channelThreads = threads.byChannel[channel.id] ?? [];
        const holdsActive = channelThreads.some(
          (thread) => thread.id === threads.activeId,
        );
        const open = threads.expanded.has(channel.id) || holdsActive;
        const threadsId = `channel-threads-${channel.id}`;
        const unread = channel.unread ?? 0;
        const node = sortable.item(channel.id);
        return (
          <div
            key={channel.id}
            ref={node.ref}
            className={["text-channel-node", node.className].filter(Boolean).join(" ")}
            style={node.style}
          >
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
              drag={{
                dragging: sortable.carrying === channel.id,
                handle: sortable.handle(channel.id),
              }}
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
      <DragAnnouncer
        id={sortable.instructionsId}
        instructions={t("shell.reorderInstructions")}
        announcement={sortable.announcement}
      />
    </>
  );
}
