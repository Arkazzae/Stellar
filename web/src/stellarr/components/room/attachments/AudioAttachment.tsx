import {
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
} from "react";

import { Icon } from "@stellarr/icons";
import { formatBytes } from "@/lib/format/format";
import { useLanguage } from "@/state/hooks";

import type { AudioPart } from "@/lib/messages/parts";

import { PlayButton, VolumeControl } from "./PlaybackControls";
import { useMediaPlayback } from "./useMediaPlayback";
import { formatMediaTime } from "./mediaTime";
import { BAR_COUNT, MAX_PEAK_BYTES, useAudioPeaks } from "./peaks";

/** Flat and quiet on purpose: a placeholder should not read as a measurement. */
const PLACEHOLDER = Array.from({ length: BAR_COUNT }, () => 0.22);

/** Keyboard seek step, in seconds. */
const SEEK_STEP = 5;

const MIN_BAR_PX = 3;
const BAR_RANGE_PX = 19;

export interface AudioAttachmentProps {
  part: AudioPart;
  autoPlay?: boolean;
}

export function AudioAttachment({ part, autoPlay }: AudioAttachmentProps) {
  // Playback state belongs to one resource: a refreshed signed URL must not
  // inherit the expired one's disabled player or stale duration.
  return <AudioPlayer key={part.url ?? ""} part={part} autoPlay={autoPlay} />;
}

function clampPeaks(peaks: readonly number[]): number[] {
  return peaks.map((peak) => Math.min(Math.max(peak, 0), 1));
}

function AudioPlayer({ part, autoPlay }: AudioAttachmentProps) {
  const { t } = useLanguage();
  const wave = useRef<HTMLDivElement | null>(null);
  const player = useMediaPlayback();
  const { playing, currentTime, duration, failed } = player;
  /** Where the pointer is along the waveform, 0..1, or null when it is elsewhere. */
  const [hover, setHover] = useState<number | null>(null);

  const hasUrl = part.url !== undefined && part.url !== "";
  const playable = hasUrl && !failed;
  const seekable = playable && duration > 0;

  // Decoding a file nobody can play, or a large one, is a download the reader
  // did not ask for.
  const measured = useAudioPeaks(
    part.url,
    playable && part.peaks === undefined && (part.size === undefined || part.size <= MAX_PEAK_BYTES),
  );
  const bars = useMemo(
    () => (part.peaks === undefined ? (measured ?? PLACEHOLDER) : clampPeaks(part.peaks)),
    [part.peaks, measured],
  );

  const progress = duration > 0 ? Math.min(1, currentTime / duration) : 0;
  const played = Math.round(progress * bars.length);
  const previewed = hover === null ? null : Math.round(hover * bars.length);
  const previewing = hover !== null && seekable;

  const isVoiceMessage = part.voiceMessage === true;
  const spokenName = isVoiceMessage ? t("attachment.voiceMessage") : part.name;
  const displayName = isVoiceMessage ? t("attachment.voiceMessageLabel") : part.name;
  const totalLabel = duration > 0 ? formatMediaTime(duration) : part.duration;

  const ratioAt = (event: PointerEvent<HTMLDivElement>): number | null => {
    const box = wave.current?.getBoundingClientRect();
    if (box === undefined || box.width === 0) return null;
    return Math.min(1, Math.max(0, (event.clientX - box.left) / box.width));
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!seekable) return;
    let next: number;
    switch (event.key) {
      case "ArrowLeft": next = currentTime - SEEK_STEP; break;
      case "ArrowRight": next = currentTime + SEEK_STEP; break;
      case "Home": next = 0; break;
      case "End": next = duration; break;
      default: return;
    }
    event.preventDefault();
    player.seek(next);
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (!seekable) return;
    // Captured, so a drag keeps scrubbing after the pointer leaves the bar.
    event.currentTarget.setPointerCapture(event.pointerId);
    const ratio = ratioAt(event);
    if (ratio === null) return;
    setHover(ratio);
    player.seek(ratio * duration);
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (!seekable) return;
    const ratio = ratioAt(event);
    if (ratio === null) return;
    setHover(ratio);
    // Dragging scrubs; passing over only previews.
    if (event.currentTarget.hasPointerCapture(event.pointerId)) player.seek(ratio * duration);
  };

  return (
    <div
      className={["att att-audio", playing && "playing", isVoiceMessage && "voice-message"]
        .filter(Boolean)
        .join(" ")}
    >
      <audio {...player.mediaProps} src={part.url || undefined} preload="metadata" autoPlay={autoPlay} />
      <PlayButton player={player} name={spokenName} disabled={!hasUrl} />

      <div className="au-track">
        {/* A slider rather than decoration: it shows progress and takes a click
            anywhere along it, which is how a voice note is skipped into. */}
        <div
          ref={wave}
          className={seekable ? "wave seekable" : "wave"}
          role="slider"
          tabIndex={seekable ? 0 : -1}
          aria-label={t("attachment.playbackPosition", { name: spokenName })}
          aria-valuemin={0}
          aria-valuemax={Math.round(duration)}
          aria-valuenow={Math.round(currentTime)}
          aria-valuetext={
            duration > 0
              ? t("attachment.timeOfTotal", {
                  current: formatMediaTime(currentTime),
                  duration: formatMediaTime(duration),
                })
              : undefined
          }
          aria-disabled={seekable ? undefined : true}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerLeave={() => setHover(null)}
          onPointerCancel={() => setHover(null)}
          onKeyDown={onKeyDown}
        >
          {bars.map((peak, index) => (
            <i
              key={index}
              // What has played, what a click would add to it, what is ahead.
              className={
                index < played
                  ? "done"
                  : previewed !== null && index < previewed
                    ? "preview"
                    : undefined
              }
              // The floor keeps silence a line rather than a gap.
              style={{ height: `${MIN_BAR_PX + Math.round(peak * BAR_RANGE_PX)}px` }}
            />
          ))}
          {hover === null ? null : (
            <span
              className="wave-cursor"
              style={{ left: `${hover * 100}%` }}
              aria-hidden="true"
            />
          )}
        </div>

        {failed ? (
          <span className="audio-error" role="alert">
            {t("attachment.playbackError")}
          </span>
        ) : totalLabel === undefined ? null : (
          // Always both halves, even at 0:00: swapping between "0:06" and
          // "0:03 / 0:06" on hover would resize the label, and the waveform
          // takes the remaining width, so the bar would move under the pointer.
          <span className={previewing ? "dur preview" : "dur"}>
            {formatMediaTime(previewing ? hover * duration : currentTime)}
            {" / "}
            {totalLabel}
          </span>
        )}
      </div>

      {/* Full width, so the name lines up with the card's edge. */}
      <div className="au-foot">
        <span className="au-meta">
          {isVoiceMessage ? <Icon name="mic" size={12} /> : null}
          <b title={displayName}>{displayName}</b>
          {isVoiceMessage || part.size === undefined ? null : (
            <span>{formatBytes(part.size)}</span>
          )}
        </span>
        {!isVoiceMessage && playable && <VolumeControl player={player} />}
        {isVoiceMessage || !hasUrl ? null : (
          <a
            className="media-tool"
            href={part.url}
            download={part.name}
            aria-label={t("attachment.download", { name: part.name })}
            title={t("common.download")}
          >
            <Icon name="download" size={14} />
          </a>
        )}
      </div>
    </div>
  );
}
