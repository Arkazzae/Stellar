import { useEffect, useRef } from "react";

import { Icon } from "@stellarr/icons";
import { Avatar } from "@/components/primitives/Avatar";
import { useContextMenu } from "@/components/primitives/ContextMenu";
import { UserMenu } from "@/components/roster/UserMenu";
import type { OpenProfile, PersonVM } from "@/components/view-models";
import { useLanguage } from "@/state/LanguageProvider";

export interface ParticipantTileProps {
  person: PersonVM;
  /** The name on the tile, where it is not the person's own, such as "You". */
  label?: string;
  /** Where the tile sits in a layout of its own rather than the grid. */
  className?: string;
  /** Without a stream the tile shows the person's avatar. */
  stream?: MediaStream | null;
  /** The camera is on, but held back for the viewer's link: the avatar says so. */
  held?: boolean;
  /** Your own camera: played back muted so the room does not hear itself. */
  local?: boolean;
  /** Receiver-side mixer level. Without it the tile has no mixer strip. */
  volume?: number;
  onVolumeChange?: (volume: number) => void;
  /** Receiver-side gate on the same strip. Absent hides the control. */
  gated?: boolean;
  onGateChange?: (gated: boolean) => void;
  /** Receiver-side suppressor on the same strip. Absent hides the control. */
  suppressed?: boolean;
  onSuppressionChange?: (suppressed: boolean) => void;
  onOpenProfile?: OpenProfile;
}

/** One person on the voice stage. */
export function ParticipantTile({
  person,
  label,
  className,
  stream,
  held = false,
  local = false,
  volume,
  onVolumeChange,
  gated,
  onGateChange,
  suppressed,
  onSuppressionChange,
  onOpenProfile,
}: ParticipantTileProps) {
  const video = useRef<HTMLVideoElement>(null);
  const menu = useContextMenu();
  const { t } = useLanguage();

  // `srcObject` is a property, never an attribute. Clearing it on the way out
  // drops the element's hold on the tracks, so a stopped camera does not leave
  // a frozen frame behind.
  useEffect(() => {
    const element = video.current;
    if (!element) return;
    element.srcObject = stream ?? null;
    return () => {
      element.srcObject = null;
    };
  }, [stream]);

  // Your own tile has nobody to turn down, so it never offers the strip.
  const mixer =
    !local && volume !== undefined && onVolumeChange !== undefined
      ? { volume, onVolumeChange }
      : null;
  const hasMenu = mixer !== null || onOpenProfile !== undefined;

  const classes = [
    "tile",
    className,
    hasMenu && "has-actions",
    person.speaking && "speaking",
    person.muted && "muted",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={classes} onContextMenu={hasMenu ? menu.onContextMenu : undefined}>
      {hasMenu && menu.at !== null ? (
        <UserMenu
          at={menu.at}
          userId={person.id}
          displayName={person.displayName}
          self={local}
          anchor={menu.anchor}
          onOpenProfile={onOpenProfile}
          participantVolumeOf={mixer ? () => mixer.volume : undefined}
          onParticipantVolumeChange={
            mixer ? (_userId, next) => mixer.onVolumeChange(next) : undefined
          }
          participantGateOf={mixer && onGateChange ? () => gated ?? false : undefined}
          onParticipantGateChange={
            mixer && onGateChange ? (_userId, next) => onGateChange(next) : undefined
          }
          participantSuppressionOf={
            mixer && onSuppressionChange ? () => suppressed ?? false : undefined
          }
          onParticipantSuppressionChange={
            mixer && onSuppressionChange
              ? (_userId, next) => onSuppressionChange(next)
              : undefined
          }
          onClose={menu.close}
        />
      ) : null}
      {stream ? (
        <video
          ref={video}
          autoPlay
          playsInline
          muted={local}
          aria-label={t("voice.cameraOf", { name: person.displayName })}
        />
      ) : (
        <Avatar
          id={person.id}
          displayName={person.displayName}
          initials={person.initials}
          avatarUrl={person.avatarUrl}
          size="xl"
        />
      )}
      {held && !stream ? (
        <span className="t-held" role="status">
          {t("voice.pictureHeldBack")}
        </span>
      ) : null}
      <span className="t-name">
        <span>{label ?? person.displayName}</span>
      </span>
      <span className="t-mic" aria-hidden="true">
        <Icon name="micSlash" size={14} />
      </span>
      {hasMenu ? (
        <button
          type="button"
          className="tile-more"
          aria-label={t("common.moreActions", { name: person.displayName })}
          // With no profile to open the menu is only the mixer strip, which
          // the menu presents as a dialog.
          aria-haspopup={onOpenProfile === undefined ? "dialog" : "menu"}
          aria-expanded={menu.at !== null}
          onClick={(event) => {
            const trigger = event.currentTarget;
            const bounds = trigger.getBoundingClientRect();
            menu.openAt(trigger, bounds.right, bounds.bottom + 4);
          }}
        >
          <Icon name="dots" size={16} />
        </button>
      ) : null}
    </div>
  );
}
