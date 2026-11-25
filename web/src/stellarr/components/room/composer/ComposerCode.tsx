import { useEffect, useLayoutEffect, useMemo, useRef, type RefObject } from "react";

import { Icon } from "@stellarr/icons";
import type { InterfaceLocale, Translator } from "@/i18n";

import { CodeLine, splitCodeLines } from "@/components/room/code/codeLines";
import {
  CodeLanguagePicker,
  type CodeLanguageChoice,
} from "@/components/room/code/CodeLanguagePicker";
import { pluralForm } from "@/lib/format/format";
import { tokenizeCode, type CodeLanguageId } from "@/lib/messages/codeLanguages";
import { CODE_MESSAGE_MAX_LENGTH } from "@/lib/messages/messageLimits";

import css from "./ComposerCode.module.css";

export { css as composerCodeStyles };

/** The counter shows once less than a tenth of the limit is left. */
const WARN_BELOW = CODE_MESSAGE_MAX_LENGTH / 10;

/** Whether the keyboard's send chord is written with ⌘ rather than Ctrl. */
function onApplePlatform(): boolean {
  if (typeof navigator === "undefined") return false;
  const hinted = (navigator as Navigator & { userAgentData?: { platform?: string } })
    .userAgentData?.platform;
  return /mac|iphone|ipad|ipod/i.test(hinted ?? navigator.platform ?? "");
}

export interface ComposerCodeHeadProps {
  language: CodeLanguageChoice;
  onLanguageChange(language: CodeLanguageChoice): void;
  /** What "Automatic" would pick now, or null while the draft is empty. */
  guess: CodeLanguageId | null;
  /** Characters the code may still grow by; below zero it is over the limit. */
  room: number;
  onDisable?: (() => void) | undefined;
  /** A language was picked in the open list; the code is where the next keys belong. */
  onPicked(): void;
  /** Ids the field points `aria-describedby` at. */
  helpId: string;
  counterId: string;
  locale: InterfaceLocale;
  t: Translator;
}

/**
 * The head the composer box gets in code mode: the language, how to send, how
 * much room is left near the limit, and the way out of code mode.
 */
export function ComposerCodeHead({
  language,
  onLanguageChange,
  guess,
  room,
  onDisable,
  onPicked,
  helpId,
  counterId,
  locale,
  t,
}: ComposerCodeHeadProps) {
  const sendKey = onApplePlatform() ? "⌘" : "Ctrl";
  const count = (value: number) => value.toLocaleString(locale === "en" ? "en-GB" : "pl-PL");
  const counter =
    room >= WARN_BELOW
      ? null
      : room >= 0
        ? t(`composer.codeLeft.${pluralForm(locale, room)}`, { count: count(room) })
        : t(`composer.codeOver.${pluralForm(locale, -room)}`, { count: count(-room) });

  return (
    // A choice made in the open list (the list is portalled, but its events
    // still pass through here) hands focus back to the code. Left on the
    // picker, the next letters typed would jump to another language.
    <div
      className={css.head}
      onClickCapture={(event) => {
        if (
          event.target instanceof Element &&
          event.target.closest('[role="option"]:not([aria-disabled="true"])') !== null
        ) {
          requestAnimationFrame(onPicked);
        }
      }}
      onKeyDownCapture={(event) => {
        if (
          (event.key === "Enter" || event.key === " ") &&
          event.target instanceof Element &&
          event.target.getAttribute("aria-expanded") === "true"
        ) {
          requestAnimationFrame(onPicked);
        }
      }}
    >
      <CodeLanguagePicker
        value={language}
        onChange={onLanguageChange}
        automatic={{ guess }}
        label={t("composer.codeLanguage")}
        className={css.picker}
        t={t}
      />
      <span className={css.spacer} />
      {/* Near the limit the count is what matters, and it takes the hint's place. */}
      {counter === null ? (
        <span className={css.keys} aria-hidden="true">
          <kbd>Enter</kbd> {t("composer.codeKeysNewLine")}, <kbd>{sendKey}</kbd>+<kbd>Enter</kbd>{" "}
          {t("composer.codeKeysSend")}
        </span>
      ) : (
        <span id={counterId} className={css.counter} data-over={room < 0 ? "" : undefined}>
          {counter}
        </span>
      )}
      <span id={helpId} className="sr-only">
        {t("composer.codeKeysHelp", { send: sendKey })}
      </span>
      {onDisable === undefined ? null : (
        <button
          type="button"
          className={css.close}
          aria-label={t("composer.disableCode")}
          title={t("composer.disableCode")}
          onClick={onDisable}
        >
          <Icon name="x" size={16} />
        </button>
      )}
    </div>
  );
}

/** Moves the layers under the field by as much as the field has scrolled. */
function followScroll(
  field: HTMLTextAreaElement,
  layer: HTMLElement | null,
  gutter: HTMLElement | null,
) {
  if (layer !== null) {
    layer.style.transform = `translate(${-field.scrollLeft}px, ${-field.scrollTop}px)`;
  }
  if (gutter !== null) gutter.style.transform = `translateY(${-field.scrollTop}px)`;
}

export interface ComposerCodeLayersProps {
  /** The textarea the layers are drawn under. */
  field: RefObject<HTMLTextAreaElement | null>;
  text: string;
  language: CodeLanguageId;
}

/**
 * The line numbers and the highlighted copy of the draft, drawn under a
 * textarea whose own text is transparent. The textarea stays the thing that
 * is typed in, selected, pasted into and undone; these only follow its scroll.
 */
export function ComposerCodeLayers({ field, text, language }: ComposerCodeLayersProps) {
  const gutter = useRef<HTMLDivElement | null>(null);
  const layer = useRef<HTMLElement | null>(null);

  // Tokenized once per change of the draft or the language. Each line renders
  // again only when its own runs changed, so a long draft stays smooth.
  const lines = useMemo(() => splitCodeLines(tokenizeCode(text, language)), [text, language]);
  const numbers = useMemo(
    () => Array.from({ length: lines.length }, (_, index) => index + 1).join("\n"),
    [lines.length],
  );

  useEffect(() => {
    const element = field.current;
    if (element === null) return;
    const follow = () => followScroll(element, layer.current, gutter.current);
    element.addEventListener("scroll", follow, { passive: true });
    return () => element.removeEventListener("scroll", follow);
  }, [field]);

  // A change can move the field's scroll before any scroll event says so, as
  // the field shrinking when lines are deleted does.
  useLayoutEffect(() => {
    const element = field.current;
    if (element !== null) followScroll(element, layer.current, gutter.current);
  }, [field, text]);

  return (
    <>
      <div className={css.gutter} aria-hidden="true">
        <div ref={gutter}>{numbers}</div>
      </div>
      <pre className={css.layer} aria-hidden="true">
        <code ref={layer} data-language={language}>
          {lines.map((runs, index) => (
            <CodeLine key={index} runs={runs} className={css.line} />
          ))}
        </code>
      </pre>
    </>
  );
}
