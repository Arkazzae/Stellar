import type { InterfaceLocale } from "@/i18n";

import type { TextEdit } from "./composerText";
import { GROUP_MENTION_WORDS, GROUP_MENTIONS, groupWordFor, type GroupMention } from "./groupMentions";

export interface MentionCandidate {
  id: string;
  /** What is written after the `@`: a username, or a group's word. */
  username: string;
  /** A member's name, or what calling on a group does. */
  displayName: string;
  avatarUrl?: string;
  /** Set on `@everyone` and `@here`, which call on a channel rather than a member. */
  group?: GroupMention;
}

/** The `@name` being typed at the caret. */
export interface ActiveMention {
  start: number;
  end: number;
  query: string;
  /** Changes whenever the mention does; lets Escape dismiss exactly this one. */
  key: string;
}

const MAX_SUGGESTIONS = 8;

/** An `@` that opens a word, followed by the characters a username can hold. */
const MENTION_BEFORE_CARET = /(?:^|\s)@([\p{L}\p{N}_.-]{0,32})$/u;

export function activeMention(text: string, caret: number): ActiveMention | null {
  const match = MENTION_BEFORE_CARET.exec(text.slice(0, caret));
  if (match === null) return null;
  const query = match[1] ?? "";
  const start = caret - query.length - 1;
  return { start, end: caret, query, key: `${start}:${query}` };
}

/**
 * The entries for calling on the whole channel, one per group, each writing the
 * group's word in the reader's language. Both languages' words work whichever
 * is written; offering one keeps the list short.
 */
export function groupMentionCandidates(
  locale: InterfaceLocale,
  describe: (group: GroupMention) => string,
): MentionCandidate[] {
  return GROUP_MENTIONS.map((group) => ({
    id: `group:${group}`,
    username: groupWordFor(group, locale),
    displayName: describe(group),
    group,
  }));
}

/**
 * Groups whose word, in either language, starts with the query come first;
 * then usernames that start with it, then display-name matches.
 */
export function matchMentions(
  candidates: readonly MentionCandidate[],
  query: string,
  locale: InterfaceLocale,
  groups: readonly MentionCandidate[] = [],
): MentionCandidate[] {
  const needle = query.toLocaleLowerCase();
  const startsWith = (candidate: MentionCandidate) =>
    candidate.username.toLocaleLowerCase().startsWith(needle);
  const groupMatches = groups.filter(
    (candidate) =>
      candidate.group !== undefined &&
      GROUP_MENTION_WORDS[candidate.group].some((word) => word.startsWith(query.toLowerCase())),
  );

  const members = candidates
    .filter(
      (candidate) =>
        startsWith(candidate) || candidate.displayName.toLocaleLowerCase().includes(needle),
    )
    .sort(
      (left, right) =>
        Number(startsWith(right)) - Number(startsWith(left)) ||
        left.username.localeCompare(right.username, locale),
    );
  return [...groupMatches, ...members].slice(0, MAX_SUGGESTIONS);
}

/** Replaces the typed `@query` with `@username`, followed by a space if none follows. */
export function insertMention(
  text: string,
  mention: ActiveMention,
  username: string,
): TextEdit {
  const before = text.slice(0, mention.start);
  const after = text.slice(mention.end);
  const trail = /^\s/.test(after) ? "" : " ";
  const inserted = `@${username}${trail}`;
  const caret = before.length + inserted.length;
  return { text: `${before}${inserted}${after}`, selectionStart: caret, selectionEnd: caret };
}
