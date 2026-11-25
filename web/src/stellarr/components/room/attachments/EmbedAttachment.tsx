import type { KeyboardEvent } from "react";

import { useMediaQuery } from "@/hooks/useMediaQuery";
import type { EmbedPart } from "@/lib/messages/parts";

export interface EmbedAttachmentProps {
  part: EmbedPart;
  onOpen?(part: EmbedPart): void;
}

function PreviewAnimation({ url }: { url: string }) {
  const reduceMotion = useMediaQuery("(prefers-reduced-motion: reduce)", true);
  return (
    <video className="emb-thumb" src={url} aria-hidden="true"
      autoPlay={!reduceMotion} muted loop playsInline preload="metadata" />
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
        <PreviewAnimation url={part.animationUrl} />
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
