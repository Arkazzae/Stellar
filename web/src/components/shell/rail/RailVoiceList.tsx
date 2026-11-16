"use client";

import type { MouseEventHandler, ReactNode } from "react";
import { createPortal } from "react-dom";

import { Avatar } from "@/components/primitives/Avatar";
import { ChannelButton } from "@/components/shell/rail/ChannelButton";
import type { ChannelMenuRaiser } from "@/components/shell/rail/railTypes";
import type { VoiceMove } from "@/components/shell/rail/useVoiceMove";
import { VoiceOccupantRow } from "@/components/shell/rail/VoiceOccupantRow";
import type { VoiceDragData } from "@/components/shell/rail/voiceMove";
import type { PersonVM, VoiceChannelVM } from "@/components/view-models";
import { DragAnnouncer } from "@/lib/drag/DragAnnouncer";
import { useZoneDrag } from "@/lib/drag/useZoneDrag";
import { useLanguage } from "@/state/hooks";

function VoiceDropZone({
  zoneRef,
  isOver,
  children,
}: {
  zoneRef: (node: HTMLElement | null) => void;
  isOver: boolean;
  children: ReactNode;
}) {
  return (
    <div
      ref={zoneRef}
      className={isOver ? "voice-channel-drop is-over" : "voice-channel-drop"}
    >
      {children}
    </div>
  );
}

/** One seated person's drag key: the same person can only sit in one channel at a time. */
const occupantKey = (channelId: string, personId: string) => `${channelId}:${personId}`;

export interface RailVoiceListProps {
  channels: VoiceChannelVM[];
  activeChannelId: string | null;
  /** Whose occupant row gets `.me`. */
  selfId: string;
  /** The move in progress, shared by every list in the rail. */
  move: VoiceMove;
  /** Whether staff may move people between channels. */
  canMove: boolean;
  onJoin: (channelId: string) => void;
  /** Already gated on who may edit; absent hides the cog. */
  onOpenSettings?: (channelId: string) => void;
  menuFor?: ChannelMenuRaiser;
  personMenuFor?: (person: PersonVM) => MouseEventHandler<HTMLElement>;
}

/**
 * The voice channels of one rail group, each with the people seated under it.
 *
 * Every list runs its own drag over its own channels, so a group collapsing or
 * a section being empty never leaves a drop target behind, and the copy of the
 * person being carried is painted by the list they are carried from. What a
 * drop starts, waiting for the server, is shared across the rail through
 * `move`.
 */
export function RailVoiceList({
  channels,
  activeChannelId,
  selfId,
  move,
  canMove,
  onJoin,
  onOpenSettings,
  menuFor,
  personMenuFor,
}: RailVoiceListProps) {
  const { t } = useLanguage();
  const nameOf = (channelId: string) =>
    channels.find((channel) => channel.id === channelId)?.name;
  const movable = (person: PersonVM) =>
    canMove && person.id !== selfId && move.movingUserId !== person.id;

  const drag = useZoneDrag<VoiceDragData>({
    items: channels.flatMap((channel) =>
      channel.occupants
        .filter(movable)
        .map((person) => occupantKey(channel.id, person.id)),
    ),
    zones: channels.map((channel) => channel.id),
    homeOf: (dragged) => dragged.sourceChannelId,
    onDrop: move.onDrop,
    roleDescription: t("shell.dragHandle"),
    messages: {
      lifted: (dragged) => t("shell.dragLifted", { name: dragged.person.displayName }),
      over: (channelId) => {
        const name = nameOf(channelId);
        return name === undefined ? undefined : t("shell.dragOver", { name });
      },
      dropped: (dragged, channelId) => {
        const name = channelId === null ? undefined : nameOf(channelId);
        return name === undefined
          ? t("shell.dragCancelled")
          : t("shell.dragDropped", { user: dragged.person.displayName, channel: name });
      },
      cancelled: () => t("shell.dragCancelled"),
    },
  });
  const { carried } = drag;

  return (
    <>
      {channels.map((channel) => {
        const count = channel.occupants.length;
        return (
          <VoiceDropZone
            key={channel.id}
            zoneRef={drag.zoneRef(channel.id)}
            isOver={drag.over === channel.id}
          >
            <ChannelButton
              kind="voice"
              name={channel.name}
              active={channel.id === activeChannelId}
              count={count}
              countLabel={t("shell.peopleInCall", { count })}
              onOpenSettings={
                onOpenSettings === undefined
                  ? undefined
                  : () => onOpenSettings(channel.id)
              }
              onSelect={() => onJoin(channel.id)}
              onContextMenu={menuFor?.({
                id: channel.id,
                kind: "voice",
                name: channel.name,
                unread: 0,
                sectionId: channel.sectionId,
              })}
            />
            <ul className="voice-occupants">
              {channel.occupants.map((person) => {
                const key = occupantKey(channel.id, person.id);
                return (
                  <VoiceOccupantRow
                    key={person.id}
                    person={person}
                    self={person.id === selfId}
                    moving={move.movingUserId === person.id}
                    drag={
                      movable(person)
                        ? {
                            dragging: carried?.key === key,
                            handle: drag.handle(key, {
                              type: "voice-occupant",
                              person,
                              sourceChannelId: channel.id,
                            }),
                          }
                        : undefined
                    }
                    onContextMenu={personMenuFor?.(person)}
                  />
                );
              })}
            </ul>
          </VoiceDropZone>
        );
      })}
      <DragAnnouncer
        id={drag.instructionsId}
        instructions={t("shell.dragInstructions")}
        announcement={drag.announcement}
      />
      {carried === null || typeof document === "undefined"
        ? null
        : createPortal(
            <div className="voice-occupant-carry" style={carried.style}>
              <div className="voice-occupant-overlay">
                <Avatar
                  id={carried.item.person.id}
                  displayName={carried.item.person.displayName}
                  initials={carried.item.person.initials}
                  avatarUrl={carried.item.person.avatarUrl}
                  size="xs"
                />
                <span>{carried.item.person.displayName}</span>
              </div>
            </div>,
            document.body,
          )}
    </>
  );
}
