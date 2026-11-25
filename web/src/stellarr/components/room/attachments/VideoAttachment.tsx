import { useEffect, useRef, useState } from "react";

import { Icon } from "@stellarr/icons";
import type { VideoPart } from "@/lib/messages/parts";
import { useLanguage } from "@/state/LanguageProvider";

import { formatMediaTime } from "./mediaTime";
import { PlayButton, PlaybackSlider, VolumeControl } from "./PlaybackControls";
import { useMediaPlayback } from "./useMediaPlayback";

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

  const canExpand = document.fullscreenEnabled || "webkitEnterFullscreen" in HTMLVideoElement.prototype;
  const aspectRatio = part.width && part.height ? part.width / part.height : undefined;
  return (
    <div ref={frame} className={expanded ? "att att-video expanded" : "att att-video"}>
      <div className="video-stage" style={{ aspectRatio }}>
        {part.url && !player.failed ? (
          <>
            <video {...player.mediaProps} src={part.url} poster={part.thumb} preload="metadata"
              playsInline autoPlay={autoPlay} aria-label={name} />
            {/* The entire picture is a pointer target. The labelled button below handles keyboard playback. */}
            <button type="button" className="video-surface" tabIndex={-1} aria-hidden="true"
              onClick={() => void player.toggle()}>
              {!player.playing && <span><Icon name="play" size={20} /></span>}
            </button>
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
      {part.url && !player.failed && (
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
      )}
    </div>
  );
}
