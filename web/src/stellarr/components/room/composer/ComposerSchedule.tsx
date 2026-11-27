import { useEffect, useId, useRef, useState, type KeyboardEvent, type RefObject } from "react";

import { Button } from "@stellarr/ui";
import { useLayer } from "@stellarr/ui/behavior";
import { Icon } from "@stellarr/icons";
import { trapTab } from "@/components/primitives/focus";
import type { InterfaceLocale, Translator } from "@/i18n";
import { pluralForm } from "@/lib/format/format";
import {
  dayName,
  monthTitle,
  weekdayHeaders,
  whenPhrase,
  zoneName,
} from "@/lib/schedule/scheduleLabels";
import { lastAllowedDay, timeProblem } from "@/lib/schedule/scheduleTime";
import {
  addDays,
  daysBetween,
  instantAt,
  sameDay,
  wallClock,
  weekdayOf,
  type CalendarDay,
} from "@/lib/schedule/zonedTime";

import css from "./ComposerSchedule.module.css";

const systemClock = () => new Date();
/** Labels like "Jutro" turn over at midnight; a minute is close enough. */
const CLOCK_TICK_MS = 30_000;

/** The time, kept current while something on screen depends on it. */
export function useClock(clock: () => Date = systemClock): Date {
  const [now, setNow] = useState(() => clock());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(clock()), CLOCK_TICK_MS);
    return () => window.clearInterval(timer);
  }, [clock]);
  return now;
}

/** A form narrower than this has no room in its row for the time beside the words. */
const ROOM_FOR_TIME_PX = 640;

/**
 * Whether the chosen time waits over the box rather than in its row: on a
 * phone, in a thread beside its channel, in any form too narrow for both.
 */
export function useNarrowForm(form: RefObject<HTMLFormElement | null>): boolean {
  const [narrow, setNarrow] = useState(false);
  useEffect(() => {
    const element = form.current;
    if (element === null || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(([entry]) => {
      if (entry !== undefined) setNarrow(entry.contentRect.width < ROOM_FOR_TIME_PX);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [form]);
  return narrow;
}

export interface WhenChipProps {
  label: string;
  /** The time has passed or is too far: the chip says so until it is changed. */
  stale?: boolean;
  onRemove(): void;
  t: Translator;
}

/** The chosen send time, in the field, with the way to drop it. */
export function WhenChip({ label, stale = false, onRemove, t }: WhenChipProps) {
  return (
    <span className={stale ? `${css.chip} ${css.stale}` : css.chip}>
      <Icon name="clock" size={14} />
      <span>{label}</span>
      <button
        type="button"
        className={css.chipRemove}
        aria-label={t("composer.schedule.remove")}
        title={t("composer.schedule.remove")}
        onClick={onRemove}
      >
        <Icon name="x" size={14} />
      </button>
    </span>
  );
}

export interface ScheduledBarProps {
  count: number;
  next: Date | null;
  place: "channel" | "thread" | "conversation";
  now: Date;
  zone: string;
  locale: InterfaceLocale;
  onShow(): void;
  t: Translator;
}

/**
 * What the author has waiting here, over the field. Only the author sees it:
 * it is drawn from their own list, which nobody else is told about.
 */
export function ScheduledBar({ count, next, place, now, zone, locale, onShow, t }: ScheduledBarProps) {
  const counted = t(`scheduled.bar.count.${pluralForm(locale, count)}`, { count });
  const where = t(`scheduled.bar.place.${place}`);
  return (
    <div className={css.bar}>
      <Icon name="clock" size={15} />
      <span className={css.barText}>
        <b>{counted}</b>{" "}
        {next === null
          ? where
          : t("scheduled.bar.when", { place: where, when: whenPhrase(next, now, zone, locale, t) })}
      </span>
      <button type="button" className={css.barShow} onClick={onShow}>
        {t("scheduled.bar.show")}
      </button>
    </div>
  );
}

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

function readTime(value: string): { hour: number; minute: number } | null {
  const match = /^(\d{2}):(\d{2})$/.exec(value);
  if (match === null) return null;
  const hour = Number(match[1]);
  const minute = Number(match[2]);
  return hour < 24 && minute < 60 ? { hour, minute } : null;
}

export interface SchedulePickerProps {
  now: Date;
  zone: string;
  locale: InterfaceLocale;
  /** The time it opens on; tomorrow at 9:00 without one. */
  initial?: Date | null;
  /** Where it stands: over the composer, or over a row of the list. */
  className?: string;
  onCancel(): void;
  onConfirm(at: Date): void;
  t: Translator;
}

/**
 * A day from a month grid and a time, in the reader's zone. Days before
 * today and past the 30-day limit cannot be picked; the line under the time
 * says when the message will go out, or why it cannot.
 *
 * It is modal: Tab stays in it (the shell keeps Tab from walking the page),
 * and Escape or a press beside it cancels.
 */
export function SchedulePicker({
  now,
  zone,
  locale,
  initial = null,
  className,
  onCancel,
  onConfirm,
  t,
}: SchedulePickerProps) {
  const today = wallClock(now, zone);
  const last = lastAllowedDay(now, zone);
  const start = initial === null ? null : wallClock(initial, zone);
  const [day, setDay] = useState<CalendarDay>(start ?? addDays(today, 1));
  const [view, setView] = useState({ year: (start ?? addDays(today, 1)).year, month: (start ?? addDays(today, 1)).month });
  const [time, setTime] = useState(start === null ? "09:00" : `${pad(start.hour)}:${pad(start.minute)}`);
  const titleId = useId();
  const timeId = useId();
  const panel = useRef<HTMLDivElement>(null);
  const grid = useRef<HTMLDivElement>(null);

  useLayer(true, {
    node: () => panel.current,
    onEscape: (event) => {
      event.preventDefault();
      onCancel();
    },
    onTab: (event) => {
      if (panel.current !== null) trapTab(event, panel.current);
    },
    onPointerDownOutside: () => onCancel(),
  });

  // The chosen day has the keyboard first.
  useEffect(() => {
    grid.current?.querySelector<HTMLButtonElement>('button[tabindex="0"]')?.focus();
    // Once, when the picker opens.
  }, []);

  const allowed = (candidate: CalendarDay) =>
    daysBetween(today, candidate) >= 0 && daysBetween(candidate, last) >= 0;
  const clock = readTime(time);
  const at = clock === null ? null : instantAt(day, clock, zone);
  const problem = at === null ? "unknown" : timeProblem(at, now);

  const firstOfView = { year: view.year, month: view.month, day: 1 };
  const lead = weekdayOf(firstOfView) - 1;
  const length = daysBetween(firstOfView, { year: view.month === 12 ? view.year + 1 : view.year, month: (view.month % 12) + 1, day: 1 });
  const cells: (CalendarDay | null)[] = [
    ...Array.from({ length: lead }, () => null),
    ...Array.from({ length }, (_, index) => addDays(firstOfView, index)),
  ];
  const shiftView = (by: number) => {
    const moved = addDays({ year: view.year, month: view.month, day: 15 }, by * 30);
    setView({ year: moved.year, month: moved.month });
  };
  const canGoBack = view.year > today.year || (view.year === today.year && view.month > today.month);
  const canGoOn = view.year < last.year || (view.year === last.year && view.month < last.month);

  const choose = (candidate: CalendarDay) => {
    if (!allowed(candidate)) return;
    setDay(candidate);
    if (candidate.month !== view.month || candidate.year !== view.year) {
      setView({ year: candidate.year, month: candidate.month });
    }
    requestAnimationFrame(() =>
      grid.current?.querySelector<HTMLButtonElement>('button[tabindex="0"]')?.focus(),
    );
  };

  // Arrows walk the days, as in any calendar grid; only allowed days take them.
  const onGridKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 }[event.key];
    if (step === undefined) return;
    event.preventDefault();
    choose(addDays(day, step));
  };

  return (
    <div
      ref={panel}
      className={className === undefined ? css.picker : `${css.picker} ${className}`}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
    >
      <div className={css.pickerHead}>
        <span id={titleId}>{monthTitle(view.year, view.month, locale)}</span>
        <span className={css.pickerNav}>
          <button
            type="button"
            aria-label={t("composer.schedule.previousMonth")}
            disabled={!canGoBack}
            onClick={() => shiftView(-1)}
          >
            <Icon name="caretL" size={16} />
          </button>
          <button
            type="button"
            aria-label={t("composer.schedule.nextMonth")}
            disabled={!canGoOn}
            onClick={() => shiftView(1)}
          >
            <Icon name="caretR" size={16} />
          </button>
        </span>
      </div>
      <div ref={grid} className={css.grid} role="group" aria-labelledby={titleId} onKeyDown={onGridKey}>
        {weekdayHeaders(t).map((name) => (
          <i key={name} aria-hidden="true">
            {name}
          </i>
        ))}
        {cells.map((cell, index) =>
          cell === null ? (
            <b key={`blank-${index}`} aria-hidden="true" />
          ) : (
            <button
              key={`${cell.month}-${cell.day}`}
              type="button"
              className={css.day}
              aria-label={dayName(cell, locale)}
              aria-pressed={sameDay(cell, day)}
              aria-current={sameDay(cell, today) ? "date" : undefined}
              disabled={!allowed(cell)}
              tabIndex={sameDay(cell, day) ? 0 : -1}
              onClick={() => choose(cell)}
            >
              {cell.day}
            </button>
          ),
        )}
      </div>
      <div className={css.timeRow}>
        <label className="sr-only" htmlFor={timeId}>
          {t("composer.schedule.time")}
        </label>
        <input
          id={timeId}
          className={css.time}
          type="time"
          step={60}
          required
          value={time}
          onChange={(event) => setTime(event.target.value)}
        />
        <span className={css.zone}>{zoneName(zone)}</span>
      </div>
      <p className={problem === null ? css.note : `${css.note} ${css.problem}`} aria-live="polite">
        {problem === null && at !== null
          ? `${t("composer.schedule.summary", { when: whenPhrase(at, now, zone, locale, t) })} ${t("composer.schedule.limit")}`
          : problem === "tooFar"
            ? t("composer.schedule.tooFar")
            : problem === "unknown"
              ? t("composer.schedule.noTime")
              : t("composer.schedule.passed")}
      </p>
      <div className={css.pickerFoot}>
        <Button type="button" variant="secondary" onClick={onCancel}>
          {t("common.cancel")}
        </Button>
        <Button
          type="button"
          variant="primary"
          disabled={problem !== null || at === null}
          onClick={() => {
            if (at !== null && problem === null) onConfirm(at);
          }}
        >
          {t("composer.schedule.confirm")}
        </Button>
      </div>
    </div>
  );
}
