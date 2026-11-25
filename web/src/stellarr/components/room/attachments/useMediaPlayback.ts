import { useCallback, useRef, useState, type SyntheticEvent } from "react";

type MediaEvent = SyntheticEvent<HTMLMediaElement>;

function finiteDuration(element: HTMLMediaElement) {
  return Number.isFinite(element.duration) ? Math.max(0, element.duration) : 0;
}

/** The element owns playback. Controls also follow changes made by the OS or fullscreen player. */
export function useMediaPlayback() {
  const element = useRef<HTMLMediaElement | null>(null);
  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  const [failed, setFailed] = useState(false);

  const attach = useCallback((node: HTMLMediaElement | null) => {
    element.current = node;
    if (node) setDuration(finiteDuration(node));
  }, []);

  async function toggle() {
    const media = element.current;
    if (!media || failed) return;
    if (!media.paused) {
      media.pause();
      return;
    }
    if (media.ended) media.currentTime = 0;
    try {
      await media.play();
    } catch (error) {
      // A pause can interrupt a pending play, and a browser can require a
      // second user gesture after decryption. Both leave the resource usable.
      if (error instanceof DOMException && (error.name === "AbortError" || error.name === "NotAllowedError")) return;
      if (element.current === media) setFailed(true);
    }
  }

  function seek(seconds: number) {
    const media = element.current;
    if (!media || !duration) return;
    const next = Math.min(duration, Math.max(0, seconds));
    media.currentTime = next;
    setCurrentTime(next);
  }

  function changeVolume(value: number) {
    const media = element.current;
    if (!media) return;
    media.volume = value;
    media.muted = false;
  }

  function toggleMute() {
    const media = element.current;
    if (!media) return;
    if (media.volume === 0) changeVolume(1);
    else media.muted = !media.muted;
  }

  const updateDuration = (event: MediaEvent) => setDuration(finiteDuration(event.currentTarget));
  const mediaProps = {
    ref: attach,
    onLoadedMetadata: updateDuration,
    onDurationChange: updateDuration,
    onTimeUpdate: (event: MediaEvent) => setCurrentTime(event.currentTarget.currentTime),
    onPlay: () => setPlaying(true),
    onPause: () => setPlaying(false),
    onEnded: () => {
      setPlaying(false);
      setCurrentTime(0);
    },
    onError: () => {
      setPlaying(false);
      setFailed(true);
    },
    onVolumeChange: (event: MediaEvent) => {
      setVolume(event.currentTarget.volume);
      setMuted(event.currentTarget.muted);
    },
  };

  return { mediaProps, playing, currentTime, duration, failed, volume: muted ? 0 : volume, toggle, seek, changeVolume, toggleMute };
}

export type MediaPlayback = ReturnType<typeof useMediaPlayback>;
