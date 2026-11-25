import { useEffect, useLayoutEffect, useRef, useState } from "react";

import type { Translator } from "@/i18n";
import { documentLocale } from "@/i18n/defaultLocale";

import { resolveCodeLanguage } from "@/lib/messages/codeLanguages";
import { fencedCode } from "@/lib/messages/composerText";
import { CODE_MESSAGE_MAX_LENGTH, messageContentLimit } from "@/lib/messages/messageLimits";

/** How tall the field may grow before it scrolls: eight lines, like the composer. */
const MAX_HEIGHT_PX = 8 * 24;

export interface MessageEditorProps {
  /** What the message says now; the field starts from it. */
  original: string;
  /** A rejection keeps the editor open with the text intact and says it failed. */
  onSave(text: string): Promise<void> | void;
  onCancel(): void;
  t: Translator;
}

/**
 * A message being rewritten in the place it sits.
 *
 * Its own component so the draft dies with it: closing the editor and opening
 * it again starts from what the message actually says, never from an abandoned
 * attempt. Which message is being edited belongs to the provider, and arrives
 * as `original`.
 *
 * The keys are the composer's, because this is still writing a message: Enter
 * saves, Shift+Enter breaks a line, Escape gives up. Escape is stopped here so
 * it does not also close whatever is behind the transcript.
 */
export function MessageEditor({ original, onSave, onCancel, t }: MessageEditorProps) {
  const [text, setText] = useState(original);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const field = useRef<HTMLTextAreaElement>(null);

  // Caret at the end, nothing selected: a selected body would be wiped by the
  // first keystroke of somebody who came to change one word.
  useEffect(() => {
    const element = field.current;
    if (element === null) return;
    element.focus();
    element.setSelectionRange(element.value.length, element.value.length);
  }, []);

  // Grows with the text: an editor that scrolls inside three lines is worse
  // than the message it replaced.
  useLayoutEffect(() => {
    const element = field.current;
    if (element === null) return;
    element.style.height = "auto";
    element.style.height = `${Math.min(element.scrollHeight, MAX_HEIGHT_PX)}px`;
  }, [text]);

  const empty = text.trim().length === 0;

  const save = async () => {
    // Emptying a message is deleting it, and there is a bin for that.
    if (empty) return;
    const authored = text.trim();
    if (authored === original.trim()) {
      onCancel();
      return;
    }
    const next = authored.length > messageContentLimit(authored) && authored.length <= CODE_MESSAGE_MAX_LENGTH
      ? fencedCode(resolveCodeLanguage("", text), text)
      : authored;
    if (next.length > CODE_MESSAGE_MAX_LENGTH) {
      setError(t("composer.codeTooLong", {
        count: CODE_MESSAGE_MAX_LENGTH.toLocaleString(documentLocale() === "en" ? "en-GB" : "pl-PL"),
      }));
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await onSave(next);
    } catch {
      setError(t("message.edit.failed"));
      setSaving(false);
    }
  };

  return (
    <div className="msg-edit">
      <textarea
        ref={field}
        className="msg-edit-field"
        rows={1}
        value={text}
        aria-label={t("message.edit.label")}
        disabled={saving}
        onChange={(event) => {
          setText(event.target.value);
          setError(null);
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            onCancel();
          } else if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
            // Enter also closes an IME candidate list; saving then would eat
            // the word still being composed.
            event.preventDefault();
            void save();
          }
        }}
      />
      <div className="msg-edit-foot">
        <span className="msg-edit-hint">{t("message.edit.hint")}</span>
        <button type="button" className="msg-edit-x" onClick={onCancel}>
          {t("common.cancel")}
        </button>
        <button
          type="button"
          className="msg-edit-ok"
          disabled={saving || empty}
          onClick={() => void save()}
        >
          {t("common.save")}
        </button>
      </div>
      {error !== null ? (
        <p className="acc-answer bad" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}
