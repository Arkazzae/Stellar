/**
 * The emoji the picker offers, and the rules for finding one.
 *
 * The set is curated rather than the full Unicode table: what people reach for,
 * each with Polish keywords, so that searching "smutny" or "kciuk" works in the
 * language the rest of the app is written in. The seed tables live in
 * `emojiTable.ts`; this module indexes them and holds the search and the
 * recents list. It depends on nothing, so components may import it directly.
 */

import {
  ACTIVITY,
  FLAGS,
  FOOD,
  NATURE,
  OBJECTS,
  PEOPLE,
  SMILEYS,
  SYMBOLS,
  TRAVEL,
  type Seed,
} from "./emojiTable";

export type EmojiCategoryId =
  | "recent"
  | "server"
  | "smileys"
  | "people"
  | "nature"
  | "food"
  | "activity"
  | "travel"
  | "objects"
  | "symbols"
  | "flags";

export interface EmojiCategory {
  id: EmojiCategoryId;
  /** What the rail shows. A glyph, because an icon set has no "🐾". */
  mark: string;
}

export interface EmojiEntry {
  /** The character itself, or a URL when this is a server's own emoji. */
  glyph: string;
  /** Without colons; the footer and the composer add them. */
  shortcode: string;
  category: EmojiCategoryId;
  /** Space-separated, lower case, already stripped of diacritics for search. */
  terms: string;
  /** Set for a server's own emoji, whose `glyph` is a URL rather than a char. */
  imageUrl?: string;
}

/**
 * The rail, top to bottom. "recent" and "server" have no table: the picker
 * fills them from this device and from the server, and they are listed here so
 * the rail and the scroller agree on the order.
 */
export const EMOJI_CATEGORIES: readonly EmojiCategory[] = [
  { id: "recent", mark: "🕘" },
  { id: "server", mark: "🧩" },
  { id: "smileys", mark: "😀" },
  { id: "people", mark: "👋" },
  { id: "nature", mark: "🐾" },
  { id: "food", mark: "🍔" },
  { id: "activity", mark: "⚽" },
  { id: "travel", mark: "✈️" },
  { id: "objects", mark: "💡" },
  { id: "symbols", mark: "💠" },
  { id: "flags", mark: "🏳️" },
];

const TABLES: readonly (readonly [EmojiCategoryId, readonly Seed[]])[] = [
  ["smileys", SMILEYS],
  ["people", PEOPLE],
  ["nature", NATURE],
  ["food", FOOD],
  ["activity", ACTIVITY],
  ["travel", TRAVEL],
  ["objects", OBJECTS],
  ["symbols", SYMBOLS],
  ["flags", FLAGS],
];

/** Every built-in emoji, in rail order. The picker puts server emoji before them. */
export const EMOJI: readonly EmojiEntry[] = TABLES.flatMap(([category, seeds]) =>
  seeds.map(([glyph, shortcode, keywords]) => ({
    glyph,
    shortcode,
    category,
    // Searchable by shortcode too, so ":kciuk" finds the thumb without a
    // second index.
    terms: `${keywords} ${shortcode.replaceAll("_", " ")}`,
  })),
);

/**
 * Reduces a query to the form `terms` are written in: no case, no colons, no
 * diacritics. "zaba" and ":zaba:" are the same search with or without the
 * Polish letters, because Polish is typed both ways depending on the keyboard.
 */
export function foldQuery(raw: string): string {
  return (
    raw
      .trim()
      .toLowerCase()
      .replace(/^:+|:+$/g, "")
      .normalize("NFD")
      .replace(/[̀-ͯ]/g, "")
      // The Polish l with a stroke is a letter of its own, not an l plus a
      // mark, so NFD leaves it.
      .replaceAll("ł", "l")
  );
}

/**
 * Entries matching a query, best first: the exact shortcode, then a term that
 * starts with the query, then a term word that starts with it, then any
 * substring. Within a rank the order of `entries` is kept.
 */
export function searchEmoji(
  entries: readonly EmojiEntry[],
  query: string,
  limit = 120,
): EmojiEntry[] {
  const needle = foldQuery(query);
  if (needle.length === 0) return [];

  const ranks: EmojiEntry[][] = [[], [], [], []];
  for (const entry of entries) {
    const at = entry.terms.indexOf(needle);
    if (at < 0) continue;
    if (entry.shortcode === needle) ranks[0]!.push(entry);
    else if (at === 0) ranks[1]!.push(entry);
    else if (entry.terms[at - 1] === " ") ranks[2]!.push(entry);
    else ranks[3]!.push(entry);
  }
  return ranks.flat().slice(0, limit);
}

const RECENT_KEY = "stellarr.emoji.recent";
const RECENT_MAX = 32;

/** Storage can be absent or throw (blocked cookies); recents are optional. */
function deviceStorage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

/** Shortcodes, most recent first. The caller drops codes it no longer knows. */
export function readRecentEmoji(): string[] {
  return parseRecentEmoji(recentEmojiSnapshot());
}

/** The list a `recentEmojiSnapshot` holds; anything unreadable is no list. */
export function parseRecentEmoji(raw: string): string[] {
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed.filter((code): code is string => typeof code === "string")
      : [];
  } catch {
    return [];
  }
}

/**
 * The recents as stored, unparsed. Two equal strings are the same list, so a
 * component that keeps this as its snapshot draws again only when the list
 * really changed.
 */
export function recentEmojiSnapshot(): string {
  try {
    return deviceStorage()?.getItem(RECENT_KEY) ?? "";
  } catch {
    return "";
  }
}

const recentListeners = new Set<() => void>();

/** Calls `listener` whenever the recents change, in this tab or in another. */
export function subscribeRecentEmoji(listener: () => void): () => void {
  const fromAnotherTab = (event: StorageEvent) => {
    // A null key is the whole storage cleared.
    if (event.key === RECENT_KEY || event.key === null) listener();
  };
  recentListeners.add(listener);
  window.addEventListener("storage", fromAnotherTab);
  return () => {
    recentListeners.delete(listener);
    window.removeEventListener("storage", fromAnotherTab);
  };
}

/** Moves a shortcode to the front and returns the new list. */
export function pushRecentEmoji(shortcode: string): string[] {
  const next = [shortcode, ...readRecentEmoji().filter((code) => code !== shortcode)].slice(
    0,
    RECENT_MAX,
  );
  try {
    deviceStorage()?.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    // Quota or private mode: the list still holds for this session.
  }
  for (const listener of [...recentListeners]) listener();
  return next;
}
