import { spanAt, stepIndex, type Span } from "./geometry";
import { layoutTop } from "./layout";
import type { Frame, Motion, Stage } from "./motion";
import type { SortList } from "./sortMotion";

/** What a screen reader hears while something is carried onto a zone. */
export interface DropMessages {
  lifted: (key: string) => string;
  /** Undefined says nothing. */
  over: (zone: string) => string | undefined;
  dropped: (key: string, zone: string | null) => string;
  cancelled: (key: string) => string;
}

export interface DropConfig {
  /** The kind of list whose items take drops: each of those items is a zone. */
  zoneKind: string;
  /** Keys of everything that can be carried now; a carried key that leaves ends the drag. */
  carriable: readonly string[];
  /** The zone a key starts in, where the keyboard begins. */
  homeOf: (key: string) => string;
  messages: DropMessages;
  /** A release over `zone`, or over none. Not called for a cancel. */
  commit: (key: string, zone: string | null) => void;
}

interface Zone extends Span {
  id: string;
}

/** Every open zone, in rail order, measured in content pixels; null when one is not on the page. */
function measureZones(stage: Stage, lists: readonly SortList[], kind: string): Zone[] | null {
  const zones: Zone[] = [];
  for (const list of lists) {
    if (list.kind !== kind || list.closed) continue;
    for (const id of list.items) {
      const node = stage.items.get(id);
      if (node === undefined) return null;
      zones.push({ id, top: layoutTop(node, stage.scroller), height: node.offsetHeight });
    }
  }
  return zones;
}

/** The copy that follows the pointer: React renders it, the drag places it. */
export interface CopyHost {
  /** Says what is carried, or null, to whoever renders the copy. */
  carry: (key: string | null) => void;
}

/**
 * Something carried onto one of the zones, as a person onto a voice channel,
 * with the pointer or the keyboard.
 *
 * The thing stays in place and a copy follows the pointer up and down. The
 * zone under it is found from the pointer's height alone, since the copy
 * cannot move sideways: a pointer that drifts off the list still targets the
 * zone the copy is over. Outside every zone there is no target, and a release
 * there moves nothing. The arrow keys jump from zone to zone, through every
 * open one in the rail. The zones are measured once, like a sort's rows.
 */
export function createDropMotion(
  stage: Stage,
  config: () => DropConfig & { lists: readonly SortList[] },
  key: string,
  handle: HTMLElement,
  frame: Frame,
  press: { y: number; scrollTop: number } | null,
  host: CopyHost,
): Motion & { copy: (node: HTMLElement | null) => void } | null {
  const { lists, zoneKind, homeOf } = config();
  let zones = measureZones(stage, lists, zoneKind);
  if (zones === null) return null;
  // The handle's box, where the copy starts, read before anything is written.
  const box = handle.getBoundingClientRect();
  const home = homeOf(key);
  let over: string | null = press === null ? home : null;
  let y = press?.y ?? 0;
  let scrollTop = press?.scrollTop ?? 0;
  /** How far below the handle the copy is drawn. */
  let dy = 0;
  let copy: HTMLElement | null = null;

  const place = (by: number) => {
    dy = by;
    if (copy !== null) copy.style.transform = `translateY(${by}px)`;
  };

  const zoneNode = (id: string | null) => (id === null ? undefined : stage.items.get(id));

  const moveOver = (next: string | null) => {
    if (next === over) return;
    zoneNode(over)?.removeAttribute("data-drop-over");
    zoneNode(next)?.setAttribute("data-drop-over", "");
    over = next;
    if (next === null) return;
    const said = config().messages.over(next);
    if (said !== undefined) stage.say(said);
  };

  const zoneAt = () => zones?.[spanAt(zones, y - frame.origin + scrollTop)]?.id ?? null;

  /** Keyboard drags: the copy sits on the targeted zone, wherever the list has scrolled. */
  const align = () => {
    const zone = zones?.find((each) => each.id === over);
    if (zone !== undefined) place(zone.top + frame.origin - scrollTop - box.top);
  };

  const clear = () => {
    zoneNode(over)?.removeAttribute("data-drop-over");
    delete handle.dataset.drag;
    handle.removeAttribute("aria-pressed");
    host.carry(null);
  };

  return {
    lift: () => {
      handle.dataset.drag = "lifted";
      handle.setAttribute("aria-pressed", "true");
      zoneNode(over)?.setAttribute("data-drop-over", "");
      host.carry(key);
      stage.say(config().messages.lifted(key));
    },
    copy: (node) => {
      copy = node;
      if (node === null) return;
      node.style.top = `${box.top}px`;
      node.style.left = `${box.left}px`;
      node.style.width = `${box.width}px`;
      node.style.height = `${box.height}px`;
      place(dy);
    },
    follow: (pointer, scrolled) => {
      y = pointer;
      scrollTop = scrolled;
      moveOver(zoneAt());
      place(press === null ? dy : y - press.y);
    },
    step: (direction, scrolled) => {
      if (zones === null) return null;
      scrollTop = scrolled;
      const from = zones.findIndex((zone) => zone.id === (over ?? home));
      const next = zones[stepIndex(from === -1 ? 0 : from, direction, zones.length)];
      if (next === undefined || next.id === over) return null;
      moveOver(next.id);
      align();
      const top = next.top + frame.origin - scrolled;
      return { top, bottom: top + next.height };
    },
    scrolled: (scrolled) => {
      scrollTop = scrolled;
      align();
    },
    finish: (dropped, pointer) => {
      if (dropped && pointer !== null) {
        y = pointer.y;
        scrollTop = pointer.scrollTop;
        moveOver(zoneAt());
      }
      const target = over;
      clear();
      const { messages, commit } = config();
      stage.say(dropped ? messages.dropped(key, target) : messages.cancelled(key));
      if (dropped) commit(key, target);
      return stage.handles.get(key) ?? null;
    },
    committed: () => {
      const { carriable, lists: now } = config();
      // The person left the call, or was moved by someone else.
      if (!carriable.includes(key)) return false;
      // The calls only changed size, or the channels moved: measure again.
      zones = measureZones(stage, now, zoneKind);
      if (zones === null) return false;
      if (press === null) align();
      else moveOver(zoneAt());
      return true;
    },
    clear,
  };
}
