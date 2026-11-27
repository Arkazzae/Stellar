import { useEffect, useMemo, useState, useSyncExternalStore } from "react";

import type { EventCardActions, EventCardManagement } from "@/components/events/EventCard";
import type { EventChannelNow } from "@/lib/events/eventCard";
import { eventWindow } from "@/lib/events/eventSheetRequest";
import type { ServerEvent } from "@/ports";
import { useDirectory, usePorts, useSession, useUi } from "@/state/hooks";

const TICK_MS = 30_000;

/** The time, kept current while something on screen depends on it. */
export function useEventNow(): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), TICK_MS);
    return () => window.clearInterval(timer);
  }, []);
  return now;
}

/** The coming and running events of the server on screen, read when it is opened. */
export function useServerEvents(): readonly ServerEvent[] {
  const { events } = usePorts();
  const { server } = useDirectory();
  const serverId = server.id;
  useEffect(() => {
    void events.load(serverId).catch(() => undefined);
  }, [events, serverId]);
  return useSyncExternalStore(
    events.subscribe,
    () => events.list(serverId),
    () => events.list(serverId),
  );
}

/** The newest copy of one event: the store's, or the one it came with if that is newer. */
export function useEvent(snapshot: ServerEvent): ServerEvent {
  const { events } = usePorts();
  useEffect(() => events.remember(snapshot), [events, snapshot]);
  const known = useSyncExternalStore(
    events.subscribe,
    () => events.get(snapshot.id),
    () => events.get(snapshot.id),
  );
  return known !== undefined && known.revision >= snapshot.revision ? known : snapshot;
}

/** The voice channels as the cards name them, with how many are on each. */
export function useEventChannels(): Readonly<Record<string, EventChannelNow>> {
  const { voiceChannels } = useDirectory();
  return useMemo(
    () => Object.fromEntries(voiceChannels.map((channel) => [channel.id, { name: channel.name, present: channel.occupants.length }])),
    [voiceChannels],
  );
}

/**
 * How many of those holding a seat are not on the running event's channel
 * yet: the sign-ups, read while it runs, against who is there.
 */
export function useSignedAbsent(event: ServerEvent, live: boolean): number | undefined {
  const { events } = usePorts();
  const { voiceChannels } = useDirectory();
  const channelId = event.place.kind === "voice" ? event.place.channelId : null;
  const [seated, setSeated] = useState<{ key: string; ids: ReadonlySet<string> } | null>(null);
  const key = `${event.id}:${event.occurrenceId}:${event.signedUp}`;
  useEffect(() => {
    if (!live || channelId === null) return;
    let current = true;
    events.people(event).then(
      (page) => {
        if (!current) return;
        const ids = page.people.filter((person) => person.state === "signed").map((person) => person.id);
        setSeated({ key, ids: new Set(ids) });
      },
      () => undefined,
    );
    return () => {
      current = false;
    };
    // Read again when the sign-ups change, not on every copy of the event.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, live, channelId, events]);
  if (!live || channelId === null || seated === null || seated.key !== key) return undefined;
  const present = new Set(voiceChannels.find((channel) => channel.id === channelId)?.occupants.map((person) => person.id));
  let absent = 0;
  for (const id of seated.ids) if (!present.has(id)) absent += 1;
  return absent;
}

/** Signing up and out, and entering the channel, as any reader may. */
export function useEventActions(): EventCardActions {
  const { events } = usePorts();
  const ui = useUi();
  return useMemo(
    () => ({
      signUp: (event) => events.signUp(event),
      signOut: (event) => events.signOut(event),
      join: (channelId) => ui.joinVoiceChannel(channelId),
    }),
    [events, ui],
  );
}

/** Changing and cancelling, for its creator and whoever manages events. */
export function useEventManagement(event: ServerEvent): EventCardManagement | undefined {
  const { events } = usePorts();
  const { canManageEvents } = useDirectory();
  const { user } = useSession();
  const allowed = canManageEvents || (user !== null && user.id === event.createdBy);
  return useMemo(
    () =>
      allowed
        ? {
            edit: (target) => eventWindow.open({ kind: "edit", event: target }),
            cancel: (target) => events.cancel(target.id),
          }
        : undefined,
    [allowed, events],
  );
}
