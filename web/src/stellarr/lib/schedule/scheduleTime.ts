import type { InterfaceLocale } from "@/i18n";
import { foldCommandText } from "@/lib/messages/composerCommands";

import { addDays, instantAt, wallClock, weekdayOf, type CalendarDay } from "./zonedTime";

/** A scheduled message goes out at least this far ahead… */
export const MIN_LEAD_MS = 60_000;
/** …and at most this many days ahead, as the server allows. */
export const MAX_AHEAD_DAYS = 30;
const MAX_AHEAD_MS = MAX_AHEAD_DAYS * 86_400_000;

/** Today's evening is offered only until about an hour before it. */
const EVENING_OFFERED_UNTIL_HOUR = 17;
const EVENING = { hour: 18, minute: 0 };
const MORNING = { hour: 9, minute: 0 };

export type TimeProblem = "past" | "tooSoon" | "tooFar";

/** Why `at` cannot be a send time from `now`, or null when it can. */
export function timeProblem(at: Date, now: Date): TimeProblem | null {
  const ahead = at.getTime() - now.getTime();
  if (ahead <= 0) return "past";
  if (ahead < MIN_LEAD_MS) return "tooSoon";
  if (ahead > MAX_AHEAD_MS) return "tooFar";
  return null;
}

/** The last calendar day that has any time still allowed, in `zone`. */
export function lastAllowedDay(now: Date, zone: string): CalendarDay {
  return wallClock(new Date(now.getTime() + MAX_AHEAD_MS), zone);
}

export interface SchedulePreset {
  id: "today" | "tomorrow" | "monday";
  at: Date;
}

/**
 * The times the send-later menu offers: this evening while there is still time
 * to write, tomorrow morning, and next Monday morning unless that is
 * tomorrow anyway.
 */
export function schedulePresets(now: Date, zone: string): SchedulePreset[] {
  const today = wallClock(now, zone);
  const presets: SchedulePreset[] = [];
  if (today.hour < EVENING_OFFERED_UNTIL_HOUR) {
    presets.push({ id: "today", at: instantAt(today, EVENING, zone) });
  }
  presets.push({ id: "tomorrow", at: instantAt(addDays(today, 1), MORNING, zone) });
  if (today.weekday !== 7) {
    presets.push({ id: "monday", at: instantAt(addDays(today, 8 - today.weekday), MORNING, zone) });
  }
  return presets;
}

/*
 * The words "kiedy" understands, folded (lower case, no diacritics) so the
 * tables stay plain ASCII and a word typed with or without its accents is
 * found either way. English works in every
 * language; the reader's own language adds its words.
 */
interface Words {
  today: string[];
  tomorrow: string[];
  dayAfter: string[];
  /** "in" before an amount of time. */
  within: string[];
  /** Words that may stand before a day or a time and mean nothing more. */
  filler: string[];
  hours: string[];
  minutes: string[];
  /** Monday first. */
  weekdays: string[][];
}

const ENGLISH: Words = {
  today: ["today"],
  tomorrow: ["tomorrow", "tmrw", "tmr"],
  dayAfter: [],
  within: ["in"],
  filler: ["at", "on", "next"],
  hours: ["h", "hr", "hrs", "hour", "hours"],
  minutes: ["m", "min", "mins", "minute", "minutes"],
  weekdays: [
    ["mon", "monday"],
    ["tue", "tues", "tuesday"],
    ["wed", "weds", "wednesday"],
    ["thu", "thur", "thurs", "thursday"],
    ["fri", "friday"],
    ["sat", "saturday"],
    ["sun", "sunday"],
  ],
};

const WORDS: Record<InterfaceLocale, Words> = {
  en: ENGLISH,
  pl: {
    today: ["dzis", "dzisiaj"],
    tomorrow: ["jutro"],
    dayAfter: ["pojutrze"],
    within: ["za"],
    filler: ["o", "w", "we", "na", "godz"],
    hours: ["h", "g", "godz", "godzina", "godzine", "godziny", "godzin"],
    minutes: ["m", "min", "minuta", "minute", "minuty", "minut"],
    weekdays: [
      ["pn", "pon", "poniedzialek"],
      ["wt", "wto", "wtorek"],
      ["sr", "sroda", "srode"],
      ["cz", "czw", "czwartek"],
      ["pt", "pia", "piatek"],
      ["sb", "sob", "sobota", "sobote"],
      ["nd", "ndz", "niedz", "niedziela", "niedziele"],
    ],
  },
  de: {
    today: ["heute"],
    tomorrow: ["morgen"],
    dayAfter: ["ubermorgen", "uebermorgen"],
    within: ["in"],
    filler: ["um", "am", "nachsten", "naechsten"],
    hours: ["h", "std", "stunde", "stunden"],
    minutes: ["m", "min", "minute", "minuten"],
    weekdays: [
      ["mo", "montag"],
      ["di", "dienstag"],
      ["mi", "mittwoch"],
      ["do", "donnerstag"],
      ["fr", "freitag"],
      ["sa", "samstag"],
      ["so", "sonntag"],
    ],
  },
};

function wordsFor(locale: InterfaceLocale): Words[] {
  return locale === "en" ? [ENGLISH] : [WORDS[locale], ENGLISH];
}

const has = (lists: Words[], pick: (words: Words) => string[], token: string) =>
  lists.some((words) => pick(words).includes(token));

export type ParsedWhen = { at: Date } | { problem: TimeProblem | "unknown" };

const TIME = /^(\d{1,2})(?:[:.](\d{2}))?(am|pm)?$/;

function clockTime(token: string): { hour: number; minute: number } | null {
  const match = TIME.exec(token);
  if (match === null) return null;
  let hour = Number(match[1]);
  const minute = match[2] === undefined ? 0 : Number(match[2]);
  const half = match[3];
  if (half !== undefined) {
    if (hour < 1 || hour > 12) return null;
    hour = (hour % 12) + (half === "pm" ? 12 : 0);
  }
  if (hour > 23 || minute > 59) return null;
  return { hour, minute };
}

/**
 * Reads "kiedy" as typed after /zaplanuj: a day and a time ("jutro 9:00",
 * "pon 18:00", "tomorrow 9am"), a time alone (today, or tomorrow once it has
 * passed), a day alone (at 9:00), or an amount of time ("za 2 h",
 * "in 30 min"). A weekday is the next one that is still ahead.
 */
export function parseWhen(input: string, now: Date, zone: string, locale: InterfaceLocale): ParsedWhen {
  const lists = wordsFor(locale);
  const tokens = foldCommandText(input)
    .replace(/,/g, " ")
    // "9 am" and "9am" are the same time.
    .replace(/(\d)\s+(am|pm)\b/g, "$1$2")
    .split(/\s+/)
    .filter((token) => token !== "");
  if (tokens.length === 0) return { problem: "unknown" };

  const checked = (at: Date): ParsedWhen => {
    const problem = timeProblem(at, now);
    return problem === null ? { at } : { problem };
  };

  // An amount of time: "za 2 h", "in 30 min", "in 2h".
  if (has(lists, (words) => words.within, tokens[0] ?? "")) {
    const rest = tokens.slice(1).join(" ");
    const match = /^(\d{1,4})\s*([a-z]+)$/.exec(rest);
    if (match === null) return { problem: "unknown" };
    const amount = Number(match[1]);
    const unit = match[2] ?? "";
    const step = has(lists, (words) => words.hours, unit)
      ? 3_600_000
      : has(lists, (words) => words.minutes, unit)
        ? 60_000
        : null;
    if (step === null || amount === 0) return { problem: "unknown" };
    return checked(new Date(now.getTime() + amount * step));
  }

  let day: CalendarDay | null = null;
  let weekday: number | null = null;
  let time: { hour: number; minute: number } | null = null;
  const today = wallClock(now, zone);
  for (const token of tokens) {
    if (has(lists, (words) => words.filler, token)) continue;
    if (has(lists, (words) => words.today, token) && day === null && weekday === null) {
      day = today;
    } else if (has(lists, (words) => words.tomorrow, token) && day === null && weekday === null) {
      day = addDays(today, 1);
    } else if (has(lists, (words) => words.dayAfter, token) && day === null && weekday === null) {
      day = addDays(today, 2);
    } else if (day === null && weekday === null && lists.some((words) => words.weekdays.some((names) => names.includes(token)))) {
      for (const words of lists) {
        const index = words.weekdays.findIndex((names) => names.includes(token));
        if (index !== -1) {
          weekday = index + 1;
          break;
        }
      }
    } else if (time === null && clockTime(token) !== null) {
      time = clockTime(token);
    } else {
      return { problem: "unknown" };
    }
  }

  if (weekday !== null) {
    const at = time ?? MORNING;
    const ahead = (weekday - weekdayOf(today) + 7) % 7;
    // The same weekday as today means today while that time is still ahead.
    const candidate = instantAt(addDays(today, ahead), at, zone);
    return checked(
      timeProblem(candidate, now) === null || ahead !== 0 ? candidate : instantAt(addDays(today, 7), at, zone),
    );
  }
  if (day !== null) return checked(instantAt(day, time ?? MORNING, zone));
  if (time !== null) {
    const later = instantAt(today, time, zone);
    return checked(timeProblem(later, now) === null ? later : instantAt(addDays(today, 1), time, zone));
  }
  return { problem: "unknown" };
}
