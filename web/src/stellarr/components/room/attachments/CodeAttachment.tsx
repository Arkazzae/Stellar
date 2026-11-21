import { useEffect, useId, useMemo, useState } from "react";

import { Icon } from "@stellarr/icons";
import { useLanguage } from "@/state/hooks";

import { CodeLine, splitCodeLines } from "@/components/room/code/codeLines";
import {
  AUTO_LANGUAGE,
  CodeLanguagePicker,
  codeLanguageName,
} from "@/components/room/code/CodeLanguagePicker";
import { pluralForm } from "@/lib/format/format";
import {
  resolveCodeLanguage,
  tokenizeCode,
  type CodeLanguageId,
} from "@/lib/messages/codeLanguages";
import type { CodePart } from "@/lib/messages/parts";

export interface CodeAttachmentProps {
  part: CodePart;
  onCopy?(part: CodePart): void;
}

/** A listing longer than this starts collapsed, with a control to open it. */
const COLLAPSED_LINES = 16;
/** How long the copy button shows its outcome before going back to "copy". */
const COPY_FEEDBACK_MS = 1600;

type CopyState = "idle" | "copied" | "error";

export function CodeAttachment({ part, onCopy }: CodeAttachmentProps) {
  const { t, locale } = useLanguage();
  const contentId = useId();
  const source = part.source ?? part.tokens.map((token) => token.text).join("");
  const initialLanguage = part.language ?? resolveCodeLanguage(part.lang, source);
  const [language, setLanguage] = useState<CodeLanguageId>(initialLanguage);
  const [expanded, setExpanded] = useState(false);
  const [copyState, setCopyState] = useState<CopyState>("idle");

  const lineCount = source.length === 0 ? 0 : source.split("\n").length;
  const collapsible = lineCount > COLLAPSED_LINES;

  // The tokens that arrived with the part are only valid for the language they
  // were made in; any other choice is highlighted here.
  const tokens = useMemo(
    () =>
      language === initialLanguage && part.tokens.length > 0
        ? part.tokens
        : tokenizeCode(source, language),
    [initialLanguage, language, part.tokens, source],
  );
  const lines = useMemo(() => splitCodeLines(tokens), [tokens]);

  useEffect(() => {
    if (copyState === "idle") return;
    const timer = window.setTimeout(() => setCopyState("idle"), COPY_FEEDBACK_MS);
    return () => window.clearTimeout(timer);
  }, [copyState]);

  const copy = async () => {
    try {
      if (onCopy === undefined) await navigator.clipboard.writeText(source);
      else onCopy({ ...part, source, language, tokens });
      setCopyState("copied");
    } catch {
      setCopyState("error");
    }
  };

  const copyLabel =
    copyState === "copied"
      ? t("attachment.codeCopied")
      : copyState === "error"
        ? t("attachment.retryCodeCopy")
        : t("attachment.copyCode");

  return (
    <div className="att att-window att-code">
      <div className="code-head">
        {/* Recolours the block for this reader only; the message keeps the
            language it was sent with. */}
        <CodeLanguagePicker
          value={language}
          onChange={(next) => {
            if (next !== AUTO_LANGUAGE) setLanguage(next);
          }}
          label={t("attachment.highlightLanguage")}
          className="code-lang"
          t={t}
        />
        <span className="code-count">
          {t(`attachment.codeLines.${pluralForm(locale, lineCount)}`, { count: lineCount })}
        </span>
        {/* The button stays the same element through its feedback, so a
            keyboard press does not lose its place. */}
        <button
          type="button"
          className="code-copy"
          data-state={copyState}
          aria-label={copyLabel}
          title={copyLabel}
          onClick={() => void copy()}
        >
          <Icon name={copyState === "copied" ? "tick" : "copy"} size={16} />
          {copyState === "copied" ? (
            <span aria-hidden="true">{t("attachment.codeCopiedShort")}</span>
          ) : null}
        </button>
      </div>
      <pre
        id={contentId}
        className={collapsible ? (expanded ? "expanded" : "compact") : undefined}
        aria-label={t("attachment.codeLabel", { language: codeLanguageName(language, t) })}
        // Scrollable, so a keyboard user must be able to focus it.
        tabIndex={0}
      >
        <code data-language={language}>
          {lines.map((runs, index) => (
            <CodeLine key={index} runs={runs} className="code-line" />
          ))}
        </code>
      </pre>
      {collapsible ? (
        // Where the code breaks off, not up in the head, so the way on is
        // where the reader stopped.
        <button
          type="button"
          className="code-more"
          aria-expanded={expanded}
          aria-controls={contentId}
          onClick={() => setExpanded((current) => !current)}
        >
          {expanded ? t("attachment.showLessCode") : t("attachment.showWholeCode")}
          <Icon name={expanded ? "caretUp" : "caret"} size={14} />
        </button>
      ) : null}
    </div>
  );
}
