import { useId, useSyncExternalStore } from "react";

import { cx } from "@stellarr/ui";
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

/** Whatever event window is asked for: a new one, a change, or one card on its own. */
export function EventWindowHost() {
  const open = useSyncExternalStore(eventWindow.subscribe, eventWindow.get, eventWindow.get);
  const { events } = usePorts();
  const { server, voiceChannels, textChannels } = useDirectory();
  const { locale, t } = useLanguage();
  const zone = useReaderZone();
  const now = useEventNow();
  const titleId = useId();
  if (open === null) return null;

  if (open.kind === "view") {
    const event = events.get(open.eventId);
    if (event === undefined) return null;
    return (
      <Modal open onClose={eventWindow.close} labelledBy={titleId} className={cx(css.wrap)} exitMs={0}>
        <div className={css.card}>
          <h2 id={titleId} className="sr-only">
            {event.name}
          </h2>
          <EventCardContainer snapshot={event} />
        </div>
      </Modal>
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
