import { useId, useRef, useState } from "react";

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

/**
 * The row over the channels: how many events are coming, or that one runs
 * now. It opens the list of them, with "Nowe" for whoever may create one.
 */
export function EventsRail({ events, now, zone, channels = {}, onOpen, onCreate }: EventsRailProps) {
  const { t } = useLanguage();
  /** Where the row is, while its list is open: the list hangs under it, over the room. */
  const [open, setOpen] = useState<DOMRect | null>(null);
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
            const row = click.currentTarget.getBoundingClientRect();
            setOpen((current) => (current === null ? row : null));
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
          anchor={open}
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
  /** The row it opened from. */
  anchor: DOMRect;
  channels: Readonly<Record<string, EventChannelNow>>;
  onClose(): void;
}

/** The coming and running events, soonest first; ended ones are left out. */
function EventsList({ id, anchor, events, now, zone, channels, onOpen, onCreate, onClose }: EventsListProps) {
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
      // Fixed, so the rail's scrolling does not clip a list wider than the rail.
      style={{ left: anchor.left, top: anchor.bottom + 4 }}
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
