import { locales, type InterfaceLocale, type Translator } from "@/i18n";
import { formatTime, isSameDay, pluralForm } from "@/lib/format/format";
import type { Poll } from "@/ports/polls";

type Key = Parameters<Translator>[0];

const MINUTE = 60_000;

/** "9 votes" under a poll with one answer each, "14 people" under one with several. */
export function pollCountLabel(poll: Poll, locale: InterfaceLocale, t: Translator): string {
  const noun = poll.multiple ? "poll.people" : "poll.votes";
  return t(`${noun}.${pluralForm(locale, poll.voters)}` as Key, { count: poll.voters });
}

/** "23 hours left": minutes in the last hour, hours in the last two days, days before. */
export function pollRemainingLabel(endsAt: string, now: Date, locale: InterfaceLocale, t: Translator): string {
  const minutes = Math.max(1, Math.ceil((Date.parse(endsAt) - now.getTime()) / MINUTE));
  const [unit, count]: [string, number] =
    minutes < 60
      ? ["minutes", minutes]
      : minutes < 48 * 60
        ? ["hours", Math.floor(minutes / 60)]
        : ["days", Math.floor(minutes / (24 * 60))];
  return t(`poll.left.${unit}.${pluralForm(locale, count)}` as Key, { count });
}

/** "ended yesterday at 21:15", in the reader's own time. */
export function pollEndedLabel(endedAt: string, now: Date, locale: InterfaceLocale, t: Translator): string {
  const at = new Date(endedAt);
  const time = formatTime(endedAt, locale);
  if (isSameDay(at, now)) return t("poll.ended.today", { time });
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (isSameDay(at, yesterday)) return t("poll.ended.yesterday", { time });
  const date = new Intl.DateTimeFormat(locales[locale].tag, { day: "numeric", month: "long" }).format(at);
  return t("poll.ended.date", { date, time });
}
