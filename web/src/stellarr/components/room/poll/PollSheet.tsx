import { useId, useRef, useState, type CSSProperties, type DragEvent, type KeyboardEvent } from "react";

import { Button, cx, Dropdown } from "@stellarr/ui";
import { useLayer } from "@stellarr/ui/behavior";
import { Icon } from "@stellarr/icons";
import { EmojiPicker } from "@/components/emoji/EmojiPicker";
import { trapTab } from "@/components/primitives/focus";
import { Modal } from "@/components/primitives/Modal";
import { Switch } from "@/components/primitives/Switch";
import type { MessageKey, Translator } from "@/i18n";
import type { EmojiEntry } from "@/lib/emoji/emoji";
import {
  DEFAULT_POLL_DURATION,
  POLL_DURATIONS,
  POLL_LIMITS,
  PollDraftRefusedError,
  type CustomEmoji,
  type PollDraft,
  type PollDraftField,
  type PollDuration,
} from "@/ports";

import { PollEmoji } from "./PollEmoji";
import css from "./PollSheet.module.css";

export interface PollSheetProps {
  /** Typed after the command; the question field starts with it. */
  initialQuestion: string;
  /** Where it goes: "Na kanale #general". */
  place: string;
  customEmoji?: readonly CustomEmoji[];
  /** Rejects with a `PollDraftRefusedError` naming what the server refused. */
  onSend(draft: PollDraft): Promise<void>;
  onClose(): void;
  t: Translator;
}

interface AnswerRow {
  key: number;
  text: string;
  emoji?: string;
}

type Problems = {
  question?: MessageKey;
  answers?: MessageKey;
  duration?: MessageKey;
  /** By row key. */
  rows: ReadonlyMap<number, MessageKey>;
  general?: MessageKey;
};

const NO_PROBLEMS: Problems = { rows: new Map() };

/** What a reaction is stored as, and so an answer's emoji: the character, or `:shortcode:`. */
const emojiName = (entry: EmojiEntry) => (entry.imageUrl === undefined ? entry.glyph : `:${entry.shortcode}:`);

/** Puts the item at `from` at `to`, the others keeping their order. */
function moved<T>(items: readonly T[], from: number, to: number): T[] {
  const next = [...items];
  const [item] = next.splice(from, 1);
  if (item !== undefined) next.splice(to, 0, item);
  return next;
}

/**
 * A new poll: the question, its answers with an emoji each if wanted, how
 * long voting stays open and whether several answers may be chosen. Answers
 * left empty are not sent.
 */
export function PollSheet({ initialQuestion, place, customEmoji = [], onSend, onClose, t }: PollSheetProps) {
  const titleId = useId();
  const questionId = useId();
  const durationId = useId();
  const moveHintId = useId();
  const nextKey = useRef(3);
  const [question, setQuestion] = useState(initialQuestion.slice(0, POLL_LIMITS.question));
  const [answers, setAnswers] = useState<AnswerRow[]>([
    { key: 1, text: "" },
    { key: 2, text: "" },
  ]);
  const [duration, setDuration] = useState<PollDuration>(DEFAULT_POLL_DURATION);
  const [multiple, setMultiple] = useState(false);
  const [sending, setSending] = useState(false);
  const [problems, setProblems] = useState<Problems>(NO_PROBLEMS);
  /** The answer whose emoji is being chosen, and the button it opened from. */
  const [picking, setPicking] = useState<{ key: number; button: HTMLElement } | null>(null);
  const [dragged, setDragged] = useState<number | null>(null);
  const rows = useRef(new Map<number, HTMLInputElement>());

  const filled = answers.filter((answer) => answer.text.trim() !== "");
  const ready = question.trim() !== "" && filled.length >= POLL_LIMITS.minAnswers && !sending;

  const change = (key: number, patch: Partial<AnswerRow>) => {
    setAnswers((current) => current.map((answer) => (answer.key === key ? { ...answer, ...patch } : answer)));
    if (problems.rows.has(key)) {
      const rest = new Map(problems.rows);
      rest.delete(key);
      setProblems({ ...problems, rows: rest });
    }
  };

  const add = () => {
    if (answers.length >= POLL_LIMITS.maxAnswers) return;
    const key = nextKey.current++;
    setAnswers((current) => [...current, { key, text: "" }]);
    requestAnimationFrame(() => rows.current.get(key)?.focus());
  };

  const remove = (key: number) => {
    if (answers.length <= POLL_LIMITS.minAnswers) return;
    const at = answers.findIndex((answer) => answer.key === key);
    setAnswers((current) => current.filter((answer) => answer.key !== key));
    // The field that takes its place, or the one before the last.
    const after = answers[at + 1] ?? answers[at - 1];
    if (after !== undefined) requestAnimationFrame(() => rows.current.get(after.key)?.focus());
  };

  const move = (key: number, by: -1 | 1) => {
    const from = answers.findIndex((answer) => answer.key === key);
    const to = from + by;
    if (from === -1 || to < 0 || to >= answers.length) return;
    setAnswers((current) => moved(current, from, to));
  };

  const send = async () => {
    if (!ready) return;
    const sent = filled;
    setSending(true);
    setProblems(NO_PROBLEMS);
    try {
      await onSend({
        question,
        answers: sent.map(({ text, emoji }) => ({ text, ...(emoji === undefined ? {} : { emoji }) })),
        multiple,
        duration,
      });
      onClose();
    } catch (error) {
      setProblems(
        error instanceof PollDraftRefusedError
          ? read(error.fields, sent)
          : { ...NO_PROBLEMS, general: "composer.poll.problem.other" },
      );
    } finally {
      setSending(false);
    }
  };

  const dragHandlers = (index: number) => ({
    draggable: dragged !== null || undefined,
    onDragOver(event: DragEvent<HTMLDivElement>) {
      if (dragged === null) return;
      event.preventDefault();
      if (dragged !== index) {
        setAnswers((current) => moved(current, dragged, index));
        setDragged(index);
      }
    },
    onDrop(event: DragEvent<HTMLDivElement>) {
      event.preventDefault();
      setDragged(null);
    },
    onDragEnd: () => setDragged(null),
  });

  return (
    <Modal open onClose={onClose} labelledBy={titleId} className={cx(css.wrap)} exitMs={200}>
      <form
        className={css.sheet}
        onSubmit={(event) => {
          event.preventDefault();
          void send();
        }}
      >
        <h2 id={titleId}>{t("composer.poll.title")}</h2>
        <p className={css.sub}>{place}</p>

        <div className={css.field}>
          <label className={css.label} htmlFor={questionId}>
            {t("composer.poll.question")}
            <span className={css.count}>
              {t("composer.poll.length", { count: question.length, max: POLL_LIMITS.question })}
            </span>
          </label>
          <input
            id={questionId}
            className={css.input}
            value={question}
            maxLength={POLL_LIMITS.question}
            placeholder={t("composer.poll.questionPlaceholder")}
            aria-invalid={problems.question === undefined ? undefined : true}
            // Opened to be written in.
            autoFocus
            onChange={(event) => {
              setQuestion(event.target.value);
              if (problems.question !== undefined) setProblems({ ...problems, question: undefined });
            }}
            onKeyDown={(event) => {
              // Enter goes on to the answers; the poll is sent with its button.
              if (event.key !== "Enter") return;
              event.preventDefault();
              const first = answers[0];
              if (first !== undefined) rows.current.get(first.key)?.focus();
            }}
          />
          {problems.question === undefined ? null : <p className={css.problem}>{t(problems.question)}</p>}
        </div>

        <div className={css.field}>
          <span className={css.label}>
            {t("composer.poll.answers")}
            <span className={css.count}>
              {t("composer.poll.answersCount", { count: answers.length, max: POLL_LIMITS.maxAnswers })}
            </span>
          </span>
          <div className={css.answers}>
            {answers.map((answer, index) => {
              const label = t("composer.poll.answer", { number: index + 1 });
              const problem = problems.rows.get(answer.key);
              return (
                <div
                  key={answer.key}
                  className={[css.answer, dragged === index && css.dragging, problem !== undefined && css.invalid]
                    .filter(Boolean)
                    .join(" ")}
                  {...dragHandlers(index)}
                  onDragStart={(event) => event.dataTransfer.setData("text/plain", answer.text)}
                >
                  <button
                    type="button"
                    className={answer.emoji === undefined ? `${css.icon} ${css.emojiEmpty}` : css.icon}
                    aria-label={answer.emoji === undefined ? t("composer.poll.addEmoji") : t("composer.poll.answerEmoji")}
                    aria-expanded={picking?.key === answer.key}
                    onClick={(event) =>
                      setPicking(
                        picking?.key === answer.key
                          ? null
                          : { key: answer.key, button: event.currentTarget },
                      )
                    }
                  >
                    {answer.emoji === undefined ? (
                      <Icon name="smileyAdd" size={18} />
                    ) : (
                      <PollEmoji emoji={answer.emoji} customEmoji={customEmoji} />
                    )}
                  </button>
                  <input
                    ref={(element) => {
                      if (element === null) rows.current.delete(answer.key);
                      else rows.current.set(answer.key, element);
                    }}
                    className={css.answerInput}
                    value={answer.text}
                    maxLength={POLL_LIMITS.answer}
                    placeholder={label}
                    aria-label={label}
                    aria-invalid={problem === undefined ? undefined : true}
                    onChange={(event) => change(answer.key, { text: event.target.value })}
                    onKeyDown={(event) => {
                      // Enter goes on to the next answer, and past the last adds one.
                      if (event.key !== "Enter") return;
                      event.preventDefault();
                      const next = answers[index + 1];
                      if (next !== undefined) rows.current.get(next.key)?.focus();
                      else add();
                    }}
                  />
                  <button
                    type="button"
                    className={`${css.icon} ${css.onPoint} ${css.grip}`}
                    aria-label={t("composer.poll.moveAnswer")}
                    aria-describedby={moveHintId}
                    disabled={answers.length < 2}
                    onPointerDown={() => setDragged(index)}
                    onPointerUp={() => setDragged(null)}
                    onKeyDown={(event: KeyboardEvent<HTMLButtonElement>) => {
                      if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
                      event.preventDefault();
                      move(answer.key, event.key === "ArrowUp" ? -1 : 1);
                    }}
                  >
                    <Icon name="grip" size={16} />
                  </button>
                  {answers.length > POLL_LIMITS.minAnswers ? (
                    <button
                      type="button"
                      className={`${css.icon} ${css.onPoint}`}
                      aria-label={t("composer.poll.removeAnswer")}
                      onClick={() => remove(answer.key)}
                    >
                      <Icon name="x" size={16} />
                    </button>
                  ) : null}
                  {picking?.key === answer.key ? (
                    <AnswerEmojiPicker
                      anchor={picking.button}
                      customEmoji={customEmoji}
                      chosen={answer.emoji}
                      onPick={(emoji) => {
                        change(answer.key, emoji === undefined ? { emoji: undefined } : { emoji });
                        setPicking(null);
                      }}
                      onClose={() => setPicking(null)}
                      t={t}
                    />
                  ) : null}
                  {problem === undefined ? null : <p className={css.rowProblem}>{t(problem)}</p>}
                </div>
              );
            })}
          </div>
          <span id={moveHintId} hidden>
            {t("composer.poll.moveHint")}
          </span>
          {answers.length < POLL_LIMITS.maxAnswers ? (
            <button type="button" className={css.add} onClick={add}>
              <Icon name="plus" size={16} />
              {t("composer.poll.addAnswer")}
            </button>
          ) : null}
          {problems.answers === undefined ? null : <p className={css.problem}>{t(problems.answers)}</p>}
        </div>

        <div className={css.options}>
          <div className={css.field}>
            <label className={css.label} htmlFor={durationId}>
              {t("composer.poll.duration")}
            </label>
            <Dropdown
              id={durationId}
              options={POLL_DURATIONS.map((value) => ({
                value,
                label: t(`composer.poll.duration.${value}` as MessageKey),
              }))}
              value={duration}
              onChange={setDuration}
              invalid={problems.duration !== undefined}
            />
          </div>
          <label className={css.check}>
            <span>{t("composer.poll.multiple")}</span>
            <Switch checked={multiple} onChange={setMultiple} label={t("composer.poll.multiple")} />
          </label>
        </div>
        {problems.duration === undefined ? null : <p className={css.problem}>{t(problems.duration)}</p>}
        {problems.general === undefined ? null : (
          <p className={css.problem} role="alert">
            {t(problems.general)}
          </p>
        )}

        <div className={css.foot}>
          <Button type="button" variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button type="submit" variant="primary" disabled={!ready}>
            {t("composer.poll.send")}
          </Button>
        </div>
      </form>
    </Modal>
  );
}

/** What the server refused, put on the fields it named; answer numbers are of those sent. */
function read(fields: readonly PollDraftField[], sent: readonly AnswerRow[]): Problems {
  const rows = new Map<number, MessageKey>();
  const problems: Problems = { rows };
  for (const field of fields) {
    if (typeof field === "object") {
      const row = sent[field.answer];
      if (row !== undefined) rows.set(row.key, "composer.poll.problem.answer");
    } else if (field === "question") problems.question = "composer.poll.problem.question";
    else if (field === "answers") problems.answers = "composer.poll.problem.answers";
    else if (field === "duration") problems.duration = "composer.poll.problem.duration";
    else problems.general = "composer.poll.problem.attachments";
  }
  return problems;
}

/** The picker's room, as the emoji picker draws itself. */
const PICKER = { width: 352, height: 430, gap: 6, margin: 8 };

/**
 * Under the button it opened from, or over it where the window ends first,
 * kept inside the window. Fixed, so the sheet's scrolling does not clip it.
 */
function pickerPlace(anchor: DOMRect): CSSProperties {
  const below = anchor.bottom + PICKER.gap;
  const top =
    below + PICKER.height <= window.innerHeight - PICKER.margin
      ? below
      : Math.max(PICKER.margin, anchor.top - PICKER.gap - PICKER.height);
  const left = Math.max(PICKER.margin, Math.min(anchor.left, window.innerWidth - PICKER.width - PICKER.margin));
  return { top, left };
}

interface AnswerEmojiPickerProps {
  /** The emoji button it opened from. */
  anchor: HTMLElement;
  customEmoji: readonly CustomEmoji[];
  chosen: string | undefined;
  /** An emoji, or none. */
  onPick(emoji: string | undefined): void;
  onClose(): void;
  t: Translator;
}

/** The emoji picker over an answer, a layer of its own so Escape closes it and not the sheet. */
function AnswerEmojiPicker({ anchor, customEmoji, chosen, onPick, onClose, t }: AnswerEmojiPickerProps) {
  const panel = useRef<HTMLDivElement>(null);
  useLayer(true, {
    node: () => panel.current,
    onEscape: (event) => {
      event.preventDefault();
      onClose();
    },
    onTab: (event) => {
      if (panel.current !== null) trapTab(event, panel.current);
    },
    // Its own button closes it with its click; closing here would open it again.
    onPointerDownOutside: (event) => {
      if (!(event.target instanceof Node && anchor.contains(event.target))) onClose();
    },
  });
  return (
    <div
      ref={panel}
      className={css.picker}
      style={pickerPlace(anchor.getBoundingClientRect())}
      role="dialog"
      aria-label={t("composer.poll.answerEmoji")}
    >
      {chosen === undefined ? null : (
        <button type="button" className={css.noEmoji} onClick={() => onPick(undefined)}>
          <Icon name="x" size={14} />
          {t("composer.poll.noEmoji")}
        </button>
      )}
      <EmojiPicker customEmoji={customEmoji} onPick={(entry) => onPick(emojiName(entry))} onClose={onClose} />
    </div>
  );
}
