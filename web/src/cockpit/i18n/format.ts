import { getLocale, locales } from "./index";

const intlTag = () => locales[getLocale()].tag;

export function formatNumber(value: number, options?: Intl.NumberFormatOptions): string {
  return value.toLocaleString(intlTag(), options);
}

/** A byte count shown in megabytes, with the unit. */
export function formatMegabytes(bytes: number, maximumFractionDigits: number): string {
  return formatNumber(bytes / 1024 / 1024, {
    style: "unit",
    unit: "megabyte",
    maximumFractionDigits,
  });
}

/** Date and time down to the second, as the logs and backups show them. */
export function formatDateTime(value: string): string {
  return new Date(value).toLocaleString(intlTag());
}

/** Date and time to the minute, in the abbreviated style of the account tables. */
export function formatShortDateTime(value: string): string {
  return new Date(value).toLocaleString(intlTag(), { dateStyle: "medium", timeStyle: "short" });
}

/** A time of day to the minute, as the charts of the last hour name their points. */
export function formatTime(value: string | number, options?: Intl.DateTimeFormatOptions): string {
  return new Date(value).toLocaleTimeString(intlTag(), { hour: "2-digit", minute: "2-digit", ...options });
}

/** A date with the month as a word, as a table shows when something came: "20 Nov 2026". */
export function formatMediumDate(value: string | number): string {
  return new Date(value).toLocaleDateString(intlTag(), { dateStyle: "medium" });
}

/** A day of the month and a time of day, for a moment of the last few days: "27 Nov, 16:31". */
export function formatDayTime(value: string | number): string {
  return new Date(value).toLocaleString(intlTag(), { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

export function formatDate(value: string): string {
  return new Date(value).toLocaleDateString(intlTag());
}

/**
 * A calendar date given as YYYY-MM-DD, shown as that day wherever the browser
 * is: the date is the instance's, so it is read and formatted in UTC.
 */
export function formatDay(value: string, options: Intl.DateTimeFormatOptions): string {
  return new Date(`${value}T00:00:00Z`).toLocaleDateString(intlTag(), { ...options, timeZone: "UTC" });
}

/** Lower-cases text for a case-insensitive search in the active language. */
export function lowerCase(text: string): string {
  return text.toLocaleLowerCase(getLocale());
}
