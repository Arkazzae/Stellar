import { useState } from "react";

import { Icon } from "@stellarr/icons";
import { useLanguage } from "@/state/LanguageProvider";
import type { AttachmentCallbacks, AudioPart, VideoPart } from "@/lib/messages/parts";

import { AudioAttachment } from "./AudioAttachment";
import { VideoAttachment } from "./VideoAttachment";

/** Private originals are fetched and decrypted only when the reader presses Play. */
export function DeferredMediaAttachment({ part, resolveOriginal }: {
  part: AudioPart | VideoPart;
  resolveOriginal: AttachmentCallbacks["resolveOriginal"];
}) {
  const { t } = useLanguage();
  const [src, setSrc] = useState<string>();
  const [status, setStatus] = useState<"idle" | "loading" | "failed">("idle");
  const video = part.kind === "video";
  const name = part.name ?? t("media.video");

  if (src) {
    return video
      ? <VideoAttachment part={{ ...part, url: src }} autoPlay />
      : <AudioAttachment part={{ ...part, url: src }} autoPlay />;
  }

  async function open() {
    if (!part.url || !resolveOriginal || status === "loading") return;
    setStatus("loading");
    try {
      const resolved = await resolveOriginal(part.url);
      if (resolved && resolved !== part.url) {
        setSrc(resolved);
        return;
      }
    } catch {
      // Keep the play button available for another attempt.
    }
    setStatus("failed");
  }

  return (
    <div className={video ? "att att-video deferred" : "att att-audio"}>
      {video && part.thumb && <img src={part.thumb} alt="" />}
      <button
        type="button"
        className={video ? "overlay-play" : "play-btn"}
        aria-label={t("attachment.play", { name })}
        disabled={!resolveOriginal || status === "loading"}
        onClick={() => void open()}
      >
        <span><Icon name="play" size={16} /></span>
      </button>
      {!video && <span className="au-meta"><b>{name}</b></span>}
      {status !== "idle" && (
        <span className={video ? "vdur" : "au-foot"} role="status">
          {t(status === "loading" ? "common.loading" : "attachment.playbackError")}
        </span>
      )}
    </div>
  );
}
