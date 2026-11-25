import { useState, type ReactNode } from "react";

import type { PersonVM, ScreenShareVM } from "@/components/view-models";
import { CallClock } from "@/components/voice/CallClock";
import { ParticipantTile } from "@/components/voice/ParticipantTile";
import {
  participantTileProps,
  type ParticipantTileSources,
} from "@/components/voice/ParticipantTiles";
import { ScreenTile } from "@/components/voice/ScreenTile";
import { useChromeIdle } from "@/components/voice/useChromeIdle";
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
  /** The screens shared in the call, your own first. */
  shares: readonly ScreenShareVM[];
  onWatchShare: (key: string) => void;
  onStopWatchingShare: () => void;
  /** Stops your own share, from its tile. */
  onStopSharing?: () => void;
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
  shares,
  onWatchShare,
  onStopWatchingShare,
  onStopSharing,
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
  const chrome = useChromeIdle();
  const [chosenKey, setChosenKey] = useState<string | null>(null);
  const [previewHidden, setPreviewHidden] = useState(false);

  // The middle goes to the screen you picked, else the one you watch, else
  // your own, else theirs waiting to be watched. Any other one sits small in
  // the corner.
  const share =
    shares.find((item) => item.key === chosenKey) ??
    shares.find((item) => item.watching) ??
    shares.find((item) => item.local) ??
    shares[0] ??
    null;
  const others = shares.filter((item) => item !== share);
  const screenTile = (item: ScreenShareVM, focused: boolean) => (
    <ScreenTile
      key={item.key}
      share={item}
      className={focused ? "dm-call-screen" : "dm-call-small"}
      focused={focused}
      volume={participantVolumes?.get(item.ownerId)}
      onVolumeChange={
        onParticipantVolumeChange && ((volume) => onParticipantVolumeChange(item.ownerId, volume))
      }
      previewHidden={item.local && previewHidden}
      onTogglePreview={item.local ? () => setPreviewHidden((hidden) => !hidden) : undefined}
      onStopSharing={item.local ? onStopSharing : undefined}
      onWatch={() => {
        onWatchShare(item.key);
        setChosenKey(item.key);
      }}
      onStopWatching={() => {
        onStopWatchingShare();
        setChosenKey(null);
      }}
      onFocus={() => setChosenKey(item.key)}
    />
  );

  const meta =
    status ?? (ringing ? t("dm.calling") : since === null ? null : <CallClock since={since} />);

  const classes = ["dm-call", share !== null && "sharing", cinema && share !== null && "cinema"]
    .filter(Boolean)
    .join(" ");

  return (
    <div
      className={classes}
      data-chrome={chrome.idle ? "idle" : "awake"}
      onPointerMove={chrome.wake}
      onPointerDown={chrome.wake}
      onKeyDown={chrome.wake}
    >
      {notice}
      <div className="dm-call-stage">
        {share !== null ? (
          screenTile(share, true)
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
        {others.map((item) => screenTile(item, false))}
        {share === null ? null : <ParticipantTile {...them} className="dm-call-small" />}
        <ParticipantTile {...you} label={t("dm.you")} className="dm-call-small" />
      </div>
      {bar}
    </div>
  );
}
