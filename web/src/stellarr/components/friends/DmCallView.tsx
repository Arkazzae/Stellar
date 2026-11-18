import type { ReactNode } from "react";

import type { PersonVM, ScreenShareVM } from "@/components/view-models";
import { CallClock } from "@/components/voice/CallClock";
import { ParticipantTile } from "@/components/voice/ParticipantTile";
import {
  participantTileProps,
  type ParticipantTileSources,
} from "@/components/voice/ParticipantTiles";
import { ShareBand } from "@/components/voice/ShareBand";
import { useLanguage } from "@/state/LanguageProvider";

export interface DmCallViewProps extends ParticipantTileSources {
  /** The other person, with what the call knows of their voice. */
  peer: PersonVM;
  /** You, with your own face and whether your microphone is off. */
  self: PersonVM;
  /** You started the call and the other person has not joined it. */
  ringing: boolean;
  /** When the call began, for its duration; null keeps the time off. */
  since: number | null;
  /** Stands in for the duration while the call is not up, such as "Connecting". */
  status?: string;
  /** The call is encrypted end to end, which the line under the name says. */
  encrypted?: boolean;
  /** Null while nobody shares a screen. */
  share: ScreenShareVM | null;
  /** The screen is your own, which plays back muted. */
  localShare?: boolean;
  onWatchShare: () => void;
  onStopWatchingShare: () => void;
  /** Cinema mode: a shared screen keeps the room to itself. */
  cinema?: boolean;
  /** What went wrong with the call, above it. */
  notice?: ReactNode;
  /** The control bar, under the call. */
  bar: ReactNode;
}

/**
 * A call in a conversation, given the whole room.
 *
 * There are two people, so there is no grid to arrange. The other person has
 * the middle: their face, ringed while they speak, their name and how long
 * the call has lasted, or their camera in a frame instead of the face. You
 * are the small tile in the corner. A shared screen, yours or theirs, takes
 * the middle, and the other person moves to a small tile above yours.
 *
 * Every person is a voice tile, so their camera, the note that it is held
 * back, their volume, gate and suppressor and their menu are the ones a
 * channel call has.
 */
export function DmCallView({
  peer,
  self,
  ringing,
  since,
  status,
  encrypted = false,
  share,
  localShare = false,
  onWatchShare,
  onStopWatchingShare,
  cinema = false,
  notice,
  bar,
  ...sources
}: DmCallViewProps) {
  const { t } = useLanguage();
  const them = participantTileProps(peer, sources);
  // Your own tile has no menu: there is nobody in it to turn down.
  const you = { ...participantTileProps(self, sources), onOpenProfile: undefined };
  const { participantVolumes, onParticipantVolumeChange } = sources;

  const meta =
    status ?? (ringing ? t("dm.calling") : since === null ? null : <CallClock since={since} />);

  const classes = ["dm-call", share !== null && "sharing", cinema && share !== null && "cinema"]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={classes}>
      {notice}
      <div className="dm-call-stage">
        {share !== null ? (
          <ShareBand
            share={share}
            local={localShare}
            volume={participantVolumes?.get(share.ownerId)}
            onVolumeChange={
              onParticipantVolumeChange &&
              ((volume) => onParticipantVolumeChange(share.ownerId, volume))
            }
            onWatch={onWatchShare}
            onStopWatching={onStopWatchingShare}
          />
        ) : them.stream ? (
          <ParticipantTile {...them} className="dm-call-frame" />
        ) : (
          <div className="dm-call-who">
            <ParticipantTile {...them} className={ringing ? "dm-call-face ringing" : "dm-call-face"} />
            {/* How long, then how the call is protected, behind a dot. */}
            {meta === null && !encrypted ? null : (
              <p className="dm-call-meta">
                {meta === null ? null : <span>{meta}</span>}
                {encrypted ? <span>{t("dm.callEncrypted")}</span> : null}
              </p>
            )}
          </div>
        )}
      </div>
      <div className="dm-call-corner">
        {share === null ? null : <ParticipantTile {...them} className="dm-call-small" />}
        <ParticipantTile {...you} label={t("dm.you")} className="dm-call-small" />
      </div>
      {bar}
    </div>
  );
}
