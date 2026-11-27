import { useId, useState } from "react";

import { Button } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { SchedulePicker } from "@/components/room/composer/ComposerSchedule";
import { SCHEDULE_REFUSALS } from "@/components/room/composer/useComposerDraft";
import type { MessageKey } from "@/i18n";
import { pluralForm } from "@/lib/format/format";
import { clockLabel, dayHeading, failedTimeLabel } from "@/lib/schedule/scheduleLabels";
import { wallClock, type CalendarDay } from "@/lib/schedule/zonedTime";
import {
  ScheduleRefusedError,
  type ScheduledFailureCode,
  type ScheduledMessage,
} from "@/ports";
import { useLanguage } from "@/state/LanguageProvider";

import css from "./ScheduledList.module.css";

/** Where a scheduled message goes, as the list names it. */
export interface ScheduledTargetLabel {
  text: string;
  /** A conversation's person, drawn small before the name. */
  avatarUrl?: string;
}

export interface ScheduledListActions {
  onSendNow(id: string): Promise<unknown>;
  onEdit(id: string, content: string): Promise<unknown>;
  onReschedule(id: string, at: Date): Promise<unknown>;
  /** Asks first; resolves once it is gone or the question was dismissed. */
  onDelete(id: string): Promise<unknown>;
  onCopy(content: string): Promise<unknown>;
}

export interface ScheduledListProps extends ScheduledListActions {
  items: readonly ScheduledMessage[];
  now: Date;
  zone: string;
  targetOf(item: ScheduledMessage): ScheduledTargetLabel;
  /** Present where the list is a dialog of its own, absent inside the inbox. */
  onClose?: () => void;
  /** The id of the heading, for a dialog that is named by it. */
  titleId?: string;
  /** Inside the inbox, whose tab already names the list: only the count line. */
  compact?: boolean;
}

const FAILURES: Record<ScheduledFailureCode, MessageKey> = {
  no_access: "scheduled.failure.no_access",
  target_gone: "scheduled.failure.target_gone",
  thread_gone: "scheduled.failure.thread_gone",
  encrypted: "scheduled.failure.encrypted",
  other: "scheduled.failure.other",
};

interface Group {
  key: string;
  day: CalendarDay | null;
  items: ScheduledMessage[];
}

/** Waiting ones by the day they go out, soonest first; the ones that failed last. */
function groupsOf(items: readonly ScheduledMessage[], zone: string): Group[] {
  const groups: Group[] = [];
  const failed: ScheduledMessage[] = [];
  for (const item of items) {
    if (item.state === "failed") {
      failed.push(item);
      continue;
    }
    const day = wallClock(new Date(item.sendAt), zone);
    const key = `${day.year}-${day.month}-${day.day}`;
    const last = groups[groups.length - 1];
    if (last?.key === key) last.items.push(item);
    else groups.push({ key, day, items: [item] });
  }
  if (failed.length > 0) groups.push({ key: "failed", day: null, items: failed });
  return groups;
}

/**
 * The author's scheduled messages: when each goes out, what it says and
 * where, with the ways to send it now, change it or drop it. Those that did
 * not go out say why, and their words can be copied to try again.
 */
export function ScheduledList({
  items,
  now,
  zone,
  targetOf,
  onClose,
  titleId: givenTitleId,
  compact = false,
  onSendNow,
  onEdit,
  onReschedule,
  onDelete,
  onCopy,
}: ScheduledListProps) {
  const { t, locale } = useLanguage();
  const ownTitleId = useId();
  const titleId = givenTitleId ?? ownTitleId;
  const [editing, setEditing] = useState<{ id: string; content: string } | null>(null);
  const [rescheduling, setRescheduling] = useState<ScheduledMessage | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);

  const waiting = items.filter((item) => item.state === "pending").length;
  const failed = items.length - waiting;
  const summary = [
    waiting === 0 ? null : t(`scheduled.waiting.${pluralForm(locale, waiting)}`, { count: waiting }),
    failed === 0 ? null : t(`scheduled.failedCount.${pluralForm(locale, failed)}`, { count: failed }),
  ]
    .filter((part) => part !== null)
    .join(", ");

  /** One action at a time per message; a refusal says why above the list. */
  const act = (id: string, work: () => Promise<unknown>) => {
    setBusy(id);
    setError(null);
    work()
      .catch((cause: unknown) =>
        setError(t(cause instanceof ScheduleRefusedError ? SCHEDULE_REFUSALS[cause.reason] : "scheduled.actionFailed")),
      )
      .finally(() => setBusy((current) => (current === id ? null : current)));
  };

  const row = (item: ScheduledMessage) => {
    const at = new Date(item.sendAt);
    const isFailed = item.state === "failed";
    const target = targetOf(item);
    const files = item.attachments.length;
    const meta = [
      target.text,
      files === 0 ? null : t(`scheduled.attachments.${pluralForm(locale, files)}`, { count: files }),
      isFailed ? t(FAILURES[item.failure?.code ?? "other"]) : null,
    ]
      .filter((part) => part !== null)
      .join(" · ");
    const isEditing = editing?.id === item.id;
    const working = busy === item.id;
    return (
      <li key={item.id} className={isFailed ? css.item : `${css.item} ${css.waiting}`} aria-busy={working || undefined}>
        <span className={isFailed ? `${css.time} ${css.failedTime}` : css.time}>
          {isFailed ? failedTimeLabel(at, zone, locale, t) : clockLabel(at, zone, locale)}
        </span>
        <span className={css.body}>
          {isEditing ? (
            <form
              className={css.edit}
              onSubmit={(event) => {
                event.preventDefault();
                const content = editing.content.trim();
                if (content === "" && files === 0) return;
                act(item.id, async () => {
                  await onEdit(item.id, content);
                  setEditing(null);
                });
              }}
            >
              <textarea
                aria-label={t("scheduled.editLabel")}
                value={editing.content}
                rows={3}
                // Opened on purpose to be typed into.
                autoFocus
                onChange={(event) => setEditing({ id: item.id, content: event.target.value })}
              />
              <span className={css.editActions}>
                <Button type="button" variant="secondary" onClick={() => setEditing(null)}>
                  {t("common.cancel")}
                </Button>
                <Button type="submit" variant="primary" disabled={working}>
                  {t("common.save")}
                </Button>
              </span>
            </form>
          ) : (
            <q className={css.text}>{item.content.trim() === "" ? t("scheduled.onlyFiles") : item.content}</q>
          )}
          <span className={isFailed ? `${css.meta} ${css.bad}` : css.meta}>
            {target.avatarUrl === undefined ? null : (
              <img src={target.avatarUrl} alt="" width={16} height={16} className={css.avatar} />
            )}
            {meta}
          </span>
        </span>
        <span className={css.actions}>
          {isFailed ? (
            <>
              {/* Only a failure with no lasting cause may go out on a second try. */}
              {item.failure?.code !== "other" ? null : (
                <button
                  type="button"
                  className={css.iconAction}
                  aria-label={t("scheduled.sendNow")}
                  title={t("scheduled.sendNow")}
                  disabled={working}
                  onClick={() => act(item.id, () => onSendNow(item.id))}
                >
                  <Icon name="send" size={16} />
                </button>
              )}
              <button
                type="button"
                className={css.textAction}
                onClick={() =>
                  act(item.id, async () => {
                    await onCopy(item.content);
                    setCopied(item.id);
                  })
                }
              >
                {copied === item.id ? t("scheduled.copied") : t("scheduled.copy")}
              </button>
              <button
                type="button"
                className={css.iconAction}
                aria-label={t("scheduled.delete")}
                title={t("scheduled.delete")}
                disabled={working}
                onClick={() => act(item.id, () => onDelete(item.id))}
              >
                <Icon name="trash" size={16} />
              </button>
            </>
          ) : (
            <>
              <button
                type="button"
                className={css.iconAction}
                aria-label={t("scheduled.sendNow")}
                title={t("scheduled.sendNow")}
                disabled={working}
                onClick={() => act(item.id, () => onSendNow(item.id))}
              >
                <Icon name="send" size={16} />
              </button>
              <button
                type="button"
                className={css.iconAction}
                aria-label={t("scheduled.edit")}
                title={t("scheduled.edit")}
                disabled={working}
                onClick={() => setEditing({ id: item.id, content: item.content })}
              >
                <Icon name="edit" size={16} />
              </button>
              <button
                type="button"
                className={css.iconAction}
                aria-label={t("scheduled.reschedule")}
                title={t("scheduled.reschedule")}
                disabled={working}
                onClick={() => setRescheduling(item)}
              >
                <Icon name="clock" size={16} />
              </button>
              <button
                type="button"
                className={css.iconAction}
                aria-label={t("scheduled.delete")}
                title={t("scheduled.delete")}
                disabled={working}
                onClick={() => act(item.id, () => onDelete(item.id))}
              >
                <Icon name="trash" size={16} />
              </button>
              {/* What a row that is not pointed at shows in place of its actions. */}
              <span className={css.rest} aria-hidden="true">
                <Icon name="dots" size={16} />
              </span>
            </>
          )}
        </span>
      </li>
    );
  };

  return (
    <div className={css.list}>
      <div className={compact ? `${css.head} ${css.compactHead}` : css.head}>
        <div>
          {compact ? null : <h2 id={titleId}>{t("scheduled.title")}</h2>}
          <p>{summary === "" ? t("scheduled.none") : summary}</p>
        </div>
        {onClose === undefined ? null : (
          <button
            type="button"
            className={css.close}
            aria-label={t("common.close")}
            title={t("common.close")}
            onClick={onClose}
          >
            <Icon name="x" size={16} />
          </button>
        )}
      </div>
      {error === null ? null : (
        <p className={css.error} role="alert">
          {error}
        </p>
      )}
      {items.length === 0 ? (
        <p className={css.empty}>{t("scheduled.empty")}</p>
      ) : (
        <div className={css.scroll}>
          {groupsOf(items, zone).map((group) => {
            const headingId = `${titleId}-${group.key}`;
            return (
              <section key={group.key} aria-labelledby={headingId}>
                <h3 id={headingId} className={css.day}>
                  {group.day === null ? t("scheduled.failedGroup") : dayHeading(group.day, now, zone, locale, t)}
                </h3>
                <ul>{group.items.map(row)}</ul>
              </section>
            );
          })}
        </div>
      )}
      {rescheduling === null ? null : (
        <SchedulePicker
          now={now}
          zone={zone}
          locale={locale}
          initial={new Date(rescheduling.sendAt)}
          className={css.picker}
          onCancel={() => setRescheduling(null)}
          onConfirm={(at) => {
            const { id } = rescheduling;
            setRescheduling(null);
            act(id, () => onReschedule(id, at));
          }}
          t={t}
        />
      )}
    </div>
  );
}
