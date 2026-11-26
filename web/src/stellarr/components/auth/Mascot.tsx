import { MascotImage, type StickerName } from "@/components/primitives/MascotArt";

/**
 * The moment each account screen stands for, with the mascot art that shows
 * it: a scene beside the form on a wide screen, a sticker above the heading
 * on a narrow one. The files are the brand's own, exported to WebP by
 * the StellarrBranding export script into `public/mascot/`; the components that
 * draw them are shared with the rest of the app.
 */
export type AuthScene =
  | "login"
  | "second-step"
  | "invite"
  | "register"
  | "password"
  | "setup-owner"
  | "setup-server"
  | "setup-invite"
  /** A provider confirmed someone no account here is linked to. */
  | "unlinked";

/**
 * An outcome page shows a sticker at every width, and no scene: something
 * worked, failed in a way that can be fixed, the server did not answer, or
 * a mailed link waits for a press.
 */
export type OutcomeSticker = "success" | "failure" | "offline" | "mail";

const SCENES: Record<AuthScene, { scene: string; sticker: StickerName }> = {
  login: { scene: "69-late-night-chat", sticker: "01-welcome" },
  "second-step": { scene: "66-cockpit-observation", sticker: "41-avatar-online" },
  invite: { scene: "68-zero-g-game-night", sticker: "05-invite-friends" },
  register: { scene: "70-launch-party", sticker: "38-high-five" },
  password: { scene: "67-signal-rescue", sticker: "33-finding-message" },
  "setup-owner": { scene: "65-home-server-workshop", sticker: "25-self-hosting" },
  "setup-server": { scene: "61-launch-mission", sticker: "04-creating-channel" },
  "setup-invite": { scene: "64-shared-cinema", sticker: "30-release" },
  unlinked: { scene: "63-translation-orbit", sticker: "27-connecting" },
};

const OUTCOMES: Record<OutcomeSticker, StickerName> = {
  success: "30-release",
  failure: "27-connecting",
  offline: "28-offline",
  mail: "33-finding-message",
};

/** The size the scenes are drawn at; the files hold this and twice this. */
const SCENE_SIZE = { width: 400, height: 600 };

export function sceneArt(scene: AuthScene): string {
  return SCENES[scene].scene;
}

export function stickerArt(from: { scene: AuthScene } | { outcome: OutcomeSticker }): StickerName {
  return "scene" in from ? SCENES[from.scene].sticker : OUTCOMES[from.outcome];
}

export function SceneImage({ name }: { name: string }) {
  return <MascotImage key={name} name={name} file={SCENE_SIZE.width} {...SCENE_SIZE} rise />;
}
