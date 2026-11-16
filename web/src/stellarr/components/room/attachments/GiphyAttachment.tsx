import { useEffect, useRef, useState } from "react";

import { useGifsEnabled, useGiphyAccess } from "@/hooks/useGifsEnabled";
import {
  fetchGiphyGif,
  GIPHY_REFERRER_POLICY,
  resolveGiphyApiKey,
  trackGiphyAction,
  type GiphyGif,
} from "@/lib/giphy/giphy";
import { useLanguage } from "@/state/LanguageProvider";

import type { GalleryImage, GiphyPart } from "@/lib/messages/parts";

export interface GiphyAttachmentProps {
  part: GiphyPart;
  onOpen?(images: GalleryImage[], index: number): void;
}

/** What was learned about one GIF id; `gif` is null when it could not be had. */
interface Outcome {
  id: string;
  gif: GiphyGif | null;
}

const REL = "ugc nofollow noopener noreferrer";

/**
 * A GIF from GIPHY, loaded by this browser straight from GIPHY.
 *
 * It is the one place a GIF in a message reaches GIPHY, so it reads the
 * person's switch itself. Off, it asks GIPHY nothing: it names the GIF, links
 * to its page and offers to load this one GIF. While the account's copy of the
 * switch is still on its way it waits, as if loading.
 */
export function GiphyAttachment({ part, onOpen }: GiphyAttachmentProps) {
  const { t } = useLanguage();
  const access = useGiphyAccess();
  const offered = useGifsEnabled();
  const [outcome, setOutcome] = useState<Outcome | null>(null);
  // The GIF the person loaded by hand while the switch is off.
  const [loadedByHand, setLoadedByHand] = useState<string | null>(null);
  const viewReported = useRef<string | null>(null);

  const load = access === "on" || loadedByHand === part.id;

  useEffect(() => {
    if (!load) return;
    const controller = new AbortController();
    resolveGiphyApiKey()
      .then((key) => fetchGiphyGif(key, part.id, controller.signal))
      .then((gif) => setOutcome({ id: part.id, gif }))
      .catch(() => {
        if (!controller.signal.aborted) setOutcome({ id: part.id, gif: null });
      });
    return () => controller.abort();
  }, [load, part.id]);

  if (access === "off" && !load) {
    return (
      <div className="att att-giphy-fallback">
        <span>{t("attachment.giphyHeld")}</span>
        <span className="att-giphy-actions">
          {offered ? (
            <button type="button" onClick={() => setLoadedByHand(part.id)}>
              {t("attachment.giphyShow")}
            </button>
          ) : null}
          <a href={part.pageUrl} target="_blank" rel={REL}>
            {t("attachment.openGiphy")}
          </a>
        </span>
      </div>
    );
  }

  // An outcome for a previous id says nothing about this one.
  const current = load && outcome?.id === part.id ? outcome : null;

  if (current !== null && current.gif === null) {
    return (
      <div className="att att-giphy-fallback">
        <span>{t("attachment.gifUnavailable")}</span>
        <a href={part.pageUrl} target="_blank" rel={REL}>
          {t("attachment.openGiphy")}
        </a>
      </div>
    );
  }

  if (current === null || current.gif === null) {
    // The GIF's size is GIPHY's to tell, so the box is a guess until it
    // answers. The attribution line is not: it comes with every GIF, so it
    // is held too, and a 4:3 GIF arrives without moving anything.
    return (
      <div className="att att-giphy" role="status">
        <div className="att-giphy-loading" />
        <div className="att-giphy-caption" aria-hidden="true">
          {" "}
        </div>
        <span className="sr-only">{t("attachment.loadingGif")}</span>
      </div>
    );
  }

  const { gif } = current;
  const alt = gif.alt ?? t("gif.untitled");
  const media = (
    <picture>
      {gif.stillUrl === undefined ? null : (
        <source media="(prefers-reduced-motion: reduce)" srcSet={gif.stillUrl} />
      )}
      {/* The image's referrer policy covers the <source> chosen in its place. */}
      <img
        src={gif.mediaUrl}
        alt={alt}
        width={gif.width}
        height={gif.height}
        loading="lazy"
        referrerPolicy={GIPHY_REFERRER_POLICY}
        onError={() => setOutcome({ id: part.id, gif: null })}
        onLoad={() => {
          // GIPHY counts one view per GIF, not one per load event.
          if (viewReported.current === gif.id) return;
          viewReported.current = gif.id;
          void trackGiphyAction(gif.analytics.onload);
        }}
      />
    </picture>
  );

  return (
    <figure className="att att-giphy">
      {onOpen === undefined ? (
        media
      ) : (
        <button
          type="button"
          className="att-giphy-media"
          aria-label={t("attachment.zoomGif", { alt })}
          onClick={() => onOpen([{ src: gif.mediaUrl, alt }], 0)}
        >
          {media}
        </button>
      )}
      <figcaption>
        <a href={gif.pageUrl} target="_blank" rel={REL}>
          {gif.username === undefined ? "GIPHY" : t("attachment.giphyBy", { name: gif.username })}
        </a>
      </figcaption>
    </figure>
  );
}
