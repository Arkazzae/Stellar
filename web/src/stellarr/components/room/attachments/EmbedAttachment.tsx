import { useEffect, useRef, type KeyboardEvent } from "react";

import { useMediaQuery } from "@/hooks/useMediaQuery";
import type { EmbedPart } from "@/lib/messages/parts";

export interface EmbedAttachmentProps {
  part: EmbedPart;
  onOpen?(part: EmbedPart): void;
}

const previews = new Map<Element, (visible: boolean) => void>();
let observer: IntersectionObserver | undefined;

function observePreview(video: HTMLVideoElement, update: (visible: boolean) => void) {
  if (typeof IntersectionObserver === "undefined") {
    update(true);
    return;
  }
  observer ??= new IntersectionObserver((entries) => {
    for (const entry of entries) previews.get(entry.target)?.(entry.isIntersecting);
  });
  previews.set(video, update);
  observer.observe(video);
  return () => {
    previews.delete(video);
    observer?.unobserve(video);
    if (previews.size === 0) {
      observer?.disconnect();
      observer = undefined;
    }
  };
}

function PreviewAnimation({ url }: { url: string }) {
  const ref = useRef<HTMLVideoElement>(null);
  const reduceMotion = useMediaQuery("(prefers-reduced-motion: reduce)", true);
  useEffect(() => {
    const video = ref.current;
    if (!video) return;
    const stop = observePreview(video, (visible) => {
      if (visible && !video.hasAttribute("src")) video.src = url;
      if (visible && !reduceMotion) {
        void video.play().catch(() => {});
      } else {
        video.pause();
      }
    });
    return () => {
      stop?.();
      video.pause();
    };
  }, [url, reduceMotion]);

  return (
    <video ref={ref} className="emb-thumb" aria-hidden="true" muted loop playsInline preload="metadata" />
  );
}

export function EmbedAttachment({ part, onOpen }: EmbedAttachmentProps) {
  const open = onOpen && (() => onOpen(part));
  const layout = part.showFullImage ? " full-image" : part.thumb === undefined ? "" : " rich";

  // A link role comes with the keyboard contract of a link.
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (open === undefined || event.key !== "Enter") return;
    event.preventDefault();
    open();
  };

  return (
    <div
      className={`att att-embed${layout}`}
      role="link"
      tabIndex={0}
      // The card stands in for a URL no longer written out in the body, so it
      // has to say where it leads.
      aria-label={`${part.title} (${part.domain})`}
      onClick={open}
      onKeyDown={onKeyDown}
    >
      {/* The server copied the thumbnail when it unfurled the link, so opening
          a channel never calls out to whoever wrote the page. */}
      {part.animationUrl !== undefined ? (
        <PreviewAnimation key={part.animationUrl} url={part.animationUrl} />
      ) : part.thumb === undefined ? null : (
        <img className="emb-thumb" src={part.thumb} alt="" loading="lazy" />
      )}
      <div className="emb-text">
        <span className="dom">{part.domain}</span>
        <b>{part.title}</b>
        {part.description === "" ? null : <p>{part.description}</p>}
      </div>
    </div>
  );
}
