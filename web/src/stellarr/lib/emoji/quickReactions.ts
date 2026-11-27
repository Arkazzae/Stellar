/**
 * The few reactions a message's toolbar offers in one press: the emoji this
 * device picked last, the same recents the picker shows first, filled up with
 * a common three until there are enough of them.
 */

import { useLayoutEffect, useMemo, useSyncExternalStore } from "react";

import type { CustomEmoji } from "@/ports";

import {
  EMOJI,
  parseRecentEmoji,
  pushRecentEmoji,
  recentEmojiSnapshot,
  subscribeRecentEmoji,
  type EmojiEntry,
} from "./emoji";

export const QUICK_REACTION_COUNT = 3;

/** Thumbs up, heart and tears of joy: what a row with no history yet offers. */
export const DEFAULT_QUICK_REACTIONS: readonly string[] = ["👍", "❤️", "😂"];

export interface QuickReaction {
  /** What the reaction is stored as: the character, or `:shortcode:` for a server emoji. */
  reaction: string;
  /** A server emoji's picture; a built-in one is drawn as its character. */
  imageUrl?: string;
}

const bare = (shortcode: string) => shortcode.replace(/^:|:$/g, "");

/** Without the emoji presentation selector, which some senders leave off "❤". */
const plain = (glyph: string) => glyph.replaceAll("️", "");

let tables: { byCode: Map<string, EmojiEntry>; byGlyph: Map<string, EmojiEntry> } | null = null;

/** Built on first use: the toolbar is the only reader, and it may never hang. */
function lookup() {
  if (tables === null) {
    const byCode = new Map<string, EmojiEntry>();
    const byGlyph = new Map<string, EmojiEntry>();
    for (const entry of EMOJI) {
      byCode.set(entry.shortcode, entry);
      byGlyph.set(entry.glyph, entry);
      if (!byGlyph.has(plain(entry.glyph))) byGlyph.set(plain(entry.glyph), entry);
    }
    tables = { byCode, byGlyph };
  }
  return tables;
}

/**
 * A recent shortcode as a reaction, or null when nothing answers to it any
 * more. A built-in emoji comes first, as it does in the picker's own recents;
 * a server's only while the server still has it.
 */
function resolve(code: string, customEmoji: readonly CustomEmoji[]): QuickReaction | null {
  const builtIn = lookup().byCode.get(code);
  if (builtIn !== undefined) return { reaction: builtIn.glyph };
  const custom = customEmoji.find((item) => bare(item.shortcode) === code);
  return custom === undefined ? null : { reaction: `:${code}:`, imageUrl: custom.glyph };
}

/** The row for these recents, newest first, each reaction once. */
export function quickReactionsFor(
  recents: readonly string[],
  customEmoji: readonly CustomEmoji[],
): QuickReaction[] {
  const row: QuickReaction[] = [];
  const add = (item: QuickReaction | null) => {
    if (item === null || row.length >= QUICK_REACTION_COUNT) return;
    if (row.some(({ reaction }) => reaction === item.reaction)) return;
    row.push(item);
  };
  for (const code of recents) add(resolve(code, customEmoji));
  for (const reaction of DEFAULT_QUICK_REACTIONS) add({ reaction });
  return row;
}

/**
 * The next row in the places the last one had, when it holds the same emoji.
 *
 * Reacting from the row moves that emoji to the front of the recents. Drawn in
 * that order, the next emoji would slide under the pointer, and a second press
 * meant to take the reaction back would add a different one. So the order
 * changes only when an emoji joins or leaves the row, as a pick from the
 * picker makes it, and then it is newest first again.
 */
export function keepPlaces(
  next: readonly QuickReaction[],
  previous: readonly QuickReaction[],
): readonly QuickReaction[] {
  const same =
    next.length === previous.length &&
    previous.every(({ reaction }) => next.some((item) => item.reaction === reaction));
  if (!same) return next;
  return previous.map(({ reaction }) => next.find((item) => item.reaction === reaction)!);
}

/**
 * The row as the toolbars last drew it. One for the page rather than one per
 * toolbar, so every message offers the same emoji in the same places.
 */
const lastRow: { current: readonly QuickReaction[] } = { current: [] };

/** Starts the next row newest first, as on a fresh page; for tests. */
export function forgetQuickReactionOrder(): void {
  lastRow.current = [];
}

/**
 * The quick reactions, kept current with the recents. It reads storage rather
 * than context, so a toolbar drawn by a root of its own can use it.
 */
export function useQuickReactions(customEmoji: readonly CustomEmoji[]): readonly QuickReaction[] {
  const stored = useSyncExternalStore(subscribeRecentEmoji, recentEmojiSnapshot);
  const row = useMemo(
    () => keepPlaces(quickReactionsFor(parseRecentEmoji(stored), customEmoji), lastRow.current),
    [stored, customEmoji],
  );
  useLayoutEffect(() => {
    lastRow.current = row;
  }, [row]);
  return row;
}

/** The recents' name for a reaction, or null for an emoji the picker does not have. */
function shortcodeOf(reaction: string): string | null {
  const custom = /^:([^:\s]+):$/.exec(reaction);
  if (custom !== null) return custom[1] ?? null;
  const { byGlyph } = lookup();
  return (byGlyph.get(reaction) ?? byGlyph.get(plain(reaction)))?.shortcode ?? null;
}

/**
 * Counts a reaction just added as a recent, wherever it was added from. One
 * the picker does not offer has no shortcode to keep, and is left out.
 */
export function rememberReaction(reaction: string): void {
  const code = shortcodeOf(reaction);
  if (code !== null) pushRecentEmoji(code);
}
