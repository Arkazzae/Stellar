import { useCallback, useState } from "react";

import styles from "./MascotArt.module.css";

/**
 * The stickers `scripts/export-mascot.mjs` exports from the brand files
 * (`branding/mascot/stickers/`) into `public/mascot/`, at 104 and 208 pixels.
 * Keep the two lists in step.
 */
export type StickerName =
  | "01-welcome"
  | "02-first-message"
  | "03-empty-channel"
  | "04-creating-channel"
  | "05-invite-friends"
  | "25-self-hosting"
  | "27-connecting"
  | "28-offline"
  | "29-away"
  | "30-release"
  | "33-finding-message"
  | "38-high-five"
  | "41-avatar-online"
  | "42-do-not-disturb"
  | "51-quiet-night"
  | "55-moderation";

/** The width of the files every sticker is exported at, and twice that for dense screens. */
const STICKER_FILE = 104;

/**
 * One piece of the mascot art. It is decoration, so it has no text, and the
 * page never waits for it: no preload, a low priority, decoded off the main
 * thread, and shown only once it has loaded. One that fails stays hidden.
 *
 * `file` is the width the file was exported at; `width` and `height` are the
 * size it is laid out at, which the stylesheet may set smaller. `rise` lets
 * it rise into place as it arrives, as on the account screens; anywhere else
 * it simply appears.
 */
export function MascotImage({
  name,
  file,
  width,
  height,
  rise = false,
  className,
}: {
  name: string;
  file: number;
  width: number;
  height: number;
  rise?: boolean;
  className?: string;
}) {
  const [shown, setShown] = useState(false);
  // An image the browser already holds can be complete before React has
  // attached `onLoad`, and that event then never comes.
  const ref = useCallback((image: HTMLImageElement | null) => {
    if (image !== null && image.complete && image.naturalWidth > 0) setShown(true);
  }, []);

  return (
    <img
      ref={ref}
      className={[styles.art, rise ? styles.rise : null, shown ? styles.shown : null, className]
        .filter(Boolean)
        .join(" ")}
      src={`/mascot/${name}-${file}.webp`}
      srcSet={`/mascot/${name}-${file}.webp 1x, /mascot/${name}-${file * 2}.webp 2x`}
      width={width}
      height={height}
      alt=""
      decoding="async"
      fetchPriority="low"
      draggable={false}
      onLoad={() => setShown(true)}
    />
  );
}

/**
 * The sizes a sticker is drawn at: `column` in a narrow column (the rail, a
 * side panel), `main` in the main area and in dialogs, `page` above the
 * heading of an account screen.
 */
export type StickerSize = "column" | "main" | "page";

const STICKER_PIXELS: Record<StickerSize, number> = { column: 64, main: 96, page: 104 };

export interface StickerProps {
  name: StickerName;
  size: StickerSize;
  /**
   * `start` lines the art up with the left edge of the text below it, the
   * way a heading starts; `center` sits it over centred text.
   */
  align?: "start" | "center";
  /** Rises into place as it loads, as the account screens' art does. */
  rise?: boolean;
}

/**
 * The mascot marking a moment: an empty list, a first step, an outcome. The
 * text beside it says what the moment is, so the art is decoration and has
 * no text of its own.
 */
export function Sticker({ name, size, align = "start", rise = false }: StickerProps) {
  const pixels = STICKER_PIXELS[size];
  return (
    <MascotImage
      key={name}
      name={name}
      file={STICKER_FILE}
      width={pixels}
      height={pixels}
      rise={rise}
      className={[styles.sticker, styles[size], align === "start" ? styles.start : null]
        .filter(Boolean)
        .join(" ")}
    />
  );
}
