import type { MouseEventHandler } from "react";

import { Icon } from "@stellarr/icons";
import { Avatar } from "@/components/primitives/Avatar";
import type { PersonVM } from "@/components/view-models";
import type { DragHandleProps } from "@/lib/drag/useRailDrag";
import { useLanguage } from "@/state/hooks";

export interface VoiceOccupantRowProps {
  person: PersonVM;
  self: boolean;
  /** The server has not confirmed this person's move yet. */
  moving: boolean;
  /**
   * Present for your own seat, and when staff may move this person and they
   * are not already in flight. The rail marks the row while its copy is
   * carried.
   */
  drag?: DragHandleProps;
  onContextMenu?: MouseEventHandler<HTMLLIElement>;
}

/**
 * Someone seated under a voice channel.
 *
 * Mute and speech are printed as well as drawn, since this list is where you
 * learn who is on the call. When the row can be moved, the whole row is the
 * drag handle, and a focusable button for the keyboard.
 */
export function VoiceOccupantRow({
  person,
  self,
  moving,
  drag,
  onContextMenu,
}: VoiceOccupantRowProps) {
  const { t } = useLanguage();
  const enabled = drag !== undefined;
  const className = [
    "voice-occupant",
    self && "me",
    person.speaking && "speaking",
    person.muted === true && "muted",
    enabled && "is-draggable",
    moving && "is-moving",
  ]
    .filter(Boolean)
    .join(" ");

  const state = moving
    ? t("shell.moving")
    : person.muted === true
      ? t("shell.muted")
      : person.speaking
        ? t("roster.speaking")
        : t("roster.silent");

  return (
    <li
      className={className}
      onContextMenu={onContextMenu}
      title={enabled ? t("shell.dragVoiceMember") : undefined}
      {...(enabled ? { role: "button", tabIndex: 0, ...drag } : {})}
      aria-label={
        enabled
          ? t("shell.moveVoiceMember", { name: person.displayName })
          : undefined
      }
      aria-busy={moving || undefined}
    >
      <Avatar
        id={person.id}
        displayName={person.displayName}
        initials={person.initials}
        avatarUrl={person.avatarUrl}
        size="xs"
        speaking={person.speaking}
      />
      <span className="occ-name">{person.displayName}</span>
      {person.muted === true ? (
        <span className="occ-mic" aria-hidden="true">
          <Icon name="micSlash" size={13} />
        </span>
      ) : null}
      <span className="sr-only">{state}</span>
    </li>
  );
}
