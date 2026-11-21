import { useDeferredValue, useMemo, useState, type FormEvent } from "react";

import type { InterfaceLocale, Translator } from "@/i18n";
import { triggerHaptic } from "@/lib/platform/haptics";

import {
  AUTO_LANGUAGE,
  type CodeLanguageChoice,
} from "@/components/room/code/CodeLanguagePicker";
import { resolveCodeLanguage, type CodeLanguageId } from "@/lib/messages/codeLanguages";
import type { ComposerChip } from "./ComposerChips";
import { fencedCode } from "@/lib/messages/composerText";
import { MESSAGE_MAX_LENGTH } from "@/lib/messages/messageLimits";

interface Options {
  chips: readonly ComposerChip[];
  /** Receives the trimmed text, fenced when the draft is a code message. */
  onSubmit(text: string): Promise<void> | void;
  /** The draft was delivered and consumed. */
  onSent(): void;
  locale: InterfaceLocale;
  t: Translator;
}

/** What the code looks like, as a reader would guess it from a fence with no language. */
function guessLanguage(source: string): CodeLanguageId {
  return resolveCodeLanguage("", source);
}

/**
 * The text being written and what it takes to send it: the code-message mode,
 * whether a send is in flight, and why the last one failed.
 */
export function useComposerDraft({ chips, onSubmit, onSent, locale, t }: Options) {
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [codeMode, setCodeMode] = useState(false);
  const [codeLanguage, setCodeLanguage] = useState<CodeLanguageChoice>(AUTO_LANGUAGE);

  // The guess reads the whole draft (it tries it as JSON, among others), so it
  // may trail the typing by a frame; the fence takes a fresh one on sending.
  const guessed = useDeferredValue(codeMode && codeLanguage === AUTO_LANGUAGE ? text : "");
  const codeGuess = useMemo(
    () => (guessed.trim().length === 0 ? null : guessLanguage(guessed)),
    [guessed],
  );
  /** The language the draft is highlighted in now and would be sent as. */
  const codeHighlight: CodeLanguageId =
    codeLanguage === AUTO_LANGUAGE ? (codeGuess ?? "plaintext") : codeLanguage;
  /**
   * How many characters the code may still grow by once it is fenced, which
   * is what the message limit counts. Null outside code mode.
   */
  const codeRoom = codeMode
    ? MESSAGE_MAX_LENGTH - fencedCode(codeHighlight, text.trim()).length
    : null;

  const hasPayload = text.trim().length > 0 || chips.length > 0;
  const attachmentsBlocked = chips.some((chip) => chip.status !== "ready");
  const canSend = hasPayload && !attachmentsBlocked && !sending;

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canSend) return;
    // Acknowledge the deliberate send gesture, not every keystroke. Kept in the
    // submit stack so mobile browsers can use their native haptic path.
    triggerHaptic("selection");
    setSending(true);
    setSendError(null);
    try {
      const draft = text;
      const authored = draft.trim();
      // An automatic choice is written into the fence, so every reader sees
      // the colours the author saw instead of guessing again.
      const language =
        codeLanguage === AUTO_LANGUAGE ? guessLanguage(authored) : codeLanguage;
      const body = codeMode ? fencedCode(language, authored) : authored;
      if (body.length > MESSAGE_MAX_LENGTH) {
        setSendError(
          t("composer.codeTooLong", {
            count: MESSAGE_MAX_LENGTH.toLocaleString(locale === "en" ? "en-GB" : "pl-PL"),
          }),
        );
        return;
      }
      await onSubmit(body);
      // A slow request must not erase what was typed after the send started.
      // In the usual append case only the delivered prefix is consumed; if the
      // original was edited in place, the whole current draft is kept.
      setText((current) =>
        current.startsWith(draft) ? current.slice(draft.length).trimStart() : current,
      );
      setCodeMode(false);
      onSent();
    } catch {
      // The text stays in the field: a failed send must not lose what was written.
      setSendError(t("composer.sendFailed"));
    } finally {
      setSending(false);
    }
  };

  return {
    text,
    setText,
    sending,
    sendError,
    clearSendError: () => setSendError(null),
    codeMode,
    setCodeMode,
    codeLanguage,
    setCodeLanguage,
    codeGuess,
    codeHighlight,
    codeRoom,
    hasPayload,
    attachmentsBlocked,
    canSend,
    submit,
  };
}
