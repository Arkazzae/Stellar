import { useEffect, useId, useState, useSyncExternalStore } from "react";

import { Button, cx } from "@stellarr/ui";
import { EventSheet } from "@/components/events/EventSheet";
import { Modal } from "@/components/primitives/Modal";
import { eventWindow } from "@/lib/events/eventSheetRequest";
import type { EventDraft, ServerEvent } from "@/ports";
import { useDirectory, useLanguage, usePorts } from "@/state/hooks";

import { EventCardContainer } from "./EventCardContainer";
import { useEventNow } from "./useEvents";
import { useReaderZone } from "./useScheduled";
import css from "./EventWindowHost.module.css";

/** The event as its sheet starts from when it is changed. */
function draftOf(event: ServerEvent, zone: string): EventDraft {
  return {
    name: event.name,
    place: event.place,
    startsAt: new Date(event.startsAt),
    ...(event.endsAt === undefined ? {} : { endsAt: new Date(event.endsAt) }),
    timeZone: zone,
    ...(event.description === undefined ? {} : { description: event.description }),
    ...(event.seats === undefined ? {} : { seats: event.seats }),
    repeat: event.repeat,
  };
}

/**
 * One event's card on its own. One this client does not hold yet is looked
 * for in its server's events, once that server is on screen; there is no
 * other way to read a single event. Not there means it ended, was cancelled
 * or is no longer the reader's to see, and the window says so.
 */
function EventView({ eventId, serverId }: { eventId: string; serverId?: string }) {
  const { events } = usePorts();
  const { server } = useDirectory();
  const { t } = useLanguage();
  const titleId = useId();
  const event = useSyncExternalStore(
    events.subscribe,
    () => events.get(eventId),
    () => events.get(eventId),
  );
  const [looked, setLooked] = useState(false);
  const here = serverId === undefined || serverId === server.id;
  useEffect(() => {
    if (event !== undefined || looked || !here) return;
    let current = true;
    events.load(server.id).then(
      () => current && setLooked(true),
      () => current && setLooked(true),
    );
    return () => {
      current = false;
    };
  }, [event, looked, here, events, server.id]);

  if (event === undefined && !looked) return null;
  return (
    <Modal open onClose={eventWindow.close} labelledBy={titleId} className={cx(css.wrap)} exitMs={0}>
      {event === undefined ? (
        <div className={css.gone}>
          <h2 id={titleId}>{t("event.goneTitle")}</h2>
          <p>{t("event.goneBody")}</p>
          <Button type="button" variant="secondary" onClick={eventWindow.close}>
            {t("common.close")}
          </Button>
        </div>
      ) : (
        <div className={css.card}>
          <h2 id={titleId} className="sr-only">
            {event.name}
          </h2>
          <EventCardContainer snapshot={event} />
        </div>
      )}
    </Modal>
  );
}

/** Whatever event window is asked for: a new one, a change, or one card on its own. */
export function EventWindowHost() {
  const open = useSyncExternalStore(eventWindow.subscribe, eventWindow.get, eventWindow.get);
  const { events } = usePorts();
  const { server, voiceChannels, textChannels } = useDirectory();
  const { locale, t } = useLanguage();
  const zone = useReaderZone();
  const now = useEventNow();
  if (open === null) return null;

  if (open.kind === "view") {
    return (
      <EventView
        key={`${open.serverId ?? ""}:${open.eventId}`}
        eventId={open.eventId}
        {...(open.serverId === undefined ? {} : { serverId: open.serverId })}
      />
    );
  }

  const editing = open.kind === "edit" ? open.event : undefined;
  return (
    <EventSheet
      serverName={server.name}
      voiceChannels={voiceChannels}
      textChannels={textChannels}
      now={now}
      zone={zone}
      {...(editing === undefined ? {} : { initial: draftOf(editing, zone), announced: editing.announceMessageId !== undefined })}
      {...(open.kind === "create" && open.announceOn !== undefined ? { announceOn: open.announceOn } : {})}
      onSave={async (draft) => {
        if (editing === undefined) await events.create(draft);
        else await events.edit(editing, draft);
      }}
      onClose={eventWindow.close}
      locale={locale}
      t={t}
    />
  );
}
