import type { EventSignupChange, ServerEvent } from "@/ports/events";

/*
 * How copies of one event are put together: from a list read, a message, a
 * live change or an answer to a sign-up. A copy older than the one held is
 * dropped. A copy shared with everyone does not know the reader's own place,
 * so the place held stays, unless the event has moved on to a new date, which
 * nobody has signed up for yet.
 */

export function mergeEvent(known: ServerEvent | undefined, incoming: ServerEvent): ServerEvent {
  if (known === incoming || (known !== undefined && incoming.revision < known.revision)) return known;
  const mineRevision = incoming.mineRevision ?? incoming.revision;
  if (known === undefined || known.occurrenceId !== incoming.occurrenceId) {
    if (known !== undefined && incoming.revision === known.revision) return known;
    if (incoming.mine !== undefined) return { ...incoming, mineRevision };
    return known === undefined ? incoming : { ...incoming, mine: "none" };
  }

  // The public details and the reader's signup may arrive in either order.
  // Keep the cached object itself when neither stream has advanced.
  const details = incoming.revision > known.revision ? incoming : known;
  if (incoming.mine !== undefined && mineRevision > (known.mineRevision ?? -1)) {
    const { myPosition: _position, ...rest } = details;
    return {
      ...rest,
      mine: incoming.mine,
      mineRevision,
      ...(incoming.myPosition === undefined ? {} : { myPosition: incoming.myPosition }),
    };
  }
  if (details === known) return known;
  const { mine: _mine, mineRevision: _revision, myPosition: _position, ...shared } = details;
  return {
    ...shared,
    ...(known.mine === undefined ? {} : { mine: known.mine }),
    ...(known.myPosition === undefined ? {} : { myPosition: known.myPosition }),
    ...(known.mineRevision === undefined ? {} : { mineRevision: known.mineRevision }),
  };
}

/** The reader's own place, for the date it is about, in its own order. */
export function applySignupChange(known: ServerEvent | undefined, change: EventSignupChange): ServerEvent | undefined {
  if (known === undefined || known.occurrenceId !== change.occurrenceId) return known;
  if (known.mineRevision !== undefined && change.revision < known.mineRevision) return known;
  const { myPosition: _myPosition, ...rest } = known;
  return {
    ...rest,
    mine: change.mine,
    ...(change.myPosition === undefined ? {} : { myPosition: change.myPosition }),
    mineRevision: change.revision,
  };
}

/** Coming or running: what the list and the rail show. */
export function isComing(event: ServerEvent): boolean {
  return event.state === "upcoming" || event.state === "live";
}

export const soonestFirst = (left: ServerEvent, right: ServerEvent) =>
  Date.parse(left.startsAt) - Date.parse(right.startsAt);
