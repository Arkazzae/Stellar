/*
 * Calendar dates and clock times in a named time zone, as the person reads
 * them, and the instants they stand for. Intl knows every zone's rules; this
 * only asks it. Every function takes the zone, so nothing here depends on the
 * zone of the machine it runs on.
 */

/** A calendar day; months count from 1. */
export interface CalendarDay {
  year: number;
  month: number;
  day: number;
}

export interface WallClock extends CalendarDay {
  hour: number;
  minute: number;
  /** 1 is Monday, 7 is Sunday. */
  weekday: number;
}

const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const readers = new Map<string, Intl.DateTimeFormat>();

function reader(zone: string): Intl.DateTimeFormat {
  let found = readers.get(zone);
  if (found === undefined) {
    found = new Intl.DateTimeFormat("en-US", {
      timeZone: zone,
      hourCycle: "h23",
      weekday: "short",
      year: "numeric",
      month: "numeric",
      day: "numeric",
      hour: "numeric",
      minute: "numeric",
      second: "numeric",
    });
    readers.set(zone, found);
  }
  return found;
}

function partsAt(at: Date, zone: string): WallClock & { second: number } {
  const parts: Record<string, string> = {};
  for (const part of reader(zone).formatToParts(at)) parts[part.type] = part.value;
  return {
    year: Number(parts.year),
    month: Number(parts.month),
    day: Number(parts.day),
    // Some engines write midnight as 24 in a 23-hour cycle.
    hour: Number(parts.hour) % 24,
    minute: Number(parts.minute),
    second: Number(parts.second),
    weekday: WEEKDAYS.indexOf(parts.weekday ?? "") + 1,
  };
}

/** What a clock in `zone` shows at `at`. */
export function wallClock(at: Date, zone: string): WallClock {
  const { second: _second, ...clock } = partsAt(at, zone);
  return clock;
}

/** How far `zone` is ahead of UTC at `at`, in milliseconds. */
function offsetAt(at: number, zone: string): number {
  const clock = partsAt(new Date(at), zone);
  const shown = Date.UTC(clock.year, clock.month - 1, clock.day, clock.hour, clock.minute, clock.second);
  return shown - (at - (((at % 1000) + 1000) % 1000));
}

/**
 * The instant at which a clock in `zone` shows `time` on `day`. The offset is
 * the one in force at that instant, not now, so a date across a change of
 * season comes out right. A time skipped when clocks go forward lands just
 * after the skip; a time that happens twice when they go back is the first.
 */
export function instantAt(day: CalendarDay, time: { hour: number; minute: number }, zone: string): Date {
  const shown = Date.UTC(day.year, day.month - 1, day.day, time.hour, time.minute);
  // The offsets on either side of a change of season are an hour or so
  // apart, so the two readings a day earlier and later cover both of them.
  const offsets = new Set([offsetAt(shown - 86_400_000, zone), offsetAt(shown + 86_400_000, zone), offsetAt(shown, zone)]);
  const readings = [...offsets].map((offset) => shown - offset).sort((left, right) => left - right);
  const exact = readings.filter((at) => {
    const clock = partsAt(new Date(at), zone);
    return sameDay(clock, day) && clock.hour === time.hour && clock.minute === time.minute;
  });
  // Shown twice: the first time. Never shown: after the skip, the latest reading.
  return new Date(exact[0] ?? readings[readings.length - 1] ?? shown);
}

export function addDays(day: CalendarDay, count: number): CalendarDay {
  const moved = new Date(Date.UTC(day.year, day.month - 1, day.day + count));
  return { year: moved.getUTCFullYear(), month: moved.getUTCMonth() + 1, day: moved.getUTCDate() };
}

/** Whole calendar days from `from` to `to`. */
export function daysBetween(from: CalendarDay, to: CalendarDay): number {
  return Math.round(
    (Date.UTC(to.year, to.month - 1, to.day) - Date.UTC(from.year, from.month - 1, from.day)) / 86_400_000,
  );
}

export function sameDay(left: CalendarDay, right: CalendarDay): boolean {
  return left.year === right.year && left.month === right.month && left.day === right.day;
}

/** 1 is Monday, 7 is Sunday. */
export function weekdayOf(day: CalendarDay): number {
  const sunday0 = new Date(Date.UTC(day.year, day.month - 1, day.day)).getUTCDay();
  return sunday0 === 0 ? 7 : sunday0;
}

/** The zone this browser is set to, which is the reader's. */
export function readerZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
}
