import type { MouseEventHandler } from "react";

import { ChannelButton } from "@/components/shell/rail/ChannelButton";
import type { ChannelMenuRaiser, RailSortRefs } from "@/components/shell/rail/railTypes";
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
  /** The handle that carries a person to another channel; undefined for whoever cannot be moved. */
  carry?: (channelId: string, person: PersonVM) => DragHandleProps | undefined;
}

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
  carry,
}: RailVoiceListProps) {
  const { t } = useLanguage();

  return (
    <div className="rail-list" ref={list}>
      {channels.map((channel) => {
        const count = channel.occupants.length;
        return (
          <div key={channel.id} ref={item(channel.id)} className="voice-channel-drop">
            <ChannelButton
              kind="voice"
              name={channel.name}
              active={channel.id === activeChannelId}
              count={count}
              countLabel={t("shell.peopleInCall", { count })}
              onOpenSettings={
                onOpenSettings === undefined ? undefined : () => onOpenSettings(channel.id)
              }
              onSelect={() => onJoin(channel.id)}
              onContextMenu={menuFor?.({
                id: channel.id,
                kind: "voice",
                name: channel.name,
                unread: 0,
                sectionId: channel.sectionId,
              })}
              drag={handle?.(channel.id)}
            />
            <ul className="voice-occupants">
              {channel.occupants.map((person) => (
                <VoiceOccupantRow
                  key={person.id}
                  person={person}
                  self={person.id === selfId}
                  moving={movingUserId === person.id}
                  drag={carry?.(channel.id, person)}
                  onContextMenu={personMenuFor?.(person)}
                />
              ))}
            </ul>
          </div>
        );
      })}
    </div>
  );
}
