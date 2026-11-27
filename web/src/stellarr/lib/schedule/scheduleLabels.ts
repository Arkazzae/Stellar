import { locales, type InterfaceLocale, type Translator } from "@/i18n";

import type { SchedulePreset } from "./scheduleTime";
import { daysBetween, wallClock, type CalendarDay } from "./zonedTime";

/*
 * How send times read: on the chip, in the menu, in the bar above the field
 * and in the list. The dates and times themselves come from Intl in the
 * reader's language and zone; the words around them come from the catalogs.
 */

function format(locale: InterfaceLocale, zone: string, options: Intl.DateTimeFormatOptions, at: Date): string {
  return new Intl.DateTimeFormat(locales[locale].tag, { timeZone: zone, ...options }).format(at);
}

function capital(text: string, locale: InterfaceLocale): string {
  return text.charAt(0).toLocaleUpperCase(locales[locale].tag) + text.slice(1);
}

/**
 * Two letters for a weekday, 1 for Monday. Intl's short names differ in
 * length and some end in a full stop, so each language names them itself.
 */
function shortWeekday(weekday: number, t: Translator): string {
  return t(`scheduled.weekdayShort.${weekday}` as Parameters<Translator>[0]);
}

/** A calendar day as an instant at its noon in UTC, for formatting the day alone. */
const noonOf = (day: CalendarDay) => new Date(Date.UTC(day.year, day.month - 1, day.day, 12));

/** "9:00", as the board writes it: some languages' own pattern pads the hour to "09:00". */
export function clockLabel(at: Date, zone: string, locale: InterfaceLocale): string {
  return new Intl.DateTimeFormat(locales[locale].tag, { timeZone: zone, hour: "numeric", minute: "2-digit" })
    .formatToParts(at)
    .map((part) => (part.type === "hour" ? String(Number(part.value)) : part.value))
    .join("");
}

/** How many calendar days ahead `at` is in `zone`: 0 today, 1 tomorrow. */
export function daysAhead(at: Date, now: Date, zone: string): number {
  return daysBetween(wallClock(now, zone), wallClock(at, zone));
}

/** "Today, 18:00", "Tomorrow, 9:00", "Monday, 9:00", "12 Dec, 9:00": the chip and the menu. */
export function whenLabel(at: Date, now: Date, zone: string, locale: InterfaceLocale, t: Translator): string {
  const time = clockLabel(at, zone, locale);
  const ahead = daysAhead(at, now, zone);
  if (ahead === 0) return t("scheduled.label.today", { time });
  if (ahead === 1) return t("scheduled.label.tomorrow", { time });
  const day =
    ahead < 7
      ? capital(format(locale, zone, { weekday: "long" }, at), locale)
      : format(locale, zone, { day: "numeric", month: "short" }, at);
  return t("scheduled.label.day", { day, time });
}

/** The menu's row for a preset, with the short date it shows on the right. */
export function presetRow(
  preset: SchedulePreset,
  now: Date,
  zone: string,
  locale: InterfaceLocale,
  t: Translator,
): { label: string; detail?: string } {
  const label = whenLabel(preset.at, now, zone, locale, t);
  if (preset.id === "tomorrow") return { label, detail: shortWeekday(wallClock(preset.at, zone).weekday, t) };
  if (preset.id === "monday") return { label, detail: format(locale, zone, { day: "numeric", month: "short" }, preset.at) };
  return { label };
}

/** "tomorrow at 9:00", "on Monday, 30 November at 9:00": inside a sentence. */
export function whenPhrase(at: Date, now: Date, zone: string, locale: InterfaceLocale, t: Translator): string {
  const time = clockLabel(at, zone, locale);
  const ahead = daysAhead(at, now, zone);
  if (ahead === 0) return t("scheduled.phrase.today", { time });
  if (ahead === 1) return t("scheduled.phrase.tomorrow", { time });
  const { weekday } = wallClock(at, zone);
  return t("scheduled.phrase.date", {
    // The weekday after a preposition is inflected in Polish, which Intl
    // does not do, so each language says it in its own words.
    weekday: t(`scheduled.onWeekday.${weekday}` as Parameters<Translator>[0]),
    date: format(locale, zone, { day: "numeric", month: "long" }, at),
    time,
  });
}

/** A day's heading in the list: "Tomorrow, Friday 27 November", "Monday, 30 November". */
export function dayHeading(day: CalendarDay, now: Date, zone: string, locale: InterfaceLocale, t: Translator): string {
  const at = noonOf(day);
  const weekday = format(locale, "UTC", { weekday: "long" }, at);
  const date = format(locale, "UTC", { day: "numeric", month: "long" }, at);
  const ahead = daysBetween(wallClock(now, zone), day);
  if (ahead === 0) return t("scheduled.day.today", { weekday, date });
  if (ahead === 1) return t("scheduled.day.tomorrow", { weekday, date });
  return t("scheduled.day.other", { weekday: capital(weekday, locale), date });
}

/** A failed message's time, with its short weekday: "Tu 8:00". */
export function failedTimeLabel(at: Date, zone: string, locale: InterfaceLocale, t: Translator): string {
  return `${shortWeekday(wallClock(at, zone).weekday, t)} ${clockLabel(at, zone, locale)}`;
}

/** "November 2026", over the month grid. */
export function monthTitle(year: number, month: number, locale: InterfaceLocale): string {
  return capital(format(locale, "UTC", { month: "long", year: "numeric" }, noonOf({ year, month, day: 1 })), locale);
}

/** The short weekday names over the grid's columns, Monday first. */
export function weekdayHeaders(t: Translator): string[] {
  return Array.from({ length: 7 }, (_, index) => shortWeekday(index + 1, t));
}

/** The full date of a grid day, for its button's name. */
export function dayName(day: CalendarDay, locale: InterfaceLocale): string {
  return format(locale, "UTC", { weekday: "long", day: "numeric", month: "long", year: "numeric" }, noonOf(day));
}

/**
 * The zone as it is named in the system. Intl can name a zone's time
 * ("Central European Time") but not translate the city it is called after,
 * so the zone's own name is shown, which is what the settings call it too.
 */
export function zoneName(zone: string): string {
  return zone.replace(/_/g, " ");
}
