import { useId, useRef, useState, type RefObject } from "react";

import { Button, cx, Dropdown } from "@stellarr/ui";
import { useLayer } from "@stellarr/ui/behavior";
import { Icon } from "@stellarr/icons";
import { trapTab } from "@/components/primitives/focus";
import { Modal } from "@/components/primitives/Modal";
import { MonthGrid } from "@/components/room/composer/ComposerSchedule";
import { locales, type InterfaceLocale, type MessageKey, type Translator } from "@/i18n";
import { addDays, instantAt, wallClock, type CalendarDay } from "@/lib/schedule/zonedTime";
import {
  EVENT_LIMITS,
  EVENT_REPEATS,
  EventDraftRefusedError,
  EventRefusedError,
  type EventDraft,
  type EventDraftField,
  type EventRefusalReason,
  type EventRepeat,
} from "@/ports";

import css from "./EventSheet.module.css";

interface Channel {
  id: string;
  name: string;
}

export interface EventSheetProps {
  serverName: string;
  voiceChannels: readonly Channel[];
  textChannels: readonly Channel[];
  now: Date;
  /** The reader's zone: the times are written in it and sent with it. */
  zone: string;
  /** The event being changed; without it a new one is written. */
  initial?: EventDraft;
  /** The text channel it is offered to announce on first. */
  announceOn?: string;
  /** Already announced: it is announced once, so the choice is not offered again. */
  announced?: boolean;
  /** Rejects with an `EventRefusedError` when the server refuses. */
  onSave(draft: EventDraft): Promise<void>;
  onClose(): void;
  locale: InterfaceLocale;
  t: Translator;
}

const NO_ANNOUNCEMENT = "";
const DEFAULT_START = "20:00";

const REPEAT_LABELS: Record<EventRepeat, MessageKey> = {
  none: "composer.event.repeat.none",
  weekly: "composer.event.repeat.weekly",
  biweekly: "composer.event.repeat.biweekly",
  monthly: "composer.event.repeat.monthly",
};

const REFUSALS: Record<EventRefusalReason, MessageKey> = {
  over: "event.refused.over",
  live: "composer.event.problem.live",
  moved: "composer.event.problem.other",
  gone: "event.refused.gone",
  forbidden: "composer.event.problem.forbidden",
  limit: "composer.event.problem.limit",
  other: "composer.event.problem.other",
};

/** What a refused field means, said under the field. */
const FIELD_PROBLEMS: Record<EventDraftField, MessageKey> = {
  name: "composer.event.problem.name",
  description: "composer.event.problem.description",
  place: "composer.event.problem.place",
  startsAt: "composer.event.problem.start",
  endsAt: "composer.event.problem.end",
  seats: "composer.event.problem.seats",
  announce: "composer.event.problem.announce",
};

type Problems = Partial<Record<EventDraftField | "general", MessageKey>>;

const pad = (value: number) => String(value).padStart(2, "0");

function readTime(value: string): { hour: number; minute: number } | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(value);
  if (match === null) return null;
  const hour = Number(match[1]);
  const minute = Number(match[2]);
  return hour < 24 && minute < 60 ? { hour, minute } : null;
}

/** "sob., 28 listopada": the day on its field. */
function dayLabel(day: CalendarDay, locale: InterfaceLocale): string {
  return new Intl.DateTimeFormat(locales[locale].tag, {
    timeZone: "UTC",
    weekday: "short",
    day: "numeric",
    month: "long",
  }).format(new Date(Date.UTC(day.year, day.month - 1, day.day, 12)));
}

/**
 * A new event, or a change to one: its name, where it happens, when, what it
 * is about, how many may sign up, whether it repeats and where it is
 * announced. An end at or before the start is on the next day.
 */
export function EventSheet({
  serverName,
  voiceChannels,
  textChannels,
  now,
  zone,
  initial,
  announceOn,
  announced = false,
  onSave,
  onClose,
  locale,
  t,
}: EventSheetProps) {
  const titleId = useId();
  const nameId = useId();
  const channelId = useId();
  const placeId = useId();
  const startId = useId();
  const endId = useId();
  const descriptionId = useId();
  const seatsId = useId();
  const announceId = useId();
  const today = wallClock(now, zone);
  const startClock = initial === undefined ? null : wallClock(initial.startsAt, zone);
  const endClock = initial?.endsAt === undefined ? null : wallClock(initial.endsAt, zone);

  const [name, setName] = useState(initial?.name ?? "");
  const [placeKind, setPlaceKind] = useState<"voice" | "elsewhere">(
    initial?.place.kind ?? (voiceChannels.length > 0 ? "voice" : "elsewhere"),
  );
  const [voiceChannel, setVoiceChannel] = useState(
    initial?.place.kind === "voice" ? initial.place.channelId : (voiceChannels[0]?.id ?? ""),
  );
  const [placeText, setPlaceText] = useState(initial?.place.kind === "elsewhere" ? initial.place.text : "");
  const [day, setDay] = useState<CalendarDay>(startClock ?? addDays(today, 1));
  const [start, setStart] = useState(startClock === null ? DEFAULT_START : `${pad(startClock.hour)}:${pad(startClock.minute)}`);
  const [end, setEnd] = useState(endClock === null ? "" : `${pad(endClock.hour)}:${pad(endClock.minute)}`);
  const [description, setDescription] = useState(initial?.description ?? "");
  const [limited, setLimited] = useState(initial?.seats !== undefined);
  const [seats, setSeats] = useState(String(initial?.seats ?? 10));
  const [repeat, setRepeat] = useState<EventRepeat>(initial?.repeat ?? "none");
  const [announce, setAnnounce] = useState(
    initial === undefined ? (announceOn ?? NO_ANNOUNCEMENT) : (initial.announceChannelId ?? NO_ANNOUNCEMENT),
  );
  const [calendarOpen, setCalendarOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [problems, setProblems] = useState<Problems>({});
  const dateButton = useRef<HTMLButtonElement>(null);

  const startTime = readTime(start);
  const endTime = end === "" ? null : readTime(end);
  const startsAt = startTime === null ? null : instantAt(day, startTime, zone);
  // An end at or before the start is the next day's: an evening into the night.
  const endsNextDay = startTime !== null && endTime !== null && endTime.hour * 60 + endTime.minute <= startTime.hour * 60 + startTime.minute;
  const endsAt = endTime === null ? null : instantAt(endsNextDay ? addDays(day, 1) : day, endTime, zone);
  const seatCount = Number(seats);
  const seatsValid = !limited || (Number.isInteger(seatCount) && seatCount >= 1 && seatCount <= EVENT_LIMITS.maxSeats);
  const placeReady = placeKind === "voice" ? voiceChannel !== "" : placeText.trim() !== "";
  // Somewhere else has no channel to empty, so it needs an end of its own.
  const endReady = placeKind === "voice" || endsAt !== null;
  // A change that keeps the time may be saved while it runs; a new time must be ahead.
  const timeKept =
    initial !== undefined &&
    startsAt?.getTime() === initial.startsAt.getTime() &&
    endsAt?.getTime() === initial.endsAt?.getTime();
  const past = !timeKept && startsAt !== null && startsAt.getTime() <= now.getTime();
  const ready = name.trim() !== "" && placeReady && startsAt !== null && !past && endReady && seatsValid && !saving;

  /** What the server said of one field, under it. */
  const said = (field: EventDraftField) => {
    const problem = problems[field];
    return problem === undefined ? null : <p className={`${css.hint} ${css.problem}`}>{t(problem)}</p>;
  };

  const save = async () => {
    if (!ready || startsAt === null) return;
    setSaving(true);
    setProblems({});
    try {
      await onSave({
        name: name.trim(),
        place: placeKind === "voice" ? { kind: "voice", channelId: voiceChannel } : { kind: "elsewhere", text: placeText.trim() },
        startsAt,
        ...(endsAt === null ? {} : { endsAt }),
        timeZone: zone,
        ...(description.trim() === "" ? {} : { description: description.trim() }),
        ...(limited ? { seats: seatCount } : {}),
        repeat,
        ...(announce === NO_ANNOUNCEMENT ? {} : { announceChannelId: announce }),
      });
      onClose();
    } catch (error) {
      setProblems(
        error instanceof EventDraftRefusedError
          ? Object.fromEntries(error.fields.map((field) => [field, FIELD_PROBLEMS[field]]))
          : { general: REFUSALS[error instanceof EventRefusedError ? error.reason : "other"] },
      );
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal open onClose={onClose} labelledBy={titleId} className={cx(css.wrap)} exitMs={200}>
      <form
        className={css.sheet}
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <h2 id={titleId}>{t(initial === undefined ? "composer.event.title" : "composer.event.editTitle")}</h2>
        <p className={css.sub}>{t("composer.event.onServer", { server: serverName })}</p>

        <div className={css.field}>
          <label className={css.label} htmlFor={nameId}>
            {t("composer.event.name")}
          </label>
          <input
            id={nameId}
            className={css.input}
            value={name}
            maxLength={EVENT_LIMITS.name}
            placeholder={t("composer.event.namePlaceholder")}
            // Opened to be written in.
            autoFocus
            onChange={(event) => setName(event.target.value)}
          />
          {said("name")}
        </div>

        <div className={css.field} role="group" aria-labelledby={`${placeId}-label`}>
          <span className={css.label} id={`${placeId}-label`}>
            {t("composer.event.where")}
          </span>
          <div className={css.where}>
            {(["voice", "elsewhere"] as const).map((kind) => (
              <button
                key={kind}
                type="button"
                className={placeKind === kind ? `${css.option} ${css.on}` : css.option}
                aria-pressed={placeKind === kind}
                onClick={() => setPlaceKind(kind)}
              >
                <span className={css.dot} aria-hidden="true" />
                <span className={css.optionText}>
                  <b>{t(kind === "voice" ? "composer.event.voice" : "composer.event.elsewhere")}</b>
                  <span>{t(kind === "voice" ? "composer.event.voiceDetail" : "composer.event.elsewhereDetail")}</span>
                </span>
              </button>
            ))}
          </div>
          {placeKind === "voice" ? (
            voiceChannels.length === 0 ? (
              <p className={css.hint}>{t("composer.event.noVoiceChannels")}</p>
            ) : (
              <Dropdown
                id={channelId}
                className={css.full}
                label={t("composer.event.channel")}
                options={voiceChannels.map((channel) => ({ value: channel.id, label: channel.name }))}
                value={voiceChannel}
                onChange={setVoiceChannel}
              />
            )
          ) : (
            <input
              id={placeId}
              className={css.input}
              value={placeText}
              maxLength={EVENT_LIMITS.place}
              aria-label={t("composer.event.place")}
              placeholder={t("composer.event.placePlaceholder")}
              onChange={(event) => setPlaceText(event.target.value)}
            />
          )}
          {said("place")}
        </div>

        <div className={css.field} role="group" aria-labelledby={`${startId}-label`}>
          <span className={css.label} id={`${startId}-label`}>
            {t("composer.event.when")}
          </span>
          <div className={css.when}>
            <button
              ref={dateButton}
              type="button"
              className={`${css.input} ${css.select}`}
              aria-label={`${t("composer.event.date")}: ${dayLabel(day, locale)}`}
              aria-haspopup="dialog"
              aria-expanded={calendarOpen}
              onClick={() => setCalendarOpen((open) => !open)}
            >
              <span className={css.selectText}>{dayLabel(day, locale)}</span>
              <Icon name="calendar" size={16} />
            </button>
            <div className={css.times}>
              <input
                id={startId}
                className={css.input}
                type="time"
                step={60}
                required
                aria-label={t("composer.event.start")}
                value={start}
                onChange={(event) => setStart(event.target.value)}
              />
              <span className={css.until}>
                <label htmlFor={endId}>{t("composer.event.until")}</label>
                <input
                  id={endId}
                  className={css.input}
                  type="time"
                  step={60}
                  aria-label={t("composer.event.end")}
                  value={end}
                  onChange={(event) => setEnd(event.target.value)}
                />
              </span>
            </div>
            {calendarOpen ? (
              <DayPicker
                anchor={dateButton}
                today={today}
                value={day}
                onChoose={(chosen) => {
                  setDay(chosen);
                  setCalendarOpen(false);
                  dateButton.current?.focus();
                }}
                onClose={() => setCalendarOpen(false)}
                locale={locale}
                t={t}
              />
            ) : null}
          </div>
          <span className={past || !endReady ? `${css.hint} ${css.problem}` : css.hint}>
            {[
              past
                ? t("composer.event.problem.past")
                : !endReady
                  ? t("composer.event.endRequired")
                  : placeKind === "voice"
                    ? t("composer.event.endHint")
                    : "",
              endsNextDay ? t("composer.event.nextDay") : "",
            ]
              .filter((part) => part !== "")
              .join(" ")}
          </span>
          {said("startsAt")}
          {said("endsAt")}
        </div>

        <div className={css.field}>
          <label className={css.label} htmlFor={descriptionId}>
            {t("composer.event.description")} <i>{t("composer.event.optional")}</i>
          </label>
          <textarea
            id={descriptionId}
            className={`${css.input} ${css.area}`}
            rows={3}
            value={description}
            maxLength={EVENT_LIMITS.description}
            onChange={(event) => setDescription(event.target.value)}
          />
          {said("description")}
        </div>

        <div className={css.field} role="group" aria-labelledby={`${seatsId}-label`}>
          <span className={css.label} id={`${seatsId}-label`}>
            {t("composer.event.seats")}
          </span>
          <div className={css.seatsRow}>
            <div className={css.chips}>
              <button
                type="button"
                className={limited ? css.chip : `${css.chip} ${css.on}`}
                aria-pressed={!limited}
                onClick={() => setLimited(false)}
              >
                {t("composer.event.noLimit")}
              </button>
              <button
                type="button"
                className={limited ? `${css.chip} ${css.on}` : css.chip}
                aria-pressed={limited}
                onClick={() => setLimited(true)}
              >
                {t("composer.event.limit")}
              </button>
            </div>
            {limited ? (
              <>
                <input
                  id={seatsId}
                  className={`${css.input} ${css.seats}`}
                  type="number"
                  inputMode="numeric"
                  min={1}
                  max={EVENT_LIMITS.maxSeats}
                  aria-label={t("composer.event.seatCount")}
                  aria-invalid={seatsValid ? undefined : true}
                  value={seats}
                  onChange={(event) => setSeats(event.target.value)}
                />
                <span className={css.hint}>{t("composer.event.seatsUnit")}</span>
              </>
            ) : null}
          </div>
          {limited ? <span className={css.hint}>{t("composer.event.seatsHint")}</span> : null}
          {said("seats")}
        </div>

        <div className={css.field} role="group" aria-labelledby={`${seatsId}-repeat`}>
          <span className={css.label} id={`${seatsId}-repeat`}>
            {t("composer.event.repeat")}
          </span>
          <div className={css.chips}>
            {EVENT_REPEATS.map((value) => (
              <button
                key={value}
                type="button"
                className={repeat === value ? `${css.chip} ${css.on}` : css.chip}
                aria-pressed={repeat === value}
                onClick={() => setRepeat(value)}
              >
                {t(REPEAT_LABELS[value])}
              </button>
            ))}
          </div>
        </div>

        {announced ? null : (
          <div className={css.field}>
            <label className={css.label} htmlFor={announceId}>
              {t("composer.event.announce")} <i>{t("composer.event.optional")}</i>
            </label>
            <Dropdown
              id={announceId}
              className={css.full}
              options={[
                { value: NO_ANNOUNCEMENT, label: t("composer.event.announceNone") },
                ...textChannels.map((channel) => ({ value: channel.id, label: `# ${channel.name}` })),
              ]}
              value={announce}
              onChange={setAnnounce}
            />
            {said("announce")}
          </div>
        )}

        {problems.general === undefined ? null : (
          <p className={`${css.hint} ${css.problem}`} role="alert">
            {t(problems.general)}
          </p>
        )}

        <div className={css.foot}>
          <Button type="button" variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button type="submit" variant="primary" disabled={!ready}>
            {t(initial === undefined ? "composer.event.create" : "composer.event.save")}
          </Button>
        </div>
      </form>
    </Modal>
  );
}

interface DayPickerProps {
  /** The date field it opened from. */
  anchor: RefObject<HTMLElement | null>;
  today: CalendarDay;
  value: CalendarDay;
  onChoose(day: CalendarDay): void;
  onClose(): void;
  locale: InterfaceLocale;
  t: Translator;
}

/** The month grid under the date field, a layer of its own so Escape closes it and not the sheet. */
function DayPicker({ anchor, today, value, onChoose, onClose, locale, t }: DayPickerProps) {
  const panel = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const [walked, setWalked] = useState(value);
  useLayer(true, {
    node: () => panel.current,
    onEscape: (event) => {
      event.preventDefault();
      onClose();
    },
    onTab: (event) => {
      if (panel.current !== null) trapTab(event, panel.current);
    },
    // Its own field closes it with its click; closing here would open it again.
    onPointerDownOutside: (event) => {
      if (!(event.target instanceof Node && anchor.current?.contains(event.target))) onClose();
    },
  });
  return (
    <div ref={panel} className={css.calendar} role="dialog" aria-modal="true" aria-labelledby={titleId}>
      <MonthGrid
        today={today}
        first={today}
        last={addDays(today, EVENT_LIMITS.maxDaysAhead)}
        value={walked}
        onChange={setWalked}
        onPick={onChoose}
        titleId={titleId}
        autoFocus
        locale={locale}
        t={t}
      />
    </div>
  );
}
