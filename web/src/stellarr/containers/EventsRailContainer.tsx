import { EventsRail } from "@/components/events/EventsRail";
import { eventWindow } from "@/lib/events/eventSheetRequest";
import { useDirectory } from "@/state/hooks";

import { useEventChannels, useEventNow, useServerEvents } from "./useEvents";
import { useReaderZone } from "./useScheduled";

/** The events row over the channels, for the server on screen. */
export function EventsRailContainer() {
  const events = useServerEvents();
  const now = useEventNow();
  const zone = useReaderZone();
  const channels = useEventChannels();
  const { canCreateEvents } = useDirectory();
  // Nothing to show and nothing to create: no row at all.
  if (events.length === 0 && !canCreateEvents) return null;
  return (
    <EventsRail
      events={events}
      now={now}
      zone={zone}
      channels={channels}
      onOpen={(event) => eventWindow.open({ kind: "view", eventId: event.id })}
      {...(canCreateEvents ? { onCreate: () => eventWindow.open({ kind: "create" }) } : {})}
    />
  );
}
