import { EventCard } from "@/components/events/EventCard";
import { eventPhase } from "@/lib/events/eventCard";
import type { ServerEvent } from "@/ports";
import { useDirectory } from "@/state/hooks";

import { useEvent, useEventActions, useEventChannels, useEventManagement, useEventNow, useSignedAbsent } from "./useEvents";
import { useReaderZone } from "./useScheduled";

/**
 * An event's card wherever it is shown, kept current: the copy it came with
 * until a newer one is heard, the time, and who is on its channel.
 */
export function EventCardContainer({ snapshot }: { snapshot: ServerEvent }) {
  const event = useEvent(snapshot);
  const now = useEventNow();
  const zone = useReaderZone();
  const channels = useEventChannels();
  const { members } = useDirectory();
  const actions = useEventActions();
  const manage = useEventManagement(event);
  const live = eventPhase(event, now) === "live";
  const signedAbsent = useSignedAbsent(event, live);
  const known = event.place.kind === "voice" ? channels[event.place.channelId] : undefined;
  const faces = new Map(members.flatMap((member) => (member.avatarUrl === undefined ? [] : [[member.id, member.avatarUrl] as const])));
  return (
    <EventCard
      event={event}
      now={now}
      zone={zone}
      channel={{ ...known, ...(signedAbsent === undefined ? {} : { signedAbsent }) }}
      faceOf={(userId) => faces.get(userId)}
      actions={actions}
      {...(manage === undefined ? {} : { manage })}
    />
  );
}
