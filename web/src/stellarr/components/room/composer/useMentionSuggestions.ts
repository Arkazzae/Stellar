import { useMemo, useState } from "react";

import type { InterfaceLocale } from "@/i18n";

import {
  activeMention,
  matchMentions,
  type ActiveMention,
  type MentionCandidate,
} from "@/lib/messages/composerMentions";
import { isGroupWord } from "@/lib/messages/groupMentions";

const NO_GROUPS: readonly MentionCandidate[] = [];

interface Options {
  text: string;
  candidates: readonly MentionCandidate[];
  /**
   * `@everyone` and `@here`, offered before the members, for whoever may call
   * on the channel; without them the list offers members alone.
   */
  groups?: readonly MentionCandidate[];
  locale: InterfaceLocale;
  /** False while the draft is code, where an @ is just a character. */
  enabled: boolean;
  /**
   * The whole text is the name being looked up, with or without its "@", as
   * in a command's person argument, rather than an `@name` under the caret.
   */
  whole?: boolean;
}

/**
 * The suggestion list for the `@name` under the caret: which members match,
 * which one is highlighted, and whether Escape has put the list away.
 *
 * The caret is state here rather than read from the field, because the list
 * has to re-evaluate when the caret moves without the text changing.
 */
export function useMentionSuggestions({
  text,
  candidates,
  groups = NO_GROUPS,
  locale,
  enabled,
  whole = false,
}: Options) {
  const [caret, setCaret] = useState(0);
  const [index, setIndex] = useState(0);
  const [dismissedKey, setDismissedKey] = useState<string | null>(null);

  const mention: ActiveMention | null = !enabled
    ? null
    : whole
      ? { start: 0, end: text.length, query: text.replace(/^@/, "").trim(), key: `whole:${text}` }
      : activeMention(text, caret);
  const query = mention?.query;
  // In the text, writing a group's word calls on the group, so nobody who took
  // one as a username before it was reserved is offered under it there. A
  // command's person argument names an account, and may still pick them.
  const named = useMemo(
    () => (whole ? candidates : candidates.filter((candidate) => !isGroupWord(candidate.username))),
    [candidates, whole],
  );
  const matches = useMemo(
    () => (query === undefined ? [] : matchMentions(named, query, locale, groups)),
    [named, groups, query, locale],
  );
  const open = mention !== null && mention.key !== dismissedKey && matches.length > 0;

  return {
    /** The mention being typed, whether or not the list is showing. */
    mention,
    open,
    matches,
    selectedIndex: index,
    selected: matches[index] ?? matches[0],
    /** The caret moved or the text changed: start again from the first match. */
    track(position: number) {
      setCaret(position);
      setIndex(0);
      setDismissedKey(null);
    },
    step(direction: 1 | -1) {
      setIndex((current) => (current + direction + matches.length) % matches.length);
    },
    dismiss() {
      setDismissedKey(mention?.key ?? null);
    },
  };
}
