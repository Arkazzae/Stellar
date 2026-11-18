import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent,
} from "react";

import { spanAt, stepIndex } from "./geometry";
import { trackPointer } from "./pointer";
import {
  carryKey,
  closeSession,
  followScroll,
  openSession,
  restoreFocus,
  reveal,
  watchPage,
  type DragHandleProps,
  type DragSession,
} from "./session";

/** What a screen reader hears at each step. */
export interface ZoneDragMessages<T> {
  lifted: (item: T) => string;
  /** Undefined says nothing. */
  over: (zone: string) => string | undefined;
  dropped: (item: T, zone: string | null) => string;
  cancelled: (item: T) => string;
}

export interface ZoneDragOptions<T> {
  /** Keys of every item that can be carried now; a carried key that leaves ends the drag. */
  items: readonly string[];
  /** The drop zones in display order, which is the order the arrows walk. */
  zones: readonly string[];
  /** The zone an item starts in, where the keyboard begins. */
  homeOf: (item: T) => string;
  /** A release; `zone` is null when it was over none. Not called for a cancel. */
  onDrop: (item: T, zone: string | null) => void;
  messages: ZoneDragMessages<T>;
  /** Read out in place of the handle's own role. */
  roleDescription: string;
}

interface Carry<T> {
  key: string;
  item: T;
  via: "pointer" | "keyboard";
  over: string | null;
  /** The handle's box when the drag began, where the overlay is drawn from. */
  box: { top: number; left: number; width: number; height: number };
  /** How far below that box the overlay is drawn. */
  dy: number;
}

/**
 * An item carried onto one of a set of drop zones, with the pointer or the
 * keyboard.
 *
 * The item stays in place and a copy follows the pointer up and down. The zone
 * under it is found from the pointer's height alone, since the copy cannot
 * move sideways: a pointer that drifts off the list still targets the zone the
 * copy is over. Outside every zone there is no target, and a release there
 * moves nothing. The arrow keys jump from zone to zone.
 */
export function useZoneDrag<T>({
  items,
  zones,
  homeOf,
  onDrop,
  messages,
  roleDescription,
}: ZoneDragOptions<T>) {
  const instructionsId = useId();
  const [carry, setCarry] = useState<Carry<T> | null>(null);
  const [announcement, setAnnouncement] = useState("");
  // Mirrors `carry` for the pointer listeners, which outlive the render that
  // created them.
  const latest = useRef<Carry<T> | null>(null);
  const session = useRef<DragSession | null>(null);
  const zoneNodes = useRef(new Map<string, HTMLElement>());
  const options = useRef({ zones, homeOf, onDrop, messages });
  /** A Space or Enter that picked up or dropped; its keyup must not activate anything. */
  const keyTaken = useRef(false);
  /** The handle a keyboard drag ended on, which keeps focus after the drop. */
  const refocus = useRef<HTMLElement | null>(null);

  // The person left the call or was moved by someone else mid-drag.
  if (carry !== null && !items.includes(carry.key)) {
    setCarry(null);
    setAnnouncement(messages.cancelled(carry.item));
  }

  useLayoutEffect(() => {
    options.current = { zones, homeOf, onDrop, messages };
  });

  useLayoutEffect(() => {
    if (carry !== null) return;
    latest.current = null;
    closeSession(session);
    restoreFocus(refocus.current);
    refocus.current = null;
  }, [carry]);

  useEffect(() => () => closeSession(session), []);

  const update = (next: Carry<T>) => {
    latest.current = next;
    setCarry(next);
  };

  const zoneTop = (zone: string) => zoneNodes.current.get(zone)?.getBoundingClientRect().top;

  const zoneAt = (y: number): string | null => {
    const known = options.current.zones.flatMap((zone) => {
      const node = zoneNodes.current.get(zone);
      if (node === undefined) return [];
      const { top, height } = node.getBoundingClientRect();
      return [{ zone, top, height }];
    });
    return known[spanAt(known, y)]?.zone ?? null;
  };

  const lift = (next: Carry<T>) => {
    update(next);
    setAnnouncement(options.current.messages.lifted(next.item));
  };

  const moveOver = (current: Carry<T>, over: string | null, dy: number) => {
    if (over === current.over && dy === current.dy) return;
    update({ ...current, over, dy });
    if (over === current.over || over === null) return;
    const said = options.current.messages.over(over);
    if (said !== undefined) setAnnouncement(said);
  };

  const finish = (dropped: boolean) => {
    const current = latest.current;
    if (current === null) return;
    latest.current = null;
    setCarry(null);
    closeSession(session);
    const { messages: say, onDrop: drop } = options.current;
    if (!dropped) {
      setAnnouncement(say.cancelled(current.item));
      return;
    }
    setAnnouncement(say.dropped(current.item, current.over));
    drop(current.item, current.over);
  };

  /** Pointer drags: the copy follows the pointer, and the zone under it is the target. */
  const follow = (active: DragSession) => {
    const current = latest.current;
    if (current?.via !== "pointer") return;
    moveOver(current, zoneAt(active.y), active.y - active.originY);
  };

  /** Keyboard drags: the copy sits on the targeted zone, wherever the list has scrolled. */
  const align = (current: Carry<T>, over: string) => {
    const top = zoneTop(over);
    moveOver(current, over, top === undefined ? current.dy : top - current.box.top);
  };

  const pressPointer = (key: string, item: T, event: PointerEvent<HTMLElement>) => {
    if (!event.isPrimary || event.button !== 0 || latest.current !== null) return;
    // A press that was released without becoming a drag left its session here.
    closeSession(session);
    const handle = event.currentTarget;
    const active = openSession(handle, event.clientY);
    session.current = active;
    active.pointer = trackPointer(event.nativeEvent, handle, {
      onStart: () => {
        if (session.current !== active) return;
        const { top, left, width, height } = handle.getBoundingClientRect();
        followScroll(active, () => follow(active));
        lift({
          key,
          item,
          via: "pointer",
          over: zoneAt(active.y),
          box: { top, left, width, height },
          dy: active.y - active.originY,
        });
      },
      onMove: (y) => {
        if (session.current !== active) return;
        active.y = y;
        follow(active);
        active.edge.follow(y);
      },
      onDrop: () => {
        if (session.current === active) finish(true);
      },
      onCancel: () => {
        if (session.current === active) finish(false);
      },
    });
  };

  const keyDown = (key: string, item: T, event: KeyboardEvent<HTMLElement>) => {
    const current = latest.current;
    if (current === null) {
      const starts = event.key === " " || event.key === "Enter";
      if (!starts || event.repeat || event.target !== event.currentTarget) return;
      event.preventDefault();
      keyTaken.current = true;
      closeSession(session);
      const handle = event.currentTarget;
      const active = openSession(handle, 0);
      session.current = active;
      watchPage(active.listening.signal, handle, () => {
        if (session.current === active) finish(false);
      });
      // Re-align the copy when the list scrolls under it, by wheel or by `reveal`.
      active.scroller?.addEventListener(
        "scroll",
        () => {
          const now = latest.current;
          if (session.current === active && now?.over) align(now, now.over);
        },
        { signal: active.listening.signal },
      );
      const { top, left, width, height } = handle.getBoundingClientRect();
      lift({
        key,
        item,
        via: "keyboard",
        over: options.current.homeOf(item),
        box: { top, left, width, height },
        dy: 0,
      });
      return;
    }
    if (current.via !== "keyboard" || current.key !== key) return;
    const action = carryKey(event);
    if (action === null) return;
    event.preventDefault();
    if (action === "drop" || action === "cancel") {
      if (event.key === " " || event.key === "Enter") keyTaken.current = true;
      refocus.current = event.currentTarget;
      finish(action === "drop");
      return;
    }
    const all = options.current.zones;
    const from = all.indexOf(current.over ?? options.current.homeOf(current.item));
    const next = all[stepIndex(from === -1 ? 0 : from, action === "down" ? 1 : -1, all.length)];
    if (next === undefined || next === current.over) return;
    const zone = zoneNodes.current.get(next)?.getBoundingClientRect();
    if (zone !== undefined) reveal(session.current, { top: zone.top, bottom: zone.bottom });
    align(current, next);
  };

  const keyUp = (event: KeyboardEvent<HTMLElement>) => {
    if ((event.key !== " " && event.key !== "Enter") || !keyTaken.current) return;
    keyTaken.current = false;
    event.preventDefault();
  };

  /** Where to draw the copy: over the handle at first, then moved by `dy`. */
  const copyStyle = ({ box, dy }: Carry<T>): CSSProperties => ({
    ...box,
    transform: `translateY(${dy}px)`,
  });

  return {
    /** The item being carried, with where to draw its copy. */
    carried:
      carry === null ? null : { key: carry.key, item: carry.item, style: copyStyle(carry) },
    over: carry?.over ?? null,
    announcement,
    instructionsId,
    zoneRef: (zone: string) => (node: HTMLElement | null) => {
      if (node === null) zoneNodes.current.delete(zone);
      else zoneNodes.current.set(zone, node);
    },
    handle: (key: string, item: T): DragHandleProps => ({
      onPointerDown: (event) => pressPointer(key, item, event),
      onKeyDown: (event) => keyDown(key, item, event),
      onKeyUp: keyUp,
      onBlur: () => {
        // Keys go to the focused handle, so a carry it can no longer hear ends.
        const current = latest.current;
        if (current?.via === "keyboard" && current.key === key) finish(false);
      },
      "aria-describedby": instructionsId,
      "aria-roledescription": roleDescription,
      "aria-pressed": carry?.key === key ? true : undefined,
    }),
  };
}
