import { useEffect, useId, useRef, type KeyboardEvent } from "react";

import type { InterfaceLocale, Translator } from "@/i18n";
import type { MentionCandidate } from "@/lib/messages/composerMentions";

import { ArgumentHint } from "./CommandList";
import css from "./ComposerCommands.module.css";
import { MentionList, mentionOptionId } from "./MentionList";
import type { ActiveCommand } from "./useComposerCommands";
import { useMentionSuggestions } from "./useMentionSuggestions";

export interface CommandArgsProps {
  active: ActiveCommand;
  /** Who a person argument offers. */
  people: readonly MentionCandidate[];
  /** Why a typed value cannot be used, by argument id, in words. */
  problems?: Readonly<Record<string, string>>;
  locale: InterfaceLocale;
  onText(id: string, value: string): void;
  onPerson(id: string, person: MentionCandidate | undefined): void;
  onFocusArgument(index: number): void;
  /** Enter: the command runs, or asks, once what it needs is there. */
  onRun(): void;
  /** Escape, or Backspace in an empty first argument: the command goes back to being text. */
  onBack(): void;
  t: Translator;
}

/**
 * A chosen command in the field: its name as a header, then one field per
 * argument. Tab and Shift+Tab move between the arguments, because the shell
 * keeps Tab from walking the page. A person is picked from the same list "@"
 * opens; typing over a picked person lets go of them.
 */
export function CommandArgs({
  active,
  people,
  problems = {},
  locale,
  onText,
  onPerson,
  onFocusArgument,
  onRun,
  onBack,
  t,
}: CommandArgsProps) {
  const { command, current } = active;
  const inputs = useRef<(HTMLInputElement | null)[]>([]);
  const hintId = useId();
  const peopleListId = useId();

  const arg = command.args[current];
  const personArg = arg?.kind === "person" ? arg : undefined;
  const picked = personArg === undefined ? undefined : active.people[personArg.id];
  const suggestions = useMentionSuggestions({
    text: personArg === undefined ? "" : (active.text[personArg.id] ?? ""),
    candidates: people,
    locale,
    enabled: personArg !== undefined && picked === undefined,
    whole: true,
  });
  const selectedPerson = suggestions.open ? suggestions.selected : undefined;

  // The argument being filled in has the keyboard: the first one when the
  // command is chosen, the next one after Tab or a pick.
  useEffect(() => {
    inputs.current[current]?.focus();
  }, [current]);

  const move = (to: number) => {
    if (to < 0 || to >= command.args.length) return;
    onFocusArgument(to);
  };

  const pick = (index: number, person: MentionCandidate) => {
    const target = command.args[index];
    if (target === undefined) return;
    onText(target.id, person.displayName);
    onPerson(target.id, person);
    move(index + 1);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>, index: number) => {
    if (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) return;
    const target = command.args[index];
    if (target === undefined) return;
    const listing = index === current && suggestions.open;
    const field = event.currentTarget;
    switch (event.key) {
      case "ArrowDown":
      case "ArrowUp":
        if (!listing) return;
        event.preventDefault();
        suggestions.step(event.key === "ArrowDown" ? 1 : -1);
        return;
      case "Enter":
        // Never the form's own submit: that would send the draft underneath.
        event.preventDefault();
        if (listing && selectedPerson !== undefined) pick(index, selectedPerson);
        else onRun();
        return;
      case "Tab":
        if (event.ctrlKey || event.metaKey || event.altKey) return;
        event.preventDefault();
        if (!event.shiftKey && listing && selectedPerson !== undefined) pick(index, selectedPerson);
        else move(index + (event.shiftKey ? -1 : 1));
        return;
      case "Escape":
        event.preventDefault();
        // Taken here, so nothing beneath the composer acts on it too.
        event.stopPropagation();
        // The people list opens by itself on an empty field; nobody asked for
        // it, so Escape there leaves the command rather than the list.
        if (listing && field.value !== "") suggestions.dismiss();
        else onBack();
        return;
      case "Backspace": {
        const empty = field.value === "";
        if (target.kind === "person" && active.people[target.id] !== undefined) {
          // A picked person goes as a whole, not a letter at a time.
          event.preventDefault();
          onPerson(target.id, undefined);
          onText(target.id, "");
          return;
        }
        if (!empty || field.selectionStart !== 0) return;
        event.preventDefault();
        if (index === 0) onBack();
        else move(index - 1);
        return;
      }
      default:
    }
  };

  const describedBy = suggestions.open ? undefined : hintId;

  return (
    <>
      {suggestions.open ? (
        <MentionList
          id={peopleListId}
          label={t("composer.mentionPeople")}
          candidates={suggestions.matches}
          selectedIndex={suggestions.selectedIndex}
          onChoose={(person) => pick(current, person)}
        />
      ) : arg === undefined ? null : (
        <ArgumentHint
          id={hintId}
          label={t(arg.label)}
          optional={arg.optional}
          hint={t(arg.hint)}
          {...(problems[arg.id] === undefined ? {} : { problem: problems[arg.id] })}
          t={t}
        />
      )}
      <div
        className={css.args}
        role="group"
        aria-label={t("composer.commands.command", { name: `/${active.name}` })}
      >
        <span className={css.commandName} aria-hidden="true">
          /{active.name}
        </span>
        {command.args.map((entry, index) => {
          const value = active.text[entry.id] ?? "";
          const placeholder = entry.optional ? t("composer.commands.optional") : "";
          const person = entry.kind === "person";
          const here = index === current;
          return (
            <label key={entry.id} className={here ? `${css.arg} ${css.current}` : css.arg}>
              <i>{t(entry.label)}</i>
              <input
                ref={(node) => {
                  inputs.current[index] = node;
                }}
                type="text"
                value={value}
                placeholder={placeholder}
                maxLength={entry.maxLength}
                size={Math.max(4, placeholder.length, value.length + 1)}
                autoComplete="off"
                spellCheck={person ? false : undefined}
                enterKeyHint="go"
                role={person ? "combobox" : undefined}
                aria-autocomplete={person ? "list" : undefined}
                aria-expanded={person ? here && suggestions.open : undefined}
                aria-controls={person && here && suggestions.open ? peopleListId : undefined}
                aria-activedescendant={
                  person && here && selectedPerson !== undefined
                    ? mentionOptionId(peopleListId, selectedPerson)
                    : undefined
                }
                aria-describedby={here ? describedBy : undefined}
                aria-invalid={problems[entry.id] === undefined ? undefined : true}
                onFocus={() => {
                  if (!here) onFocusArgument(index);
                }}
                onChange={(event) => {
                  onText(entry.id, event.target.value);
                  if (person) {
                    onPerson(entry.id, undefined);
                    suggestions.track(0);
                  }
                }}
                onKeyDown={(event) => onKeyDown(event, index)}
              />
            </label>
          );
        })}
      </div>
    </>
  );
}
