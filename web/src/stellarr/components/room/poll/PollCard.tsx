import { useEffect, useId, useState } from "react";

import { Icon } from "@stellarr/icons";
import { Avatar } from "@/components/primitives/Avatar";
import { useConfirm } from "@/components/primitives/confirmContext";
import type { MessageKey } from "@/i18n";
import { pluralForm } from "@/lib/format/format";
import { pollCountLabel, pollEndedLabel, pollRemainingLabel } from "@/lib/polls/pollLabels";
import { isPollClosed, leadingAnswers, myAnswers, pickAnswer, pollEndedAt, shareOf } from "@/lib/polls/pollState";
import {
  PollRefusedError,
  type CustomEmoji,
  type Poll,
  type PollAnswer,
  type PollRefusalReason,
  type PollVoter,
  type PollVoterPage,
} from "@/ports";
import { useLanguage } from "@/state/hooks";

import { PollEmoji } from "./PollEmoji";
import css from "./PollCard.module.css";

/** What a poll on screen can ask for. */
export interface PollCallbacks {
  /** Sets the reader's whole vote. Rejects with a `PollRefusedError` when refused. */
  vote(messageId: string, answers: readonly number[]): Promise<void>;
  /** Ends the poll now. Rejects with a `PollRefusedError` when refused. */
  end(messageId: string): Promise<void>;
  /** A page of the people who chose an answer. */
  voters(messageId: string, answerId: number, cursor?: string): Promise<PollVoterPage>;
  /** The picture of someone who voted, when it is known. */
  faceOf?(userId: string): string | undefined;
}

export interface PollCardProps {
  messageId: string;
  poll: Poll;
  /** The reader wrote it or may manage messages here, so may end it early. */
  canEnd: boolean;
  /** Without them the card only shows the poll. */
  actions?: PollCallbacks;
  /** Resolves an answer's `:shortcode:` to the server's image. */
  customEmoji?: readonly CustomEmoji[];
  /** The time; a test sets it. */
  clock?: () => Date;
}

const MINUTE = 60_000;
/** Names before "and N others" in the list of who chose an answer. */
const NAMED_VOTERS = 3;
const systemClock = () => new Date();

const REFUSALS: Record<PollRefusalReason, MessageKey> = {
  ended: "poll.refused.ended",
  selection: "poll.refused.selection",
  forbidden: "poll.refused.forbidden",
  other: "poll.refused.other",
};

/**
 * The time a poll is read at: once a minute while it runs, for the time left,
 * and at the moment it ends, so it closes on screen without being told.
 */
function usePollNow(poll: Poll, clock: () => Date): Date {
  const [now, setNow] = useState(clock);
  const closed = isPollClosed(poll, now);
  useEffect(() => {
    if (closed) return;
    const untilEnd = Date.parse(poll.endsAt) - now.getTime();
    const timer = window.setTimeout(() => setNow(clock()), Math.max(1_000, Math.min(MINUTE, untilEnd + 50)));
    return () => window.clearTimeout(timer);
  }, [closed, poll.endsAt, now, clock]);
  return now;
}

type VoterList = { votes: number; voters: PollVoter[] } | "loading";

/**
 * A poll in the conversation: the question, its answers to pick from, then
 * the results as a thin bar under each answer. Your answers carry a check and
 * the accent; the rest is grey. Once it is over the leading answer is bold
 * and nothing more can be done with it.
 */
export function PollCard({ messageId, poll, canEnd, actions, customEmoji = [], clock = systemClock }: PollCardProps) {
  const { t, locale } = useLanguage();
  const confirm = useConfirm();
  const now = usePollNow(poll, clock);
  const tipId = useId();
  const [picked, setPicked] = useState<number[]>([]);
  const [revealed, setRevealed] = useState(false);
  const [changing, setChanging] = useState(false);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<MessageKey | null>(null);
  const [shown, setShown] = useState<number | null>(null);
  const [voterLists, setVoterLists] = useState<ReadonlyMap<number, VoterList>>(new Map());

  const closed = isPollClosed(poll, now);
  const mine = myAnswers(poll);
  const voted = mine.length > 0;
  const view = closed ? "ended" : changing ? "vote" : voted || revealed ? "results" : "vote";
  const leading = closed ? leadingAnswers(poll) : new Set<number>();

  const run = async (deed: () => Promise<void>) => {
    setBusy(true);
    setProblem(null);
    try {
      await deed();
      return true;
    } catch (error) {
      setProblem(REFUSALS[error instanceof PollRefusedError ? error.reason : "other"]);
      return false;
    } finally {
      setBusy(false);
    }
  };

  const vote = async () => {
    if (actions === undefined || picked.length === 0) return;
    if (await run(() => actions.vote(messageId, picked))) {
      setChanging(false);
      setPicked([]);
    }
  };

  const end = () => {
    if (actions === undefined) return;
    void confirm({
      title: t("poll.endTitle"),
      description: t("poll.endDescription"),
      confirmLabel: t("poll.end"),
      busyLabel: t("poll.ending"),
      failure: t("poll.refused.other"),
      run: () => actions.end(messageId),
    });
  };

  const showVoters = (answer: PollAnswer) => {
    setShown(answer.id);
    if (actions === undefined || answer.votes === 0) return;
    const known = voterLists.get(answer.id);
    if (known === "loading" || known?.votes === answer.votes) return;
    setVoterLists((lists) => new Map(lists).set(answer.id, "loading"));
    actions.voters(messageId, answer.id).then(
      (page) => setVoterLists((lists) => new Map(lists).set(answer.id, { votes: answer.votes, voters: page.voters })),
      () =>
        setVoterLists((lists) => {
          const next = new Map(lists);
          next.delete(answer.id);
          return next;
        }),
    );
  };

  const kind = closed ? "poll.kind.ended" : poll.multiple ? "poll.kind.multiple" : "poll.kind.single";

  return (
    <div className={css.card}>
      <span className={css.kind}>
        <Icon name="chart" size={14} />
        {t(kind)}
      </span>
      <h4 className={css.question}>{poll.question}</h4>
      <div className={css.list}>
        {view === "vote"
          ? poll.answers.map((answer) => {
              const on = picked.includes(answer.id);
              return (
                <button
                  key={answer.id}
                  type="button"
                  className={on ? `${css.pick} ${css.chosen}` : css.pick}
                  aria-pressed={on}
                  disabled={actions === undefined || busy}
                  onClick={() => setPicked((current) => pickAnswer(current, answer.id, poll.multiple))}
                >
                  <span className={poll.multiple ? `${css.mark} ${css.square}` : css.mark} aria-hidden="true" />
                  <PollEmoji emoji={answer.emoji} customEmoji={customEmoji} />
                  <span className={css.text}>{answer.text}</span>
                </button>
              );
            })
          : poll.answers.map((answer) => {
              const share = shareOf(answer.votes, poll.voters);
              const own = mine.includes(answer.id);
              const classes = [css.result, own && css.mine, leading.has(answer.id) && css.win].filter(Boolean);
              const list = voterLists.get(answer.id);
              const open = shown === answer.id && answer.votes > 0;
              return (
                <div key={answer.id} className={css.resultRow}>
                  <button
                    type="button"
                    className={classes.join(" ")}
                    aria-describedby={open ? tipId : undefined}
                    onMouseEnter={() => showVoters(answer)}
                    onMouseLeave={() => setShown(null)}
                    onFocus={() => showVoters(answer)}
                    onBlur={() => setShown(null)}
                    // A finger has no hover: a tap shows who chose it, another hides it.
                    onClick={() => (open ? setShown(null) : showVoters(answer))}
                  >
                    {own ? (
                      <span className={css.check}>
                        <Icon name="check" size={18} />
                        <span className="sr-only">{t("poll.mine")}</span>
                      </span>
                    ) : (
                      <span className={css.check} aria-hidden="true" />
                    )}
                    <span className={css.text}>
                      <PollEmoji emoji={answer.emoji} customEmoji={customEmoji} />
                      {answer.text}
                    </span>
                    <span className={css.share}>{share}%</span>
                    <span className={css.track} aria-hidden="true">
                      <i style={{ width: `${share}%` }} />
                    </span>
                  </button>
                  {open ? (
                    <span className={css.tip} id={tipId} role="tooltip">
                      {list === undefined || list === "loading" ? (
                        t("poll.voters.loading")
                      ) : (
                        <VoterNames list={list} actions={actions} t={t} locale={locale} />
                      )}
                    </span>
                  ) : null}
                </div>
              );
            })}
      </div>
      <div className={css.foot}>
        <span>{pollCountLabel(poll, locale, t)}</span>
        <span className={css.dot} aria-hidden="true" />
        <span>
          {closed
            ? pollEndedLabel(pollEndedAt(poll), now, locale, t)
            : pollRemainingLabel(poll.endsAt, now, locale, t)}
        </span>
        {view === "ended" || actions === undefined ? null : view === "vote" ? (
          <span className={css.buttons}>
            {changing ? (
              <button type="button" className={css.action} disabled={busy} onClick={() => setChanging(false)}>
                {t("common.cancel")}
              </button>
            ) : (
              <button type="button" className={css.action} disabled={busy} onClick={() => setRevealed(true)}>
                {t("poll.showResults")}
              </button>
            )}
            <button
              type="button"
              className={`${css.action} ${css.primary}`}
              disabled={busy || picked.length === 0}
              onClick={() => void vote()}
            >
              {t("poll.vote")}
            </button>
          </span>
        ) : (
          <span className={css.buttons}>
            <button
              type="button"
              className={css.action}
              disabled={busy}
              onClick={() => {
                // Back to the answers, with the vote already given picked.
                setPicked([...mine]);
                setRevealed(false);
                setChanging(voted);
              }}
            >
              {voted ? t("poll.changeVote") : t("poll.vote")}
            </button>
            {canEnd ? (
              <button type="button" className={css.action} disabled={busy} onClick={end}>
                {t("poll.end")}
              </button>
            ) : null}
          </span>
        )}
      </div>
      {problem === null ? null : (
        <p className={css.problem} role="alert">
          {t(problem)}
        </p>
      )}
    </div>
  );
}

function VoterNames({
  list,
  actions,
  t,
  locale,
}: {
  list: { votes: number; voters: PollVoter[] };
  actions: PollCallbacks | undefined;
  t: ReturnType<typeof useLanguage>["t"];
  locale: ReturnType<typeof useLanguage>["locale"];
}) {
  const named = list.voters.slice(0, NAMED_VOTERS);
  const others = Math.max(0, list.votes - named.length);
  return (
    <>
      <span className={css.faces} aria-hidden="true">
        {named.map((voter) => {
          const face = actions?.faceOf?.(voter.id);
          return (
            <Avatar
              key={voter.id}
              id={voter.id}
              displayName={voter.displayName}
              size="xs"
              {...(face === undefined ? {} : { avatarUrl: face })}
            />
          );
        })}
      </span>
      <span>
        <b>{named.map((voter) => voter.displayName).join(", ")}</b>
        {others === 0 ? null : ` ${t(`poll.voters.others.${pluralForm(locale, others)}` as MessageKey, { count: others })}`}
      </span>
    </>
  );
}
