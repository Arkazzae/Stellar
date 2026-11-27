import { locales, type InterfaceLocale, type Translator } from "@/i18n";
import { pluralForm } from "@/lib/format/format";
import { safeHttpUrl } from "@/lib/format/safeUrl";
import type { EventState, ServerEvent } from "@/ports/events";

/*
 * What an event's card says and offers, decided in one place from the event,
 * the time and what the client knows of its voice channel. The card, the list
 * and the rail draw from it and hold nothing of their own.
 */

type Key = Parameters<Translator>[0];

/** What the card's one button does, if it has one. */
export type EventCardAction =
  | { kind: "signUp" }
  /** Signed up; pressing it signs out and frees the seat. */
  | { kind: "signedUp" }
  | { kind: "joinWaitlist" }
  /** Waiting; pressing it leaves the waitlist. */
  | { kind: "onWaitlist" }
  /** Running on a voice channel: anyone who may enter it can, whatever the seats. */
  | { kind: "join"; channelId: string; channelName: string }
  | { kind: "none" };

/** What the client knows of an event's voice channel. */
export interface EventChannelNow {
  /** The channel's name, from the reader's own list. */
  name?: string;
  /** How many are on it now. */
  present?: number;
  /** How many holding a seat are not on it yet. */
  signedAbsent?: number;
}

/**
 * Where it stands. With an end, the clock decides as the server does, even
 * before it says so; a voice event without one runs until the server ends it.
 */
export function eventPhase(event: ServerEvent, now: Date): EventState {
  if (event.state === "cancelled" || event.state === "ended") return event.state;
  if (event.endsAt !== undefined && Date.parse(event.endsAt) <= now.getTime()) return "ended";
  if (event.state === "live" || Date.parse(event.startsAt) <= now.getTime()) return "live";
  return "upcoming";
}

export function isFull(event: ServerEvent): boolean {
  return event.seats !== undefined && event.signedUp >= event.seats;
}

/** Where, in words: the voice channel's name from the reader's list, or the place written. */
export function placeName(event: ServerEvent, channel: EventChannelNow = {}): string {
  return event.place.kind === "elsewhere" ? event.place.text : (channel.name ?? "");
}

/** A place in words that is a web address, as a link to follow; anything else is text. */
export function placeLink(event: ServerEvent): string | undefined {
  return event.place.kind === "elsewhere" ? safeHttpUrl(event.place.text.trim()) : undefined;
}

export function eventAction(event: ServerEvent, now: Date, channel: EventChannelNow = {}): EventCardAction {
  const phase = eventPhase(event, now);
  if (phase === "ended" || phase === "cancelled") return { kind: "none" };
  if (phase === "live") {
    return event.place.kind === "voice"
      ? { kind: "join", channelId: event.place.channelId, channelName: placeName(event, channel) }
      : { kind: "none" };
  }
  if (event.mine === "signed") return { kind: "signedUp" };
  if (event.mine === "waiting") return { kind: "onWaitlist" };
  return isFull(event) ? { kind: "joinWaitlist" } : { kind: "signUp" };
}

const counted = (prefix: string, count: number, locale: InterfaceLocale, t: Translator) =>
  t(`${prefix}.${pluralForm(locale, count)}` as Key, { count });

/** The line beside the button: seats, the reader's place, or who is still missing. */
export function eventStatus(
  event: ServerEvent,
  now: Date,
  locale: InterfaceLocale,
  t: Translator,
  channel: EventChannelNow = {},
): string {
  const phase = eventPhase(event, now);
  const { seats, signedUp, waiting } = event;
  const signed = counted("event.signedUp", signedUp, locale, t);
  if (phase === "cancelled") return "";
  if (phase === "ended") return signed;
  if (phase === "live") {
    return channel.signedAbsent !== undefined && channel.signedAbsent > 0
      ? counted("event.notJoined", channel.signedAbsent, locale, t)
      : signed;
  }
  if (event.mine === "waiting") {
    return event.myPosition === undefined
      ? t("event.waitingNoPosition")
      : t("event.queuePosition", { position: event.myPosition });
  }
  if (event.mine === "signed") {
    const count = seats === undefined ? signed : t("event.seatsShort", { taken: signedUp, seats });
    return `${count} · ${t("event.reminder")}`;
  }
  if (isFull(event)) {
    return waiting === 0 ? t("event.full") : `${t("event.full")} · ${counted("event.waiting", waiting, locale, t)}`;
  }
  if (signedUp === 0) return t("event.nobodyYet");
  return seats === undefined ? signed : t("event.seatsTaken", { taken: signedUp, seats });
}

function format(locale: InterfaceLocale, zone: string, options: Intl.DateTimeFormatOptions, at: Date): string {
  return new Intl.DateTimeFormat(locales[locale].tag, { timeZone: zone, ...options }).format(at);
}

/** "20:00", "9:00": the hour without the zero some languages pad it with. */
export function clockOf(at: Date, zone: string, locale: InterfaceLocale): string {
  return new Intl.DateTimeFormat(locales[locale].tag, { timeZone: zone, hour: "numeric", minute: "2-digit" })
    .formatToParts(at)
    .map((part) => (part.type === "hour" ? String(Number(part.value)) : part.value))
    .join("");
}

/** The date on the card's calendar leaf: "sob" over "28". */
export function calendarLeaf(at: Date, zone: string, locale: InterfaceLocale): { weekday: string; day: string } {
  return {
    weekday: format(locale, zone, { weekday: "short" }, at).replace(/\.$/, ""),
    day: format(locale, zone, { day: "numeric" }, at),
  };
}

const REPEATS: Record<ServerEvent["repeat"], Key | null> = {
  none: null,
  weekly: "event.repeat.weekly",
  biweekly: "event.repeat.biweekly",
  monthly: "event.repeat.monthly",
};

/**
 * When it is, under its name: "Saturday, 28 November · 20:00–22:00", and
 * while it runs, since when and how many are on its channel.
 */
export function eventWhen(
  event: ServerEvent,
  now: Date,
  zone: string,
  locale: InterfaceLocale,
  t: Translator,
  channel: EventChannelNow = {},
): string {
  const start = new Date(event.startsAt);
  if (eventPhase(event, now) === "live") {
    const since = t("event.since", { time: clockOf(new Date(event.liveSince ?? event.startsAt), zone, locale) });
    return channel.present === undefined ? since : `${since} · ${counted("event.onChannel", channel.present, locale, t)}`;
  }
  const day = format(locale, zone, { weekday: "long", day: "numeric", month: "long" }, start);
  const hours =
    event.endsAt === undefined
      ? clockOf(start, zone, locale)
      : `${clockOf(start, zone, locale)}–${clockOf(new Date(event.endsAt), zone, locale)}`;
  const repeat = REPEATS[event.repeat];
  return [day, hours, repeat === null ? null : t(repeat)].filter((part) => part !== null).join(" · ");
}

/** A row in the list: "20:00 · Gaming · 8 z 10 miejsc", with how it repeats first. */
export function eventListLine(
  event: ServerEvent,
  zone: string,
  locale: InterfaceLocale,
  t: Translator,
  channel: EventChannelNow = {},
): string {
  const repeat = REPEATS[event.repeat];
  const people =
    event.seats === undefined
      ? counted("event.people", event.signedUp, locale, t)
      : t("event.seatsOf", { taken: event.signedUp, seats: event.seats });
  return [repeat === null ? null : t(repeat), clockOf(new Date(event.startsAt), zone, locale), placeName(event, channel), people]
    .filter((part): part is string => part !== null && part !== "")
    .join(" · ");
}

/** What the rail row says: how many are coming, and whether one runs now. */
export function railSummary(events: readonly ServerEvent[], now: Date): { upcoming: number; live: boolean } {
  let upcoming = 0;
  let live = false;
  for (const event of events) {
    const phase = eventPhase(event, now);
    if (phase === "live") live = true;
    else if (phase === "upcoming") upcoming += 1;
  }
  return { upcoming, live };
}
