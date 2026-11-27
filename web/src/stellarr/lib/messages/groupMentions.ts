/**
 * The words that call on a whole channel: `@everyone` or `@wszyscy` for
 * everyone who sees it, `@here` or `@obecni` for those of them online.
 *
 * The server reads them as it reads names and stamps a message with the groups
 * its author may call on there. Anybody can type the word, so the stamp alone
 * makes it a call: without it the word is plain text that highlights, rings and
 * pops up for nobody. Both languages' words call the same group whatever
 * language the author or the reader uses.
 */

import type { InterfaceLocale } from "@/i18n";
import type { Message } from "@/ports";

export type GroupMention = "everyone" | "here";

/** Each group's words, lower-cased, the English one first. */
export const GROUP_MENTION_WORDS: Readonly<Record<GroupMention, readonly string[]>> = {
  everyone: ["everyone", "wszyscy"],
  here: ["here", "obecni"],
};

/** In the order the composer offers them: the wider call first. */
export const GROUP_MENTIONS: readonly GroupMention[] = ["everyone", "here"];

/**
 * The group a typed word calls on, in any case. Lower-casing ignores the
 * reader's locale, so a Turkish one does not turn the I of OBECNI dotless.
 */
export function groupOfWord(word: string): GroupMention | undefined {
  const lower = word.toLowerCase();
  return GROUP_MENTIONS.find((group) => GROUP_MENTION_WORDS[group].includes(lower));
}

/**
 * Whether a username is one of the words. Such a name is nobody's: it cannot be
 * registered, and an account that took it before it was reserved is not called
 * by it, since the group meaning wins.
 */
export function isGroupWord(word: string): boolean {
  return groupOfWord(word) !== undefined;
}

/** The groups the server stamped a message as calling on. */
export function stampedGroups(
  message: Pick<Message, "mentionsEveryone" | "mentionsHere">,
): ReadonlySet<GroupMention> {
  const groups = new Set<GroupMention>();
  if (message.mentionsEveryone === true) groups.add("everyone");
  if (message.mentionsHere === true) groups.add("here");
  return groups;
}

/** The word the composer writes for a group, in the reader's language. */
export function groupWordFor(group: GroupMention, locale: InterfaceLocale): string {
  const [english, polish] = GROUP_MENTION_WORDS[group];
  return (locale === "pl" ? polish : english) ?? group;
}
