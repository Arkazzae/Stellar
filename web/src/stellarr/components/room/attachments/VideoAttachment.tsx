import { useEffect, useRef, useState, type SyntheticEvent } from "react";

import { Icon } from "@stellarr/icons";
import type { VideoPart } from "@/lib/messages/parts";
import { useLanguage } from "@/state/LanguageProvider";

import { formatMediaTime } from "./mediaTime";
import { PlayButton, PlaybackSlider, VolumeControl } from "./PlaybackControls";
import { useMediaPlayback } from "./useMediaPlayback";
import { videoAspectRatio, videoFrameStyle } from "./videoGeometry";

interface VideoAttachmentProps {
  part: VideoPart;
  autoPlay?: boolean;
  expanded?: boolean;
}

export function VideoAttachment(props: VideoAttachmentProps) {
  return <VideoPlayer key={props.part.url} {...props} />;
}

interface FullscreenVideo extends HTMLVideoElement {
  webkitEnterFullscreen?: () => void;
}

function VideoPlayer({ part, autoPlay, expanded }: VideoAttachmentProps) {
  const { t } = useLanguage();
  const player = useMediaPlayback();
  const frame = useRef<HTMLDivElement>(null);
  const [intrinsicRatio, setIntrinsicRatio] = useState<number>();
  const [touchControls, setTouchControls] = useState(false);
  const pointer = useRef("");
  const [fullscreen, setFullscreen] = useState(false);
  const [fullscreenError, setFullscreenError] = useState(false);
  const name = part.name || t("media.video");
  const time = formatMediaTime(player.currentTime);
  const total = player.duration ? formatMediaTime(player.duration) : part.duration;
  const fullscreenLabel = t(fullscreen ? "voiceBar.leaveFullscreen" : "voiceBar.fullscreen");

  useEffect(() => {
    const node = frame.current;
    const follow = () => setFullscreen(document.fullscreenElement === node);
    node?.addEventListener("fullscreenchange", follow);
    return () => node?.removeEventListener("fullscreenchange", follow);
  }, []);

  async function toggleFullscreen() {
    const node = frame.current;
    if (!node) return;
    try {
      if (document.fullscreenElement === node) await document.exitFullscreen();
      else if (document.fullscreenEnabled) await node.requestFullscreen();
      else node.querySelector<FullscreenVideo>("video")?.webkitEnterFullscreen?.();
      setFullscreenError(false);
    } catch {
      setFullscreenError(true);
    }
  }

  const measure = ({ currentTarget }: SyntheticEvent<HTMLVideoElement>) => {
    setIntrinsicRatio(videoAspectRatio(currentTarget.videoWidth, currentTarget.videoHeight));
  };
  const canExpand = document.fullscreenEnabled || "webkitEnterFullscreen" in HTMLVideoElement.prototype;
  const ratio = intrinsicRatio ?? videoAspectRatio(part.width, part.height);
  const toggleLabel = t(player.playing ? "attachment.pause" : "attachment.play", { name });
  const ready = Boolean(part.url) && !player.failed;

  return (
    <div
      ref={frame}
      className={expanded ? "att att-video expanded" : "att att-video"}
      style={videoFrameStyle(ratio)}
      data-playing={player.playing || undefined}
      data-touch-controls={touchControls || undefined}
    >
      <div className="video-stage">
        {ready ? (
          <>
            <video
              {...player.mediaProps}
              src={part.url}
              poster={part.thumb}
              preload="metadata"
              playsInline
              autoPlay={autoPlay}
              aria-label={name}
              onLoadedMetadata={(event) => {
                player.mediaProps.onLoadedMetadata(event);
                measure(event);
              }}
              onResize={measure}
            />
            <button
              type="button"
              className="video-surface"
              aria-label={toggleLabel}
              onPointerDown={(event) => { pointer.current = event.pointerType; }}
              onClick={(event) => {
                const onTouch = event.detail > 0 && pointer.current === "touch";
                if (onTouch && player.playing) {
                  setTouchControls((visible) => !visible);
                } else {
                  if (!player.playing) setTouchControls(false);
                  void player.toggle();
                }
              }}
            >
              <span className="video-play-mark">
                <Icon name={player.playing ? "pause" : "play"} size={25} />
              </span>
            </button>
            <div className="video-controls">
              <PlaybackSlider
                className="video-seek"
                max={player.duration}
                step="any"
                value={Math.min(player.currentTime, player.duration)}
                disabled={!player.duration}
                aria-label={t("attachment.playbackPosition", { name })}
                aria-valuetext={total ? t("attachment.timeOfTotal", { current: time, duration: total }) : time}
                onChange={(event) => player.seek(event.currentTarget.valueAsNumber)}
              />
              <div className="video-transport">
                <PlayButton player={player} name={name} />
                <span className="video-time">{time}{total && ` / ${total}`}</span>
                <VolumeControl player={player} />
                {canExpand && (
                  <button type="button" className="media-tool" aria-label={fullscreenLabel} title={fullscreenLabel}
                    onClick={() => void toggleFullscreen()}>
                    <Icon name={fullscreen ? "collapse" : "expand"} size={16} />
                  </button>
                )}
              </div>
              {fullscreenError && <span role="status">{t("attachment.fullscreenUnavailable")}</span>}
            </div>
          </>
        ) : (
          <>
            {part.thumb && <img src={part.thumb} alt="" />}
            <div className="video-fallback" role="status">
              {part.url ? t("attachment.cannotPlay", { name }) : t("media.recordingUnavailable")}
              {part.url && !expanded && <a href={part.url} download={part.name || true}>{t("common.download")}</a>}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
