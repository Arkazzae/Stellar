import { useEffect, useId, useRef, useState, type ReactNode } from "react";

import { Icon } from "@stellarr/icons";
import type { PersonVM, ScreenShareVM } from "@/components/view-models";
import { useLanguage } from "@/state/LanguageProvider";

export interface ScreenTileProps {
  share: ScreenShareVM;
  /** Where the tile sits in a layout of its own rather than the stage's. */
  className?: string;
  /** The tile has the stage: it fills it and carries the picture's controls. */
  focused?: boolean;
  /** The share has just ended; the tile says so for a moment before it goes. */
  ended?: boolean;
  /** Who watches this screen, when the call knows. */
  viewers?: readonly PersonVM[];
  /** Local receiver-side level for the person sharing, from 0 to 200. */
  volume?: number;
  onVolumeChange?: (volume: number) => void;
  /** Your own screen only: the preview is hidden to spare the browser. */
  previewHidden?: boolean;
  onTogglePreview?: () => void;
  onStopSharing?: () => void;
  /** Opt in to this screen; the stage gives it the focus at the same time. */
  onWatch: () => void;
  onStopWatching: () => void;
  /** Give the tile the stage, or take it back when it has it. */
  onFocus?: () => void;
  /** Back to the grid; the picture keeps playing in its tile. */
  onShowGrid?: () => void;
}

/**
 * One shared screen on the voice stage: a tile among the people, or, with the
 * focus, the stage itself with its controls laid over the picture.
 *
 * The tile always says why there is no picture when there is none: nobody
 * here watches it yet, the picture is on its way, the link has no room for
 * it, it does not decrypt, your own preview is hidden, or the share ended.
 */
export function ScreenTile({
  share,
  className,
  focused = false,
  ended = false,
  viewers,
  volume,
  onVolumeChange,
  previewHidden = false,
  onTogglePreview,
  onStopSharing,
  onWatch,
  onStopWatching,
  onFocus,
  onShowGrid,
}: ScreenTileProps) {
  const { t } = useLanguage();
  const root = useRef<HTMLDivElement>(null);
  const video = useRef<HTMLVideoElement>(null);
  const volumeId = useId();
  const [fullscreen, setFullscreen] = useState(false);

  const stream = ended || (share.local && previewHidden) ? null : share.stream;

  // `srcObject` is a property, never an attribute. Clearing it on the way out
  // drops the element's hold on the tracks, so a stopped share does not leave
  // a frozen frame behind.
  useEffect(() => {
    const element = video.current;
    if (!element) return;
    element.srcObject = stream;
    return () => {
      element.srcObject = null;
    };
  }, [stream]);

  useEffect(() => {
    const follow = () => setFullscreen(document.fullscreenElement === root.current);
    document.addEventListener("fullscreenchange", follow);
    return () => document.removeEventListener("fullscreenchange", follow);
  }, []);

  const name = share.local ? t("share.yourScreen") : share.ownerName;
  const unwatched = !ended && !share.local && !share.watching;
  const canPip =
    stream !== null &&
    typeof document !== "undefined" &&
    "pictureInPictureEnabled" in document &&
    document.pictureInPictureEnabled;

  const toggleFullscreen = () => {
    const element = root.current;
    if (element === null) return;
    if (document.fullscreenElement === element) void document.exitFullscreen();
    else void element.requestFullscreen?.();
  };
  const togglePip = () => {
    const element = video.current;
    if (element === null) return;
    if (document.pictureInPictureElement === element) void document.exitPictureInPicture();
    else void element.requestPictureInPicture();
  };

  let note: ReactNode = null;
  if (ended) {
    note = <TileNote icon={<Icon name="screencast" size={20} />} text={t("share.ended")} />;
  } else if (share.sealed) {
    note = (
      <TileNote
        icon={<Icon name="lock" size={20} />}
        text={t("voice.undecryptableVideo", { name: share.ownerName })}
      />
    );
  } else if (share.heldBack) {
    note = <TileNote icon={<Icon name="screencast" size={20} />} text={t("voice.pictureHeldBack")} />;
  } else if (share.connecting) {
    note = <TileNote icon={<span className="st-ring" aria-hidden="true" />} text={t("share.connecting")} />;
  } else if (share.local && previewHidden) {
    note = (
      <TileNote icon={<Icon name="monitor" size={20} />} text={t("share.previewHidden")}>
        {onTogglePreview ? (
          <button type="button" className="st-action" onClick={onTogglePreview}>
            {t("share.showPreview")}
          </button>
        ) : null}
      </TileNote>
    );
  } else if (unwatched) {
    note = (
      <span className="st-unwatched" aria-hidden="true">
        <Icon name="screencast" size={22} />
        <span className="watch-btn">
          <Icon name="play" size={13} /> {t("share.watch")}
        </span>
      </span>
    );
  }

  const classes = [
    "tile",
    "screen",
    className,
    focused && "is-focus",
    stream !== null && "has-video",
    unwatched && "unwatched",
    ended && "ended",
    fullscreen && "is-fullscreen",
  ]
    .filter(Boolean)
    .join(" ");

  // With the stage the picture takes no click: a stray one must not throw the
  // viewer out of what they are watching. "Grid" does that on purpose.
  const hit = !ended && (unwatched || (!focused && onFocus !== undefined));
  const hitLabel = unwatched
    ? t("share.watchOf", { name: share.ownerName })
    : share.local
      ? t("stage.focusOf", { name })
      : t("stage.focusScreenOf", { name: share.ownerName });

  return (
    <div ref={root} className={classes} data-key={share.key}>
      {stream ? (
        <video
          ref={video}
          autoPlay
          playsInline
          muted
          aria-label={t("share.screenLabel", { name: share.ownerName })}
        />
      ) : null}

      {stream === null && unwatched && share.posterUrl ? (
        <img className="st-poster" src={share.posterUrl} alt="" />
      ) : null}

      {hit ? (
        <button
          type="button"
          className="tile-hit"
          aria-label={hitLabel}
          onClick={unwatched ? onWatch : onFocus}
        />
      ) : null}

      {note}

      <span className="st-label">
        <Icon name="screencast" size={12} />
        <span>{name}</span>
        {ended ? null : <span className="live-badge">{t("share.live")}</span>}
      </span>

      {ended ? null : (
        <div className="st-chrome">
          <div className="st-row top">
            <div className="st-group">
              <span className="st-chip">
                <Icon name="screencast" size={14} />
                <span className="st-chip-name">{name}</span>
                <span className="live-badge">{t("share.live")}</span>
              </span>
              {viewers && viewers.length > 0 ? <Viewers people={viewers} /> : null}
            </div>
            <div className="st-group">
              {focused && canPip ? (
                <button
                  type="button"
                  className="st-icon"
                  aria-label={t("share.pictureInPicture")}
                  title={t("share.pictureInPicture")}
                  onClick={togglePip}
                >
                  <Icon name="popout" size={16} />
                </button>
              ) : null}
              {focused && stream !== null ? (
                <button
                  type="button"
                  className="st-icon"
                  aria-label={fullscreen ? t("voiceBar.fullscreenExit") : t("voiceBar.fullscreen")}
                  title={fullscreen ? t("voiceBar.fullscreenExit") : t("voiceBar.fullscreen")}
                  aria-pressed={fullscreen}
                  onClick={toggleFullscreen}
                >
                  <Icon name={fullscreen ? "collapse" : "expand"} size={16} />
                </button>
              ) : null}
            </div>
          </div>

          <div className="st-row bottom">
            <div className="st-group">
              {!share.local && share.watching && share.hasAudio && volume !== undefined && onVolumeChange ? (
                <span
                  className="st-chip st-volume"
                  role="group"
                  aria-label={t("voice.participantVolume", { name: share.ownerName })}
                >
                  <Icon name="speaker" size={15} />
                  <label className="sr-only" htmlFor={volumeId}>
                    {t("voice.participantVolume", { name: share.ownerName })}
                  </label>
                  <input
                    id={volumeId}
                    type="range"
                    min={0}
                    max={200}
                    step={5}
                    value={volume}
                    onChange={(event) => onVolumeChange(Number(event.target.value))}
                  />
                  <output htmlFor={volumeId}>{volume}%</output>
                </span>
              ) : null}
              {!share.local && share.watching && !share.hasAudio ? (
                <span className="st-chip" role="img" aria-label={t("share.noAudio")}>
                  <Icon name="volumeCross" size={15} />
                  <span className="st-chip-text" aria-hidden="true">{t("share.noAudio")}</span>
                </span>
              ) : null}
            </div>
            <div className="st-group">
              {focused && onShowGrid ? (
                <button type="button" className="st-chip st-button" aria-label={t("stage.grid")} onClick={onShowGrid}>
                  <Icon name="apps" size={15} />
                  <span className="st-chip-text">{t("stage.grid")}</span>
                </button>
              ) : null}
              {share.local && onTogglePreview && !previewHidden ? (
                <button
                  type="button"
                  className="st-chip st-button"
                  aria-label={t("share.hidePreview")}
                  onClick={onTogglePreview}
                >
                  <Icon name="monitor" size={15} />
                  <span className="st-chip-text">{t("share.hidePreview")}</span>
                </button>
              ) : null}
              {share.local && onStopSharing ? (
                <button
                  type="button"
                  className="st-chip st-button"
                  aria-label={t("voiceBar.stopSharing")}
                  onClick={onStopSharing}
                >
                  <Icon name="stop" size={15} />
                  <span className="st-chip-text">{t("voiceBar.stopSharing")}</span>
                </button>
              ) : null}
              {!share.local && share.watching ? (
                <button
                  type="button"
                  className="st-chip st-button"
                  aria-label={t("share.stopWatching")}
                  onClick={onStopWatching}
                >
                  <Icon name="x" size={15} />
                  <span className="st-chip-text">{t("share.stopWatching")}</span>
                </button>
              ) : null}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/** Why a screen tile has no picture, in one line in its middle. */
function TileNote({ icon, text, children }: { icon: ReactNode; text: string; children?: ReactNode }) {
  return (
    <span className="st-note" role="status">
      {icon}
      <span>{text}</span>
      {children}
    </span>
  );
}

/** The faces of who watches, and how many. */
function Viewers({ people }: { people: readonly PersonVM[] }) {
  const { t } = useLanguage();
  const names = people.map((person) => person.displayName).join(", ");
  const label = t("share.watchedBy", { names });
  const shown = people.slice(0, 3);
  return (
    <span className="st-chip st-viewers" role="img" aria-label={label} title={label}>
      <Icon name="eye" size={14} />
      <span className="st-faces" aria-hidden="true">
        {shown.map((person) =>
          person.avatarUrl ? (
            <img key={person.id} src={person.avatarUrl} alt="" />
          ) : (
            <span key={person.id} className="st-face-initials">
              {person.initials}
            </span>
          ),
        )}
      </span>
      <span aria-hidden="true">{people.length}</span>
    </span>
  );
}
