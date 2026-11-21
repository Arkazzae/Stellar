import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { Button, Spinner } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { useLanguage } from "@/state/LanguageProvider";

import { ConfirmSubjectRow, type ConfirmSubject } from "./ConfirmSubject";
import { focusWithin } from "./focus";
import { Modal } from "./Modal";

export type { ConfirmSubject } from "./ConfirmSubject";

/** One line of text the deed needs before it can be done. */
export interface ConfirmField {
  /** The field's own name, shown above it in sentence case. */
  label: string;
  placeholder?: string;
  maxLength?: number;
  /**
   * Empty is a valid answer, and the label says so. Without this the deed's
   * button stays disabled until something is typed.
   */
  optional?: boolean;
  /**
   * The exact words that answer it, such as the name of what is about to be
   * deleted. The deed's button stays disabled until they are typed, letter
   * case included, and a tick at the end of the field says they are.
   */
  match?: string;
  /** A sentence under the field: where the answer will be seen. */
  hint?: string;
}

export interface ConfirmDialogProps {
  open: boolean;
  /** The question itself: short, and it ends in a question mark. */
  title: string;
  /** What the question does not say, usually the consequence. */
  description?: string;
  /**
   * The one thing the question is about, drawn as a row under the sentence.
   * The title never names it, because a name inflected into a Polish sentence
   * comes out wrong.
   */
  subject?: ConfirmSubject;
  /** Names the deed ("Delete", "Block"), never "OK". */
  confirmLabel?: string;
  cancelLabel?: string;
  /** The deed destroys something: the button is red. */
  danger?: boolean;
  /** Turns the question into one that also collects an answer in words. */
  field?: ConfirmField;
  /**
   * Nothing can be done yet, so the card has one answer, which closes it.
   * Its label is `cancelLabel`, "Zamknij" unless given.
   */
  notice?: boolean;
  /** The deed is running: the card cannot be dismissed and the deed button says so. */
  busy?: boolean;
  /** What the deed's button says while it runs ("Usuwanie…"). */
  busyLabel?: string;
  /** Why the last attempt failed, above the answers. */
  error?: string | null;
  /**
   * Which question this is. A new one that replaces another on the open card
   * starts with an empty field and on its own first answer.
   */
  questionKey?: string | number;
  /** Receives the field's contents, trimmed; an empty string without a field. */
  onConfirm: (value: string) => void;
  onCancel: () => void;
}

/**
 * One question, two answers: the app's own surface in place of
 * `window.confirm`, and with a `field` in place of `window.prompt`.
 *
 * Cancel comes first in the DOM, so the modal's initial focus lands on the
 * harmless answer and a stray Enter costs nothing. A field takes the focus
 * instead, because a question that wants words is answered by typing. The
 * destructive button is styled as such but never focused.
 */
export function ConfirmDialog({
  open,
  title,
  description,
  subject,
  confirmLabel,
  cancelLabel,
  danger = false,
  field,
  notice = false,
  busy = false,
  busyLabel,
  error = null,
  questionKey,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const { t } = useLanguage();
  const titleId = useId();
  const fieldId = useId();
  const hintId = useId();
  const formRef = useRef<HTMLFormElement>(null);
  const [value, setValue] = useState("");

  // Cleared while rendering rather than in an effect, so a reused dialog is
  // never painted holding the previous answer.
  const [wasOpen, setWasOpen] = useState(open);
  const [wasKey, setWasKey] = useState(questionKey);
  if (wasOpen !== open || wasKey !== questionKey) {
    setWasOpen(open);
    setWasKey(questionKey);
    if (open) setValue("");
  }

  // The modal moves the focus in when it opens. A question that replaces
  // another on the open card does not reopen it, and the control that had the
  // focus may have left with the old question.
  const lastKey = useRef(questionKey);
  useEffect(() => {
    if (questionKey === lastKey.current) return;
    lastKey.current = questionKey;
    const form = formRef.current;
    if (form !== null && !form.contains(document.activeElement)) focusWithin(form);
  }, [questionKey]);

  const answer = value.trim();
  const matched = field?.match !== undefined && answer === field.match;
  const incomplete =
    field !== undefined &&
    (field.match !== undefined ? !matched : field.optional !== true && answer === "");

  return (
    // While the deed runs, Escape and the backdrop do nothing: the answer has
    // been given, and closing would hide whether it worked.
    <Modal open={open} onClose={busy ? () => undefined : onCancel} labelledBy={titleId} className="cf-wrap" exitMs={200}>
      {/* A form, so that Enter in the field answers the question. */}
      <form
        ref={formRef}
        className="cf-card"
        noValidate
        aria-busy={busy || undefined}
        onSubmit={(event) => {
          event.preventDefault();
          if (!notice && !incomplete && !busy) onConfirm(answer);
        }}
      >
        <div className="cf-body">
          <h2 id={titleId}>{title}</h2>
          {description === undefined ? null : <p className="cf-sub">{description}</p>}
          {subject === undefined ? null : <ConfirmSubjectRow subject={subject} />}
          {field === undefined ? null : (
            <div className="cf-field">
              <label className="cf-lab" htmlFor={fieldId}>
                {field.label}
                {field.optional === true ? <i>{t("confirm.optional")}</i> : null}
              </label>
              <span className={matched ? "cf-in matched" : "cf-in"}>
                <input
                  id={fieldId}
                  className="set-input"
                  type="text"
                  value={value}
                  placeholder={field.placeholder}
                  maxLength={field.maxLength}
                  autoComplete="off"
                  spellCheck={false}
                  readOnly={busy}
                  aria-describedby={field.hint === undefined ? undefined : hintId}
                  onChange={(event) => setValue(event.target.value)}
                />
                {matched ? (
                  <span className="cf-ok" aria-hidden="true">
                    <Icon name="tick" size={16} />
                  </span>
                ) : null}
              </span>
              {field.hint === undefined ? null : (
                <span className="cf-hint" id={hintId}>
                  {field.hint}
                </span>
              )}
            </div>
          )}
        </div>
        {error === null ? null : (
          <p className="cf-err" role="alert">
            <Icon name="warning" size={16} />
            <span>{error}</span>
          </p>
        )}
        <div className="cf-foot">
          <Button type="button" variant="secondary" disabled={busy} onClick={onCancel}>
            {cancelLabel ?? (notice ? t("common.close") : t("common.cancel"))}
          </Button>
          {notice ? null : (
            // Busy, the button keeps the focus it was pressed with: a disabled
            // one would drop it to the page behind the card.
            <Button
              type="submit"
              variant={danger ? "danger-solid" : "primary"}
              disabled={incomplete}
              aria-busy={busy || undefined}
              className={busy ? "busy" : undefined}
            >
              {busy ? (
                <>
                  <Spinner />
                  {busyLabel ?? confirmLabel ?? t("common.confirm")}
                </>
              ) : (
                (confirmLabel ?? t("common.confirm"))
              )}
            </Button>
          )}
        </div>
      </form>
    </Modal>
  );
}

/** Everything the dialog renders, and the deed it may run; answering is the provider's half. */
export type ConfirmRequest = Omit<
  ConfirmDialogProps,
  "open" | "onConfirm" | "onCancel" | "busy" | "error"
> & {
  /**
   * The deed itself, given the typed answer. With it the card stays open while
   * the deed runs and closes only once it has succeeded; a rejection keeps the
   * card open with `failure` above the answers, and the answers work again.
   * Without it the card closes on the answer and the caller acts.
   */
  run?: (answer: string) => Promise<unknown>;
  /** The error line when `run` rejects; a generic one unless given. */
  failure?: string;
};

/** A question with a line to fill in: the field is the point. */
export interface PromptRequest extends ConfirmRequest {
  field: ConfirmField;
}

/**
 * Resolves true only if the reader pressed the confirming button, and, for a
 * request with a `run`, only after the deed succeeded.
 */
export type Confirm = (request: ConfirmRequest) => Promise<boolean>;

/** Resolves the typed answer, trimmed, or null if the question was dismissed. */
export type Prompt = (request: PromptRequest) => Promise<string | null>;

interface ConfirmValue {
  confirm: Confirm;
  prompt: Prompt;
}

const ConfirmContext = createContext<ConfirmValue | null>(null);

/** One question as it was asked: the number tells a newer one apart. */
interface Asked {
  id: number;
  request: ConfirmRequest;
}

/**
 * The app's one dialog, and the promises that stand in for `window.confirm`
 * and `window.prompt`.
 *
 * Callers await an answer instead of rendering a dialog of their own because
 * whatever asks (a context menu, a hover toolbar) is usually gone by the time
 * the reader answers, and would take its dialog with it:
 *
 *     if (!(await confirm(deleteMessageConfirmation(t)))) return;
 *
 * Only one question at a time. A second replaces the first, and the first
 * resolves as "no" so that no handler is left waiting forever. A deed that
 * settles after its question was replaced changes nothing on the card.
 */
export function ConfirmProvider({ children }: { children: ReactNode }) {
  const { t } = useLanguage();
  // `asked` outlives `open`, so the card still has its text while it fades out.
  const [asked, setAsked] = useState<Asked | null>(null);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const counter = useRef(0);
  // The question still waiting for an answer, and what resolves it: the typed
  // answer, or null when it was dismissed.
  const pendingRef = useRef<{ id: number; resolve: (answer: string | null) => void } | null>(null);
  const runningRef = useRef<number | null>(null);

  const settle = useCallback((id: number, answer: string | null) => {
    const pending = pendingRef.current;
    if (pending === null || pending.id !== id) return;
    pendingRef.current = null;
    runningRef.current = null;
    setOpen(false);
    setBusy(false);
    pending.resolve(answer);
  }, []);

  const ask = useCallback((request: ConfirmRequest) => {
    pendingRef.current?.resolve(null);
    counter.current += 1;
    const id = counter.current;
    runningRef.current = null;
    setAsked({ id, request });
    setOpen(true);
    setBusy(false);
    setFailed(false);
    return new Promise<string | null>((resolve) => {
      pendingRef.current = { id, resolve };
    });
  }, []);

  const confirm = useCallback<Confirm>(async (next) => (await ask(next)) !== null, [ask]);

  const value = useMemo<ConfirmValue>(() => ({ confirm, prompt: ask }), [confirm, ask]);

  // A question that leaves with the tree it was asked from was never answered.
  useEffect(
    () => () => {
      pendingRef.current?.resolve(null);
      pendingRef.current = null;
    },
    [],
  );

  const answer = (current: Asked, typed: string) => {
    const { id, request } = current;
    if (request.run === undefined) {
      settle(id, typed);
      return;
    }
    if (runningRef.current === id) return;
    runningRef.current = id;
    setBusy(true);
    setFailed(false);
    const { run } = request;
    // Called at once, so the deed starts with the press, and wrapped, so a
    // throw or a handler that returns nothing still settles the card.
    let deed: Promise<unknown>;
    try {
      deed = Promise.resolve(run(typed));
    } catch (cause) {
      deed = Promise.reject(cause);
    }
    deed.then(
      () => settle(id, typed),
      () => {
        if (pendingRef.current?.id !== id) return;
        runningRef.current = null;
        setBusy(false);
        setFailed(true);
      },
    );
  };

  return (
    <ConfirmContext.Provider value={value}>
      {children}
      {asked === null ? null : (
        <ConfirmDialog
          // The deed and its failure text ride along and are ignored there:
          // running the deed is the provider's half.
          {...asked.request}
          questionKey={asked.id}
          open={open}
          busy={busy}
          error={failed ? (asked.request.failure ?? t("confirm.failed")) : null}
          onConfirm={(typed) => answer(asked, typed)}
          onCancel={() => settle(asked.id, null)}
        />
      )}
    </ConfirmContext.Provider>
  );
}

function useConfirmContext(): ConfirmValue {
  const value = useContext(ConfirmContext);
  if (value === null) {
    throw new Error("useConfirm must be used inside <ConfirmProvider>");
  }
  return value;
}

export function useConfirm(): Confirm {
  return useConfirmContext().confirm;
}

export function usePrompt(): Prompt {
  return useConfirmContext().prompt;
}
