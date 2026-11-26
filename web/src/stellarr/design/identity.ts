import type { CSSProperties } from "react";

/**
 * Per-person colour and artwork.
 *
 * A stable hash of the user id is the only input, so a person looks the same in
 * the rail, the roster and the transcript and in every session. Two people can
 * still land on the same hue or the same mascot; an avatar is not an
 * identifier.
 */

/** FNV-1a: small, identical in every engine, and spreads short ids well. */
function stableHash(value: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < value.length; i++) {
    hash ^= value.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

/**
 * Twelve stops around the wheel rather than a continuous hue, so neighbouring
 * people read as clearly different instead of as two shades of one colour.
 */
const HUE_STOPS = 12;
const HUE_OFFSET = 277;

export function hueFor(id: string): number {
  return (HUE_OFFSET + (stableHash(id) % HUE_STOPS) * (360 / HUE_STOPS)) % 360;
}

/**
 * The mascot faces in `public/identity/avatars`, transparent, drawn on the
 * person's colour. Their source portraits live in StellarrBranding.
 */
const AVATARS = [
  "01-pixel-cat",
  "02-orbit-fox",
  "03-skip-rabbit",
  "04-honey-bear",
  "05-bao-panda",
  "06-radar-raccoon",
  "07-ember-red-panda",
  "08-koko-koala",
  "09-echo-frog",
  "10-nova-axolotl",
  "11-aster-owl",
  "12-nori-penguin",
  "13-noct-bat",
  "14-ocho-octopus",
  "15-lumi-jellyfish",
  "16-zeta-alien",
  "17-leaf-sprout",
  "18-comet",
  "19-nimbus-cloud",
  "20-draco-dragon",
] as const;

/**
 * The scenes in `public/identity/banners` are cropped to keep both mascots
 * visible in the mini profile. Their originals live in StellarrBranding.
 */
const BANNERS = [
  "01-orbit-lounge",
  "02-lava-valley",
  "03-coral-reef",
  "04-moonlit-forest",
  "05-greenhouse",
  "06-sky-island",
  "07-sunset-dunes",
  "08-space-arcade",
  "09-ice-shelf",
  "10-reading-nook",
  "11-deep-sea",
  "12-bamboo-grove",
  "13-observatory",
  "14-lily-pond",
] as const;

/**
 * Picks one of `names` for an id, apart from the hue and from every other
 * pick. A salt alone would not do it: FNV-1a's low bits follow only the low
 * bits of the input, so salted hashes of one id share their parity, and with
 * 20, 14 and 12 all even the picks would move together. Murmur3's finaliser
 * spreads every bit of the salted hash over the rest first.
 */
function pick<T>(names: readonly T[], salt: string, id: string): T {
  let hash = stableHash(`${salt}:${id}`);
  hash ^= hash >>> 16;
  hash = Math.imul(hash, 0x85ebca6b);
  hash ^= hash >>> 13;
  hash = Math.imul(hash, 0xc2b2ae35);
  hash ^= hash >>> 16;
  return names[(hash >>> 0) % names.length]!;
}

/**
 * The face and the banner for someone who has not uploaded their own. Nothing
 * is stored: the id picks both, each on its own, so every surface agrees
 * without a synthetic profile URL in the database.
 */
export function defaultIdentityArt(id: string): {
  avatarUrl: string;
  bannerUrl: string;
} {
  return {
    avatarUrl: `/identity/avatars/${pick(AVATARS, "avatar", id)}.webp`,
    bannerUrl: `/identity/banners/${pick(BANNERS, "banner", id)}.webp`,
  };
}

/**
 * Avatar fill, behind the mascot or the initials: dark enough that white
 * initials clear 4.5:1.
 */
export function avatarColor(id: string): string {
  return `oklch(0.58 0.17 ${hueFor(id)})`;
}

/** Author name in the transcript: light enough to clear 4.5:1 on the canvas. */
export function nameColor(id: string): string {
  return `oklch(0.79 0.14 ${hueFor(id)})`;
}

/** Someone who is offline loses their hue, not just their opacity. */
export const OFFLINE_AVATAR = "oklch(0.42 0.015 var(--h-neutral))";

/**
 * The custom properties a person-scoped element sets. Spread onto `style` so
 * nested rules can use the colour without it being passed down as props.
 */
export function identityVars(id: string, offline = false): CSSProperties {
  return {
    "--person-bg": offline ? OFFLINE_AVATAR : avatarColor(id),
    "--person-name": offline ? "var(--text-faint)" : nameColor(id),
  } as CSSProperties;
}
