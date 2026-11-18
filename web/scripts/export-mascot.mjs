/**
 * Exports the mascot art the app shows from the brand files
 * (`branding/mascot/`) to WebP, at the size each is drawn and twice that.
 *
 *   node scripts/export-mascot.mjs <public-dir>
 *
 * writes `<public-dir>/mascot/<name>-<width>.webp`. It needs `cwebp` on the
 * PATH. The output is committed; run it again only when the list below or a
 * brand file changes, and keep it in step with `StickerName` in
 * `src/stellarr/components/primitives/MascotArt.tsx` and the scenes in
 * `src/stellarr/components/auth/Mascot.tsx`.
 */
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const outputRoot = process.argv[2];
if (outputRoot === undefined) {
  console.error("usage: node scripts/export-mascot.mjs <public-dir>");
  process.exit(1);
}

const brand = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "branding", "mascot");

/** The scenes beside the form on a wide screen: 2:3, drawn 400 by 600. */
const ILLUSTRATIONS = [
  "61-launch-mission",
  "63-translation-orbit",
  "64-shared-cinema",
  "65-home-server-workshop",
  "66-cockpit-observation",
  "67-signal-rescue",
  "68-zero-g-game-night",
  "69-late-night-chat",
  "70-launch-party",
];

/**
 * The stickers: above the heading of an account screen, and over the empty
 * and first-time states in the main area of Friends and in the encryption
 * dialogs. Exported 104 square, the largest they are drawn; smaller ones
 * take the same files.
 */
const STICKERS = [
  "01-welcome",
  "04-creating-channel",
  "05-invite-friends",
  "10-reading",
  "25-self-hosting",
  "27-connecting",
  "28-offline",
  "30-release",
  "33-finding-message",
  "38-high-five",
  "41-avatar-online",
  "51-quiet-night",
  "55-moderation",
];

const directory = join(outputRoot, "mascot");
mkdirSync(directory, { recursive: true });

/**
 * The art is flat colour with dark outlines, which the drawing preset keeps
 * clean at a quality where a 2x scene weighs 50–100 KB and a sticker 5 KB.
 */
function exportImage(source, name, width, height) {
  execFileSync("cwebp", [
    "-quiet",
    "-preset", "drawing",
    "-q", "70",
    "-alpha_q", "80",
    "-m", "6",
    "-metadata", "none",
    "-resize", String(width), String(height),
    source,
    "-o", join(directory, `${name}-${width}.webp`),
  ]);
}

for (const name of ILLUSTRATIONS) {
  for (const scale of [1, 2]) exportImage(join(brand, "illustrations", `${name}.png`), name, 400 * scale, 600 * scale);
}
for (const name of STICKERS) {
  for (const scale of [1, 2]) exportImage(join(brand, "stickers", `${name}.png`), name, 104 * scale, 104 * scale);
}

console.log(`mascot: ${ILLUSTRATIONS.length} scenes and ${STICKERS.length} stickers, 1x and 2x`);
