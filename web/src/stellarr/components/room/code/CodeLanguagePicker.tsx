import { useMemo } from "react";

import { cx, Dropdown, type DropdownOption } from "@stellarr/ui";
import type { Translator } from "@/i18n";

import {
  CODE_LANGUAGES,
  codeLanguageLabel,
  type CodeLanguageId,
} from "@/lib/messages/codeLanguages";

import css from "./CodeLanguagePicker.module.css";

/** The composer's default: highlight as whatever the code looks like. */
export const AUTO_LANGUAGE = "auto";
export type CodeLanguageChoice = CodeLanguageId | typeof AUTO_LANGUAGE;

/** Where the languages proper start: everything but plain text. */
const FIRST_LANGUAGE = CODE_LANGUAGES.find((language) => language.id !== "plaintext")?.id;

export interface CodeLanguagePickerProps {
  value: CodeLanguageChoice;
  onChange(value: CodeLanguageChoice): void;
  /** The accessible name of the field and its list. */
  label: string;
  /**
   * Offers "Automatic" first, with the language it would pick now beside it,
   * or nothing beside it while there is no code to guess from. Without it the
   * list is only the languages, under its own label as a heading.
   */
  automatic?: { guess: CodeLanguageId | null };
  className?: string;
  t: Translator;
}

/** A language's name as the lists show it; plain text is named in the interface's language. */
export function codeLanguageName(language: CodeLanguageId, t: Translator): string {
  return language === "plaintext" ? t("code.plaintext") : codeLanguageLabel(language);
}

/**
 * The one control that picks how code is highlighted: in the composer, where
 * the choice travels with the message, and on a code block, where it only
 * recolours the block for this reader.
 */
export function CodeLanguagePicker({
  value,
  onChange,
  label,
  automatic,
  className,
  t,
}: CodeLanguagePickerProps) {
  const offersAutomatic = automatic !== undefined;
  const guess = automatic?.guess ?? null;

  const options = useMemo(() => {
    const languages = CODE_LANGUAGES.map(
      (language, index): DropdownOption<CodeLanguageChoice> => ({
        value: language.id,
        label: codeLanguageName(language.id, t),
        // With "Automatic" on top, plain text stays beside it as the other
        // choice that is not a language, and the languages get a heading of
        // their own. Without it the list is headed by what it does.
        caption: offersAutomatic
          ? language.id === FIRST_LANGUAGE
            ? t("code.languages")
            : undefined
          : index === 0
            ? label
            : undefined,
      }),
    );
    if (!offersAutomatic) return languages;
    const auto: DropdownOption<CodeLanguageChoice> = {
      value: AUTO_LANGUAGE,
      label: t("code.automatic"),
      detail: guess === null ? undefined : codeLanguageName(guess, t),
    };
    return [auto, ...languages];
  }, [guess, label, offersAutomatic, t]);

  return (
    <Dropdown
      variant="quiet"
      options={options}
      value={value}
      onChange={onChange}
      label={label}
      className={cx(css.trigger, className)}
      listClassName={css.list}
    />
  );
}
