import { useId, useRef, useState, type CSSProperties } from "react";

import { useLayer } from "@stellarr/ui/behavior";
import { Icon } from "@stellarr/icons";
import { trapTab } from "@/components/primitives/focus";
import { calendarLeaf, eventListLine, eventPhase, railSummary, type EventChannelNow } from "@/lib/events/eventCard";
import type { ServerEvent } from "@/ports";
import { useLanguage } from "@/state/hooks";

import css from "./EventsRail.module.css";

export interface EventsRailProps {
  events: readonly ServerEvent[];
  now: Date;
  zone: string;
  /** What is known of the voice channels, by id. */
  channels?: Readonly<Record<string, EventChannelNow>>;
  /** Opens one event's card. */
  onOpen(event: ServerEvent): void;
  /** Starts a new event; absent without the right to create them. */
  onCreate?: () => void;
}

/** Where the open list stands, in the window. */
interface ListPlace {
  left: number;
  top: number;
  /** As tall as it may grow before it scrolls. */
  maxHeight: number;
}

/** As wide as the list is, which `.list` in the stylesheet says too. */
const LIST_WIDTH = 340;
const LIST_MAX_HEIGHT = 520;
/** Between the list and the rail, and the list and the window's edges. */
const GAP = 8;
const EDGE = 12;
/** A list lower than this would be cramped; it moves up instead. */
const LIST_MIN_HEIGHT = 280;

/**
 * Beside the rail, level with the row, over the room: the list does not
 * cover the channels it was opened from. It stays inside the window,
 * moving up when the row is too low for it to fit.
 */
function placeBeside(row: Element): ListPlace {
  // The rail as a whole, so the list starts past its edge and not over it.
  const rail = (row.closest(".rail") ?? row).getBoundingClientRect();
  const top = row.getBoundingClientRect().top;
  const left = Math.max(EDGE, Math.min(rail.right + GAP, window.innerWidth - LIST_WIDTH - EDGE));
  const raised = Math.max(EDGE, Math.min(top, window.innerHeight - LIST_MIN_HEIGHT - EDGE));
  return {
    left,
    top: raised,
    maxHeight: Math.min(LIST_MAX_HEIGHT, window.innerHeight - raised - EDGE),
  };
}

/**
 * The row over the channels: how many events are coming, or that one runs
 * now. It opens the list of them, with "Nowe" for whoever may create one.
 */
export function EventsRail({ events, now, zone, channels = {}, onOpen, onCreate }: EventsRailProps) {
  const { t } = useLanguage();
  /** Where the list stands while it is open. */
  const [open, setOpen] = useState<ListPlace | null>(null);
  const listId = useId();
  const { upcoming, live } = railSummary(events, now);

  return (
    <div className={css.wrap}>
      <div className={live ? `channel ${css.row} ${css.live}` : `channel ${css.row}`}>
        <button
          type="button"
          className="channel-main"
          aria-haspopup="dialog"
          aria-expanded={open !== null}
          aria-controls={open === null ? undefined : listId}
          onClick={(click) => {
            const place = placeBeside(click.currentTarget);
            setOpen((current) => (current === null ? place : null));
          }}
        >
          <span className="glyph" aria-hidden="true">
            <Icon name="calendar" size={14} />
          </span>
          <span className="name">{t("event.rail")}</span>
          {live ? (
            <small className={css.now}>{t("event.railLive")}</small>
          ) : upcoming > 0 ? (
            <span className={css.count} aria-label={t("event.upcomingCount", { count: upcoming })}>
              {upcoming}
            </span>
          ) : null}
        </button>
      </div>
      {open === null ? null : (
        <EventsList
          id={listId}
          place={open}
          events={events}
          now={now}
          zone={zone}
          channels={channels}
          onOpen={(event) => {
            setOpen(null);
            onOpen(event);
          }}
          {...(onCreate === undefined
            ? {}
            : {
                onCreate: () => {
                  setOpen(null);
                  onCreate();
                },
              })}
          onClose={() => setOpen(null)}
        />
      )}
    </div>
  );
}

interface EventsListProps extends Omit<EventsRailProps, "channels"> {
  id: string;
  /** Where it stands, beside the row it opened from. */
  place: ListPlace;
  channels: Readonly<Record<string, EventChannelNow>>;
  onClose(): void;
}

/** The coming and running events, soonest first; ended ones are left out. */
function EventsList({ id, place, events, now, zone, channels, onOpen, onCreate, onClose }: EventsListProps) {
  const { t, locale } = useLanguage();
  const titleId = useId();
  const panel = useRef<HTMLDivElement>(null);
  useLayer(true, {
    node: () => panel.current,
    onEscape: (event) => {
      event.preventDefault();
      onClose();
    },
    onTab: (event) => {
      if (panel.current !== null) trapTab(event, panel.current);
    },
    onPointerDownOutside: () => onClose(),
  });
  // Ended and cancelled ones leave the list; their cards stay where they were announced.
  const shown = events.filter((event) => {
    const phase = eventPhase(event, now);
    return phase === "upcoming" || phase === "live";
  });

  return (
    <div
      ref={panel}
      id={id}
      className={css.list}
      // Fixed, so the rail's scrolling does not clip a list that stands outside it.
      style={{ left: place.left, top: place.top, "--list-max-height": `${place.maxHeight}px` } as CSSProperties}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
    >
      <div className={css.head}>
        <b id={titleId}>{t("event.list")}</b>
        {onCreate === undefined ? null : (
          <button type="button" className={css.create} onClick={onCreate}>
            <Icon name="plus" size={14} />
            {t("event.new")}
          </button>
        )}
      </div>
      {shown.length === 0 ? (
        <p className={css.empty}>{t("event.listEmpty")}</p>
      ) : (
        <ul className={css.items}>
          {shown.map((event) => {
            const start = new Date(event.startsAt);
            const leaf = calendarLeaf(start, zone, locale);
            const channel = event.place.kind === "voice" ? channels[event.place.channelId] : undefined;
            const phase = eventPhase(event, now);
            const mark =
              phase === "live"
                ? t("event.live")
                : event.mine === "signed"
                  ? t("event.signedUpButton")
                  : event.mine === "waiting"
                    ? t("event.onWaitlist")
                    : null;
            return (
              <li key={event.id}>
                <button type="button" className={css.item} onClick={() => onOpen(event)}>
                  <span className={css.leaf} aria-hidden="true">
                    <i>{leaf.weekday}</i>
                    <b>{leaf.day}</b>
                  </span>
                  <span className={css.itemText}>
                    <b>{event.name}</b>
                    <span>{eventListLine(event, zone, locale, t, channel)}</span>
                  </span>
                  {mark === null ? null : <span className={css.mine}>{mark}</span>}
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
