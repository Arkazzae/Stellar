import { memo, useCallback, useMemo, type MouseEventHandler, type RefCallback } from "react";

import { ChannelButton } from "@/components/shell/rail/ChannelButton";
import { RailActivityRow } from "@/components/shell/rail/RailActivityRow";
import type { ChannelMenuRaiser, RailSortRefs } from "@/components/shell/rail/railTypes";
import { carries } from "@/components/shell/rail/useRailMoves";
import { VoiceOccupantRow } from "@/components/shell/rail/VoiceOccupantRow";
import type { PersonVM, VoiceChannelVM } from "@/components/view-models";
import type { DragHandleProps } from "@/lib/drag/useRailDrag";
import { useLanguage } from "@/state/hooks";

export interface RailVoiceListProps {
  channels: VoiceChannelVM[];
  activeChannelId: string | null;
  /** Whose occupant row gets `.me`. */
  selfId: string;
  /** Whose move the server has not confirmed yet. */
  movingUserId: string | null;
  onJoin: (channelId: string) => void;
  /** Already gated on who may edit; absent hides the cog. */
  onOpenSettings?: (channelId: string) => void;
  menuFor?: ChannelMenuRaiser;
  personMenuFor?: (person: PersonVM) => MouseEventHandler<HTMLElement>;
  /** Where this list sits in the rail's sort of channels. */
  sort: RailSortRefs;
  /** Staff may carry people other than themselves; you can always carry yourself. */
  canMoveOthers?: boolean;
  /** The handle that carries a seated person to another channel. */
  carry?: (channelId: string, personId: string) => DragHandleProps;
  /** Tells someone sitting down from someone moving here. */
  arrives?: (personId: string) => boolean;
  /** Joins the activity running on a channel, joining the channel first if need be. */
  onJoinActivity?: (channelId: string) => void;
  /** Back to the activity you are inside, on the stage. */
  onOpenActivity?: (channelId: string) => void;
}

interface VoiceChannelNodeProps {
  channel: VoiceChannelVM;
  active: boolean;
  selfId: string;
  /** Whose move is unconfirmed, when they sit here; null otherwise. */
  movingUserId: string | null;
  onJoin: (channelId: string) => void;
  onOpenSettings?: (channelId: string) => void;
  menuFor?: ChannelMenuRaiser;
  personMenuFor?: (person: PersonVM) => MouseEventHandler<HTMLElement>;
  itemRef: RefCallback<HTMLElement>;
  drag?: DragHandleProps;
  canMoveOthers: boolean;
  carry?: (channelId: string, personId: string) => DragHandleProps;
  arrives?: (personId: string) => boolean;
  onJoinActivity?: (channelId: string) => void;
  onOpenActivity?: (channelId: string) => void;
}

/**
 * One voice channel with the people seated under it. Everything it takes
 * stays the same object while the channel does, so another channel's seat
 * moving, or the rail rendering for a message elsewhere, does not render it.
 */
const VoiceChannelNode = memo(function VoiceChannelNode({
  channel,
  active,
  selfId,
  movingUserId,
  onJoin,
  onOpenSettings,
  menuFor,
  personMenuFor,
  itemRef,
  drag,
  canMoveOthers,
  carry,
  arrives,
  onJoinActivity,
  onOpenActivity,
}: VoiceChannelNodeProps) {
  const { t } = useLanguage();
  const { id, name, sectionId, activity } = channel;
  const count = channel.occupants.length;
  const select = useCallback(() => onJoin(id), [onJoin, id]);
  const openSettings = useMemo(
    () => (onOpenSettings === undefined ? undefined : () => onOpenSettings(id)),
    [onOpenSettings, id],
  );
  const contextMenu = useMemo(
    () => menuFor?.({ id, kind: "voice", name, unread: 0, sectionId }),
    [menuFor, id, name, sectionId],
  );

  return (
    <div ref={itemRef} className="voice-channel-drop">
      <ChannelButton
        kind="voice"
        name={name}
        active={active}
        count={count}
        countLabel={t("shell.peopleInCall", { count })}
        onOpenSettings={openSettings}
        onSelect={select}
        onContextMenu={contextMenu}
        drag={drag}
      />
      {activity === undefined || onJoinActivity === undefined || onOpenActivity === undefined ? null : (
        <RailActivityRow channelId={id} activity={activity} onJoin={onJoinActivity} onOpen={onOpenActivity} />
      )}
      <ul className="voice-occupants">
        {channel.occupants.map((person) => (
          <VoiceOccupantRow
            key={person.id}
            person={person}
            self={person.id === selfId}
            moving={movingUserId === person.id}
            drag={
              carry !== undefined && carries(person.id, { selfId, canMoveOthers, movingUserId })
                ? carry(id, person.id)
                : undefined
            }
            menuFor={personMenuFor}
            arrives={arrives}
            activity={activity?.participants.includes(person.id) ? activity : undefined}
          />
        ))}
      </ul>
    </div>
  );
});

/**
 * The voice channels of one rail group, each with the people seated under it.
 *
 * A voice row is three things at once: a click joins the call, the row's
 * button is the handle the rail sorts channels by, and the row with its people
 * is where a person carried from anywhere in the rail is dropped. Each person
 * is a handle of their own, so the two drags never start from the same press.
 */
export function RailVoiceList({
  channels,
  activeChannelId,
  selfId,
  movingUserId,
  onJoin,
  onOpenSettings,
  menuFor,
  personMenuFor,
  sort: { list, item, handle },
  canMoveOthers = false,
  carry,
  arrives,
  onJoinActivity,
  onOpenActivity,
}: RailVoiceListProps) {
  return (
    <div className="rail-list" ref={list}>
      {channels.map((channel) => (
        <VoiceChannelNode
          key={channel.id}
          channel={channel}
          active={channel.id === activeChannelId}
          selfId={selfId}
          movingUserId={
            channel.occupants.some((person) => person.id === movingUserId) ? movingUserId : null
          }
          onJoin={onJoin}
          onOpenSettings={onOpenSettings}
          menuFor={menuFor}
          personMenuFor={personMenuFor}
          itemRef={item(channel.id)}
          drag={handle?.(channel.id)}
          canMoveOthers={canMoveOthers}
          carry={carry}
          arrives={arrives}
          onJoinActivity={onJoinActivity}
          onOpenActivity={onOpenActivity}
        />
      ))}
    </div>
  );
}
