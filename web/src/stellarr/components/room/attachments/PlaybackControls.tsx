import type { ComponentProps, CSSProperties } from "react";

import { Icon } from "@stellarr/icons";
import { useLanguage } from "@/state/LanguageProvider";

import type { MediaPlayback } from "./useMediaPlayback";

type SliderProps = ComponentProps<"input"> & { value: number; max: number };

export function PlaybackSlider({ value, max, ...props }: SliderProps) {
  const progress = max > 0 ? Math.min(100, Math.max(0, value / max * 100)) : 0;
  return (
    <input
      {...props}
      type="range"
      min={0}
      max={max}
      value={value}
      style={{ "--media-progress": `${progress}%` } as CSSProperties}
    />
  );
}

export function PlayButton({ player, name, disabled }: { player: MediaPlayback; name: string; disabled?: boolean }) {
  const { t } = useLanguage();
  const label = t(player.failed ? "attachment.cannotPlay" : player.playing ? "attachment.pause" : "attachment.play", { name });
  return (
    <button type="button" className="play-btn" aria-label={label} title={label}
      disabled={disabled || player.failed} onClick={() => void player.toggle()}>
      <Icon name={player.playing ? "pause" : "play"} size={13} />
    </button>
  );
}

export function VolumeControl({ player }: { player: MediaPlayback }) {
  const { t } = useLanguage();
  const percent = Math.round(player.volume * 100);
  const label = t(percent === 0 ? "shell.unmuteSound" : "shell.muteSound");
  return (
    <div className="media-volume">
      <button type="button" className="media-tool" aria-label={label} title={label} onClick={player.toggleMute}>
        <Icon name={percent === 0 ? "volumeCross" : "speaker"} size={15} />
      </button>
      <PlaybackSlider
        max={100}
        value={percent}
        aria-label={t("voice.volume")}
        aria-valuetext={`${percent}%`}
        onChange={(event) => player.changeVolume(event.currentTarget.valueAsNumber / 100)}
      />
    </div>
  );
}
