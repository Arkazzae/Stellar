import { locales } from "@/i18n";
import { useDeferredValue, useMemo, useState, useSyncExternalStore, type FormEvent } from "react";

import type { InterfaceLocale, MessageKey, Translator } from "@/i18n";
import { composerDraft } from "@/lib/platform/drafts";
import { timeProblem } from "@/lib/schedule/scheduleTime";
import { ScheduleRefusedError, type ScheduleRefusalReason } from "@/ports";
import { triggerHaptic } from "@/lib/platform/haptics";

import {
  AUTO_LANGUAGE,
  type CodeLanguageChoice,
} from "@/components/room/code/CodeLanguagePicker";
import { resolveCodeLanguage, type CodeLanguageId } from "@/lib/messages/codeLanguages";
import type { ComposerChip } from "./ComposerChips";
import { fencedCode } from "@/lib/messages/composerText";
import { CODE_MESSAGE_MAX_LENGTH, MESSAGE_MAX_LENGTH, messageContentLimit } from "@/lib/messages/messageLimits";

interface Options {
  draftScope?: string;
  restoringFiles?: boolean;
  chips: readonly ComposerChip[];
  /** Receives the trimmed text, fenced when the draft is a code message. */
  onSubmit(text: string): Promise<void> | void;
  /** Schedules the same text instead, while a send time is chosen. */
  onSchedule?(text: string, sendAt: Date): Promise<void> | void;
  /** The clock a chosen time is checked against when it is used. */
  clock?: () => Date;
  /** The draft was delivered and consumed. */
  onSent(): void;
  locale: InterfaceLocale;
  t: Translator;
}

/** Why a schedule was refused, in the words the field shows under itself. */
export const SCHEDULE_REFUSALS: Record<ScheduleRefusalReason, MessageKey> = {
  time: "composer.schedule.refused.time",
  content: "composer.schedule.refused.content",
  attachments: "composer.schedule.refused.attachments",
  limit: "composer.schedule.refused.limit",
  encrypted: "composer.schedule.refused.encrypted",
  failed: "composer.schedule.refused.failed",
  forbidden: "composer.schedule.refused.forbidden",
  slowMode: "composer.schedule.refused.slowMode",
  changed: "composer.schedule.refused.changed",
  other: "composer.schedule.refused.other",
};

const systemClock = () => new Date();

/** What the code looks like, as a reader would guess it from a fence with no language. */
function guessLanguage(source: string): CodeLanguageId {
  return resolveCodeLanguage("", source);
}

/**
 * The text being written and what it takes to send it: the code-message mode,
 * whether a send is in flight, and why the last one failed.
 */
export function useComposerDraft({
  draftScope,
  restoringFiles,
  chips,
  onSubmit,
  onSchedule,
  clock = systemClock,
  onSent,
  locale,
  t,
}: Options) {
  const entry = useMemo(() => composerDraft(draftScope), [draftScope]);
  const { draft: { text, manualCode, codeLanguage, sendAt: chosenAt }, loading, failed } = useSyncExternalStore(entry.subscribe, entry.get, entry.get);
  const setText = (text: string) => entry.change(draft => ({ ...draft, text }));
  // A time chosen where scheduling is not offered (a conversation that has
  // since been encrypted) is not one: the message would only post.
  const sendAt = chosenAt === undefined || onSchedule === undefined ? null : new Date(chosenAt);
  const setSendAt = (at: Date | null) =>
    entry.change(({ sendAt: _previous, ...draft }) => (at === null ? draft : { ...draft, sendAt: at.toISOString() }));
  const setCodeMode = (manualCode: boolean) => entry.change(draft => ({ ...draft, manualCode }));
  const setCodeLanguage = (codeLanguage: CodeLanguageChoice) => entry.change(draft => ({ ...draft, codeLanguage }));
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const authored = text.trim();
  const automaticCode = useMemo(
    () => authored.length > MESSAGE_MAX_LENGTH && messageContentLimit(authored) === MESSAGE_MAX_LENGTH,
    [authored],
  );
  const codeMode = manualCode || automaticCode;

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
    ? CODE_MESSAGE_MAX_LENGTH - fencedCode(codeHighlight, text).length
    : null;

  const hasPayload = authored.length > 0 || chips.length > 0;
  const attachmentsBlocked = chips.some((chip) => chip.status !== "ready");
  const canSend = !loading && !restoringFiles && hasPayload && !attachmentsBlocked && !sending;

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canSend) return;
    // The chosen time may have passed while the message was written.
    const problem = sendAt === null ? null : timeProblem(sendAt, clock());
    if (problem !== null) {
      setSendError(t(problem === "tooFar" ? "composer.schedule.tooFar" : "composer.schedule.passed"));
      return;
    }
    // Acknowledge the deliberate send gesture, not every keystroke. Kept in the
    // submit stack so mobile browsers can use their native haptic path.
    triggerHaptic("selection");
    setSending(true);
    setSendError(null);
    try {
      const draft = text;
      // An automatic choice is written into the fence, so every reader sees
      // the colours the author saw instead of guessing again.
      const language =
        codeMode && codeLanguage === AUTO_LANGUAGE ? guessLanguage(draft) : codeHighlight;
      const body = codeMode && authored.length > 0 ? fencedCode(language, draft) : authored;
      if (body.length > CODE_MESSAGE_MAX_LENGTH) {
        setSendError(
          t("composer.codeTooLong", {
            count: CODE_MESSAGE_MAX_LENGTH.toLocaleString(locales[locale].tag),
          }),
        );
        return;
      }
      if (sendAt !== null && onSchedule !== undefined) await onSchedule(body, sendAt);
      else await onSubmit(body);
      // A slow request must not erase what was typed after the send started.
      // In the usual append case only the delivered prefix is consumed; if the
      // original was edited in place, the whole current draft is kept. The
      // time goes with the message it was chosen for.
      entry.change(({ sendAt: _used, ...current }) => ({
        ...current,
        text: current.text.startsWith(draft) ? current.text.slice(draft.length).trimStart() : current.text,
        manualCode: false,
      }));
      onSent();
    } catch (cause) {
      // The text, and the chosen time, stay: a failed send must not lose what
      // was written. A refused schedule says why.
      setSendError(
        cause instanceof ScheduleRefusedError ? t(SCHEDULE_REFUSALS[cause.reason]) : t("composer.sendFailed"),
      );
    } finally {
      setSending(false);
    }
  };

  return {
    text,
    setText,
    sending,
    loading,
    sendError: sendError ?? (failed && (text || chips.length) ? t("pwa.draftFailed") : null),
    clearSendError: () => setSendError(null),
    codeMode,
    automaticCode,
    setCodeMode,
    codeLanguage,
    setCodeLanguage,
    codeGuess,
    codeHighlight,
    codeRoom,
    sendAt,
    setSendAt,
    hasPayload,
    attachmentsBlocked,
    canSend,
    submit,
  };
}
