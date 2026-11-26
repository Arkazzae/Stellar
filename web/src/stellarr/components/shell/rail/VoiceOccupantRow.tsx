import { memo, useMemo, useState, type MouseEventHandler } from "react";

import { Icon } from "@stellarr/icons";
import { ActivityIcon } from "@/components/activities/ActivityIcon";
import { Avatar } from "@/components/primitives/Avatar";
import type { PersonVM } from "@/components/view-models";
import type { DragHandleProps } from "@/lib/drag/useRailDrag";
import { useVoiceSeat } from "@/components/voice/useVoiceSeat";
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
  /** Builds the row's right-click handler; the same function for every row. */
  menuFor?: (person: PersonVM) => MouseEventHandler<HTMLElement>;
  /**
   * Whether this person is sitting down rather than moving here from another
   * channel; asked once, when the row mounts. Without it every row is an
   * arrival.
   */
  arrives?: (personId: string) => boolean;
  /** The channel's activity, when this person is inside it: its mark follows the name. */
  activity?: { install: string; name: string; icon: string | null };
}

/**
 * Someone seated under a voice channel.
 *
 * Mute and speech are printed as well as drawn, since this list is where you
 * learn who is on the call. When the row can be moved, the whole row is the
 * drag handle, and a focusable button for the keyboard.
 *
 * Someone in this client's call is drawn from their seat, which the row
 * follows on its own; anyone else from what the server says of them.
 */
export const VoiceOccupantRow = memo(function VoiceOccupantRow({
  person,
  self,
  moving,
  drag,
  menuFor,
  arrives,
  activity,
}: VoiceOccupantRowProps) {
  const { t } = useLanguage();
  // Your own row greets you when you join, not each time you change rooms.
  const [arriving] = useState(() => self && (arrives?.(person.id) ?? true));
  const onContextMenu = useMemo(() => menuFor?.(person), [menuFor, person]);
  const seat = useVoiceSeat(person.id);
  const speaking = seat?.speaking ?? person.speaking === true;
  const muted = seat?.muted ?? person.muted === true;
  const sharing = seat?.sharing ?? person.sharing === true;
  const camera = seat?.camera ?? person.camera === true;
  const enabled = drag !== undefined;
  const className = [
    "voice-occupant",
    self && "me",
    arriving && "arriving",
    speaking && "speaking",
    muted && "muted",
    enabled && "is-draggable",
    moving && "is-moving",
  ]
    .filter(Boolean)
    .join(" ");

  const state = moving
    ? t("shell.moving")
    : muted
      ? t("shell.muted")
      : speaking
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
        speaking={speaking}
      />
      <span className="occ-name">{person.displayName}</span>
      {activity === undefined ? null : (
        <span
          className="occ-act"
          role="img"
          aria-label={t("activities.inActivity", { name: activity.name })}
          title={t("activities.inActivity", { name: activity.name })}
        >
          <ActivityIcon install={activity.install} name={activity.name} icon={activity.icon} size={14} />
        </span>
      )}
      {camera ? (
        <span className="occ-camera" role="img" aria-label={t("shell.cameraOn")} title={t("shell.cameraOn")}>
          <Icon name="camera" size={13} />
        </span>
      ) : null}
      {sharing ? <span className="live-badge occ-live">{t("share.live")}</span> : null}
      {muted ? (
        <span className="occ-mic" aria-hidden="true">
          <Icon name="micSlash" size={13} />
        </span>
      ) : null}
      <span className="sr-only">{state}</span>
    </li>
  );
});
