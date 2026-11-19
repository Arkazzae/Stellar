/**
 * Exports the built-in identity art from the brand files to WebP: the mascot
 * faces (`branding/avatars/`) an account without an uploaded picture wears,
 * and the scenes (`branding/backgrounds/`) that stand in for its banner.
 *
 *   node scripts/export-identity-art.mjs <public-dir>
 *
 * writes `<public-dir>/identity/avatars/<name>.webp` and
 * `<public-dir>/identity/banners/<name>.webp`, one file each. It needs `cwebp`
 * on the PATH, and Python 3 with Pillow, which pads the faces. The output is
 * committed; run it again only when a brand file or `SCENE_FOCUS` changes,
 * and keep the names in step with `AVATARS` and `BANNERS` in
 * `src/stellarr/design/identity.ts`.
 */
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const outputRoot = process.argv[2];
if (outputRoot === undefined) {
  console.error("usage: node scripts/export-identity-art.mjs <public-dir>");
  process.exit(1);
}

const branding = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "branding");

/**
 * A face is drawn on a disc up to 128 px wide (the face of a private call),
 * so one 256 px file is sharp on a 2x screen everywhere and weighs about 5 KB.
 */
const AVATAR_SIZE = 256;

/**
 * How far out the widest face may reach, as a share of the disc's radius.
 * Ears, antennas and the comet's tail touch the edge of the brand canvas,
 * which the round crop would cut, so every face is shrunk by one common
 * factor until the one reaching furthest from the centre stays inside this.
 * One factor rather than one per face, so the visors are the same size on
 * every avatar.
 */
const AVATAR_REACH = 0.94;

/**
 * Banners are cut to the mini profile's strip, 320 by 88, at the source's
 * full width, and sized for the account pane's preview, which draws the same
 * file up to about 790 px wide.
 */
const BANNER_WIDTH = 1600;
const BANNER_HEIGHT = 440;

/**
 * The row (a share of the scene's height) each cut is centred on. The cut
 * takes the scene's full width, so only the row moves.
 *
 * The account preview is wider than the mini profile and shows only the
 * middle of the cut, so the row is the middle of the two mascots' heads, ears
 * to chin, moved only as far as it takes for both mascots to fit the cut
 * whole, ears to shadow. In 03, 08 and 11 one mascot floats above the other
 * and the pair is taller than the cut, so the row is the heads' middle and
 * the lower mascot's tail is trimmed.
 */
const SCENE_FOCUS = {
  "01-orbit-lounge": 0.554,
  "02-lava-valley": 0.551,
  "03-coral-reef": 0.443,
  "04-moonlit-forest": 0.522,
  "05-greenhouse": 0.518,
  "06-sky-island": 0.516,
  "07-sunset-dunes": 0.595,
  "08-space-arcade": 0.499,
  "09-ice-shelf": 0.686,
  "10-reading-nook": 0.572,
  "11-deep-sea": 0.455,
  "12-bamboo-grove": 0.542,
  "13-observatory": 0.671,
  "14-lily-pond": 0.58,
};

/**
 * Pads every face onto a transparent square large enough that, scaled back to
 * the avatar, nothing reaches past `AVATAR_REACH` of the radius. The reach is
 * measured on a reduced copy, which is plenty for a margin of a few pixels.
 */
const PAD_FACES = `
import sys
from PIL import Image

reach, out, sources = float(sys.argv[1]), sys.argv[2], sys.argv[3:]
faces = [Image.open(path).convert("RGBA") for path in sources]

def furthest(face):
    side = 256
    alpha = face.getchannel("A").resize((side, side))
    pixels = alpha.load()
    centre = (side - 1) / 2
    return max(
        ((x - centre) ** 2 + (y - centre) ** 2) ** 0.5
        for y in range(side) for x in range(side) if pixels[x, y] > 24
    ) / (side / 2)

worst = max(furthest(face) for face in faces)
for path, face in zip(sources, faces):
    side = round(face.width * worst / reach)
    canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    canvas.paste(face, ((side - face.width) // 2, (side - face.height) // 2))
    canvas.save(out + "/" + path.rsplit("/", 1)[-1])
print("%.3f" % worst)
`;

function brandFiles(folder) {
  return readdirSync(join(branding, folder))
    .filter((file) => /^\d{2}-[a-z-]+\.png$/.test(file))
    .sort()
    .map((file) => join(branding, folder, file));
}

/** The art is flat colour with dark outlines, which the drawing preset keeps clean. */
function webp(source, target, extra) {
  execFileSync("cwebp", [
    "-quiet",
    "-preset", "drawing",
    "-q", "70",
    "-m", "6",
    "-metadata", "none",
    ...extra,
    source,
    "-o", target,
  ]);
}

const avatars = join(outputRoot, "identity", "avatars");
const banners = join(outputRoot, "identity", "banners");
mkdirSync(avatars, { recursive: true });
mkdirSync(banners, { recursive: true });

const faces = brandFiles("avatars");
const padded = mkdtempSync(join(tmpdir(), "identity-faces-"));
try {
  const worst = execFileSync("python3", ["-c", PAD_FACES, String(AVATAR_REACH), padded, ...faces], {
    encoding: "utf8",
  }).trim();
  for (const face of faces) {
    const name = basename(face, ".png");
    webp(join(padded, `${name}.png`), join(avatars, `${name}.webp`), [
      "-alpha_q", "80",
      "-resize", String(AVATAR_SIZE), String(AVATAR_SIZE),
    ]);
  }
  console.log(`identity: ${faces.length} faces, the furthest reaching ${worst} of the canvas's half side`);
} finally {
  rmSync(padded, { recursive: true, force: true });
}

const scenes = brandFiles("backgrounds");
for (const scene of scenes) {
  const name = basename(scene, ".png");
  const focus = SCENE_FOCUS[name];
  if (focus === undefined) throw new Error(`${name}: no row in SCENE_FOCUS`);
  const [width, height] = execFileSync("python3", [
    "-c",
    "import sys; from PIL import Image; print(*Image.open(sys.argv[1]).size)",
    scene,
  ], { encoding: "utf8" }).trim().split(" ").map(Number);
  const cutHeight = Math.round((width * BANNER_HEIGHT) / BANNER_WIDTH);
  const top = Math.min(Math.max(Math.round(focus * height - cutHeight / 2), 0), height - cutHeight);
  webp(scene, join(banners, `${name}.webp`), [
    "-crop", "0", String(top), String(width), String(cutHeight),
    "-resize", String(BANNER_WIDTH), String(BANNER_HEIGHT),
  ]);
}
console.log(`identity: ${scenes.length} scenes cut to ${BANNER_WIDTH} by ${BANNER_HEIGHT}`);
