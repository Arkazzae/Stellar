import { useCallback, useState } from "react";

import styles from "./AuthFrame.module.css";

/**
 * The moment each account screen stands for, with the mascot art that shows
 * it: a scene beside the form on a wide screen, a sticker above the heading
 * on a narrow one. The files are the brand's own, exported to WebP by
 * `scripts/export-mascot.mjs` into `public/mascot/`.
 */
export type AuthScene =
  | "login"
  | "second-step"
  | "invite"
  | "register"
  | "password"
  | "setup-owner"
  | "setup-server"
  | "setup-invite";

/**
 * An outcome page shows a sticker at every width, and no scene: something
 * worked, failed in a way that can be fixed, the server did not answer, or
 * a mailed link waits for a press.
 */
export type OutcomeSticker = "success" | "failure" | "offline" | "mail";

const SCENES: Record<AuthScene, { scene: string; sticker: string }> = {
  login: { scene: "69-late-night-chat", sticker: "01-welcome" },
  "second-step": { scene: "66-cockpit-observation", sticker: "41-avatar-online" },
  invite: { scene: "68-zero-g-game-night", sticker: "05-invite-friends" },
  register: { scene: "70-launch-party", sticker: "38-high-five" },
  password: { scene: "67-signal-rescue", sticker: "33-finding-message" },
  "setup-owner": { scene: "65-home-server-workshop", sticker: "25-self-hosting" },
  "setup-server": { scene: "61-launch-mission", sticker: "04-creating-channel" },
  "setup-invite": { scene: "64-shared-cinema", sticker: "30-release" },
};

const OUTCOMES: Record<OutcomeSticker, string> = {
  success: "30-release",
  failure: "27-connecting",
  offline: "28-offline",
  mail: "33-finding-message",
};

/** The sizes the art is drawn at; the files hold these and twice these. */
const SCENE_SIZE = { width: 400, height: 600 };
const STICKER_SIZE = 104;

export function sceneArt(scene: AuthScene): string {
  return SCENES[scene].scene;
}

export function stickerArt(from: { scene: AuthScene } | { outcome: OutcomeSticker }): string {
  return "scene" in from ? SCENES[from.scene].sticker : OUTCOMES[from.outcome];
}

/**
 * One piece of the art. It is decoration, so it has no text, and the form
 * never waits for it: no preload, a low priority, decoded off the main
 * thread, and shown only once it has loaded. One that fails stays hidden.
 */
function MascotImage({
  name,
  width,
  height,
  className,
}: {
  name: string;
  width: number;
  height: number;
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
      className={[styles.mascot, shown ? styles.shown : null, className].filter(Boolean).join(" ")}
      src={`/mascot/${name}-${width}.webp`}
      srcSet={`/mascot/${name}-${width}.webp 1x, /mascot/${name}-${width * 2}.webp 2x`}
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

export function SceneImage({ name }: { name: string }) {
  return <MascotImage key={name} name={name} {...SCENE_SIZE} />;
}

export function StickerImage({ name }: { name: string }) {
  return (
    <MascotImage
      key={name}
      name={name}
      width={STICKER_SIZE}
      height={STICKER_SIZE}
      className={styles.sticker}
    />
  );
}
