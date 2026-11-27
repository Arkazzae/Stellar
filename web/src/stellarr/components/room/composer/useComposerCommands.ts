import { useState } from "react";

import type { MessageKey, Translator } from "@/i18n";
import { matchCommand, slashQuery, type CommandMatch } from "@/lib/messages/composerCommands";
import type { MentionCandidate } from "@/lib/messages/composerMentions";

import { COMMANDS, type CommandContext, type CommandInput, type ComposerCommand } from "./commandRegistry";

/** The draft a command leaves behind, and where the caret goes in it. */
export interface CommandBase {
  text: string;
  caret: number;
}

export interface CommandItem {
  command: ComposerCommand;
  /** The name in the reader's language, without the slash. */
  name: string;
  match: CommandMatch;
  /** Why it cannot run right now. It stays listed so the hint can say so. */
  blocked?: MessageKey;
}

/** A command being given its arguments, in the field in place of the draft. */
export interface ActiveCommand {
  command: ComposerCommand;
  name: string;
  /**
   * How it was reached. Going back from a typed one leaves "/name" in the
   * field to edit; from the plus, the draft as it was.
   */
  from: "plus" | "slash";
  base: CommandBase;
  text: Readonly<Record<string, string>>;
  people: Readonly<Record<string, MentionCandidate>>;
  /** The argument being filled in. */
  current: number;
}

export function commandInput(active: Pick<ActiveCommand, "text" | "people">): CommandInput {
  return {
    text: (id) => active.text[id] ?? "",
    person: (id) => active.people[id],
  };
}

interface Options {
  text: string;
  caret: number;
  /** False in code mode, where a slash starts a comment rather than a command. */
  enabled: boolean;
  /** What the commands can reach, for the draft they would leave behind. */
  contextFor(base: CommandBase): CommandContext;
  t: Translator;
}

/**
 * The command list over the composer and the command being filled in.
 *
 * The list opens from the plus, or by itself while a slash that opens the
 * draft is followed by the word under the caret. Escape puts a typed list
 * away until the draft changes. A list opened from the plus lasts until the
 * draft changes: typing is writing the message again.
 */
export function useComposerCommands({ text, caret, enabled, contextFor, t }: Options) {
  const [plus, setPlus] = useState<CommandBase | null>(null);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const [selection, setSelection] = useState({ key: "", index: 0 });
  const [active, setActive] = useState<ActiveCommand | null>(null);

  // Adjusted during render, so no frame shows a plus list over a draft that
  // has moved on, or one over code.
  if (plus !== null && (plus.text !== text || !enabled || active !== null)) setPlus(null);
  if (active !== null && !enabled) setActive(null);
  // Escape holds for the draft it was pressed on; typing back to it later is asking again.
  if (dismissed !== null && dismissed !== text) setDismissed(null);

  const typed = enabled && plus === null && active === null ? slashQuery(text, caret) : null;
  const slash = typed !== null && dismissed !== text ? typed : null;
  const mode: ActiveCommand["from"] | null = plus !== null ? "plus" : slash !== null ? "slash" : null;

  const base: CommandBase | null =
    plus ?? (slash === null ? null : { text: text.slice(slash.end).replace(/^\s/, ""), caret: 0 });
  const query = slash?.query ?? "";

  let items: CommandItem[] = [];
  if (base !== null) {
    const context = contextFor(base);
    for (const command of COMMANDS) {
      const availability = command.available(context);
      if (!availability.shown) continue;
      const name = t(command.name);
      const match = matchCommand(query, { name, alias: command.alias });
      if (match === null) continue;
      items.push({ command, name, match, ...(availability.blocked === undefined ? {} : { blocked: availability.blocked }) });
    }
    // Sorting is stable, so commands that answer equally keep the list's order.
    if (query !== "") items = items.sort((left, right) => left.match.rank - right.match.rank);
  }

  const key = `${mode ?? ""}:${query}`;
  const firstReady = Math.max(0, items.findIndex((item) => item.blocked === undefined));
  const selectedIndex =
    selection.key === key ? Math.min(selection.index, Math.max(0, items.length - 1)) : firstReady;
  const open = mode !== null && items.length > 0;

  return {
    mode,
    open,
    items,
    /** Unfiltered, the list is drawn in its groups; filtered, best match first. */
    grouped: query === "",
    query,
    /** The command word being typed, while the list follows it. */
    slash,
    /** The draft on which Escape put a typed list away, while it still applies. */
    dismissed: dismissed === text ? dismissed : null,
    selectedIndex,
    selected: open ? items[selectedIndex] : undefined,
    /** What the draft becomes once a command from the list is chosen. */
    base,
    active,
    openFromPlus(from: CommandBase) {
      setDismissed(null);
      setPlus(from);
    },
    /** Puts the list away; a typed one stays away until the draft changes. */
    close() {
      setPlus(null);
      if (mode === "slash") setDismissed(text);
    },
    step(direction: 1 | -1) {
      if (items.length === 0) return;
      setSelection({ key, index: (selectedIndex + direction + items.length) % items.length });
    },
    select(index: number) {
      setSelection({ key, index });
    },
    /** Puts the command in the field to be given its arguments; `from` is the draft it leaves. */
    begin(command: ComposerCommand, name: string, from: CommandBase, how: ActiveCommand["from"]) {
      setPlus(null);
      setActive({ command, name, from: how, base: from, text: {}, people: {}, current: 0 });
    },
    setArgument(id: string, value: string) {
      setActive((current) =>
        current === null ? current : { ...current, text: { ...current.text, [id]: value } },
      );
    },
    setPerson(id: string, person: MentionCandidate | undefined) {
      setActive((current) => {
        if (current === null) return current;
        const people = { ...current.people };
        if (person === undefined) delete people[id];
        else people[id] = person;
        return { ...current, people };
      });
    },
    focusArgument(index: number) {
      setActive((current) => (current === null ? current : { ...current, current: index }));
    },
    end() {
      setActive(null);
    },
  };
}

export type ComposerCommands = ReturnType<typeof useComposerCommands>;
