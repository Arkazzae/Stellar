import type { KeyboardEvent, PointerEvent } from "react";

import { createAnnouncer, type Announcer } from "./announcer";
import { createDropMotion, type DropConfig } from "./dropMotion";
import { edgeScrollSpeed, revealDelta } from "./geometry";
import { carryKey, restoreFocus, watchPage } from "./keyboard";
import type { Frame, Motion, Stage } from "./motion";
import { holdFingerOn, trackPointer, type PointerDrag } from "./pointer";
import { createSortMotion, type SortConfig } from "./sortMotion";

export type {
  SortConfig,
  SortList,
  SortMessages,
  SortMove,
  SortPlace,
} from "./sortMotion";
export type { DropConfig, DropMessages } from "./dropMotion";

export interface RailDragConfig extends SortConfig {
  /** Things carried onto the items of one kind of list, as people onto voice channels. */
  drops?: DropConfig;
}

/** Sorting an item among the lists, or carrying something onto a zone. */
export type DragKind = "sort" | "drop";

type Ref = (node: HTMLElement | null) => void | (() => void);

/** A sentence, or the key of what is carried, kept outside React state. */
export type Store<T> = Pick<Announcer, "subscribe"> & { read: () => T };

interface Drag {
  kind: DragKind;
  id: string;
  via: "pointer" | "keyboard";
  finger: boolean;
  motion: Motion & { copy?: (node: HTMLElement | null) => void };
  frame: Frame;
  /** Pointer drags: the pointer's latest height in the viewport. */
  y: number;
  /**
   * The list's scroll offset, kept here rather than read in each frame: by
   * then hovering has usually invalidated the styles, and reading would
   * recompute them. Its own edge scrolling sets it, a scroll event from
   * anything else reads it.
   */
  scrollTop: number;
  maxScroll: number;
  /** The next scroll event is the drag's own edge scrolling. */
  ownScroll: boolean;
  animation: number;
  pointer: PointerDrag | null;
  listening: AbortController;
}

/**
 * Every drag of the channel rail in one place: sorting channels and sections
 * across all the lists, and carrying people onto voice channels. Only one
 * drag runs at a time, with the pointer or the keyboard.
 *
 * A drag measures what it needs when it begins and from then on only writes:
 * the pointer's moves are drawn once a frame at most, straight to the DOM,
 * React renders nothing while something is carried, and the live region
 * re-renders once per new target. The scrolling element is the container and
 * has to be positioned.
 */
export function createRailDrag() {
  const announcer = createAnnouncer();
  let carried: string | null = null;
  const carryListeners = new Set<() => void>();
  const carrying: Store<string | null> = {
    read: () => carried,
    subscribe: (listener) => {
      carryListeners.add(listener);
      return () => carryListeners.delete(listener);
    },
  };
  const host = {
    carry: (key: string | null) => {
      carried = key;
      for (const listener of carryListeners) listener();
    },
  };

  let config: RailDragConfig | null = null;
  let scroller: HTMLElement | null = null;
  const items = new Map<string, HTMLElement>();
  const lists = new Map<string, HTMLElement>();
  const handles = new Map<string, HTMLElement>();
  const refs = new Map<string, Ref>();
  let drag: Drag | null = null;
  /** A press not yet a drag, which a new press replaces. */
  let pressing: PointerDrag | null = null;
  /** A Space or Enter that picked up or dropped; its keyup must not click anything. */
  let keyTaken = false;

  /** A ref callback per element and id, the same every render, so React does not detach and attach it. */
  const cached = (key: string, make: () => Ref): Ref => {
    let ref = refs.get(key);
    if (ref === undefined) {
      ref = make();
      refs.set(key, ref);
    }
    return ref;
  };
  const registry = (map: Map<string, HTMLElement>, id: string) => (node: HTMLElement | null) => {
    if (node === null) map.delete(id);
    else map.set(id, node);
  };

  const currentConfig = () => {
    if (config === null) throw new Error("The rail drag is not configured.");
    return config;
  };

  const viewOf = (node: Element) => node.ownerDocument.defaultView ?? window;

  /** One frame of a pointer drag. Nothing is read from the page, so nothing waits for a layout. */
  const tick = (current: Drag) => {
    current.animation = 0;
    const list = scroller;
    if (drag !== current || list === null) return;
    current.motion.follow(current.y, current.scrollTop);
    const speed = edgeScrollSpeed(current.y, current.frame.view);
    const step = Math.sign(speed) * Math.max(1, Math.round(Math.abs(speed)));
    const scrollTop = Math.min(current.maxScroll, Math.max(0, current.scrollTop + step));
    // At the end of the list there is nothing left to scroll; the next move
    // starts the loop again if the pointer heads back.
    if (speed === 0 || scrollTop === current.scrollTop) return;
    // Scrolling has to lay the page out first. Once this frame is drawn there
    // is nothing left to lay out, and the next frame draws the scroll and what
    // is carried over it together.
    viewOf(list).setTimeout(() => {
      if (drag !== current) return;
      current.scrollTop = scrollTop;
      current.ownScroll = true;
      list.scrollTop = scrollTop;
      request(current);
    }, 0);
  };

  const request = (current: Drag) => {
    if (current.animation === 0 && scroller !== null) {
      current.animation = viewOf(scroller).requestAnimationFrame(() => tick(current));
    }
  };

  /** Starts a drag; `pressedAt` is the viewport height a pointer pressed at. */
  const begin = (
    kind: DragKind,
    id: string,
    handle: HTMLElement,
    finger: boolean,
    pressedAt: number | null,
  ): Drag | null => {
    if (config === null || scroller === null) return null;
    const list = scroller;
    const box = list.getBoundingClientRect();
    const frame: Frame = {
      origin: box.top + list.clientTop,
      view: { top: box.top, bottom: box.bottom },
    };
    const scrollTop = list.scrollTop;
    const press = pressedAt === null ? null : { y: pressedAt, scrollTop };
    const stage: Stage = { scroller: list, items, lists, handles, say: announcer.say };
    const drops = config.drops;
    const motion =
      kind === "sort"
        ? createSortMotion(stage, currentConfig, id, handle, frame, press)
        : drops === undefined
          ? null
          : createDropMotion(
              stage,
              () => ({ ...(currentConfig().drops ?? drops), lists: currentConfig().lists }),
              id,
              handle,
              frame,
              press,
              host,
            );
    if (motion === null) return null;
    const current: Drag = {
      kind,
      id,
      via: pressedAt === null ? "keyboard" : "pointer",
      finger,
      motion,
      frame,
      y: pressedAt ?? 0,
      scrollTop,
      maxScroll: list.scrollHeight - list.clientHeight,
      ownScroll: false,
      animation: 0,
      pointer: null,
      listening: new AbortController(),
    };
    drag = current;
    // Everything is read; from here on the drag only writes.
    list.dataset.dragging = kind;
    motion.lift();
    return current;
  };

  const stop = (current: Drag) => {
    if (current.animation !== 0 && scroller !== null) {
      viewOf(scroller).cancelAnimationFrame(current.animation);
    }
    current.animation = 0;
    current.listening.abort();
    current.pointer?.cancel();
  };

  const finish = (dropped: boolean) => {
    const current = drag;
    if (current === null) return;
    drag = null;
    stop(current);
    if (scroller !== null) delete scroller.dataset.dragging;
    const focus = current.motion.finish(
      dropped,
      current.via === "pointer" ? { y: current.y, scrollTop: current.scrollTop } : null,
    );
    if (current.via === "keyboard") restoreFocus(focus);
  };

  const pressPointer = (kind: DragKind, id: string, event: PointerEvent<HTMLElement>) => {
    if (!event.isPrimary || event.button !== 0 || drag !== null) return;
    pressing?.cancel();
    const handle = event.currentTarget;
    const pressedAt = event.clientY;
    const finger = event.pointerType === "touch";
    let current: Drag | null = null;
    const press = trackPointer(event.nativeEvent, handle, {
      onStart: () => {
        pressing = null;
        current = begin(kind, id, handle, finger, pressedAt);
        if (current === null) {
          press.cancel();
          return;
        }
        const started = current;
        started.pointer = press;
        // Edge scrolling and the wheel move the list under a still pointer.
        const list = scroller;
        list?.addEventListener(
          "scroll",
          () => {
            if (started.ownScroll) started.ownScroll = false;
            else started.scrollTop = list.scrollTop;
            request(started);
          },
          { signal: started.listening.signal },
        );
      },
      onMove: (y) => {
        if (current === null || drag !== current) return;
        current.y = y;
        request(current);
      },
      onDrop: () => {
        if (current !== null && drag === current) finish(true);
      },
      onCancel: () => {
        if (current !== null && drag === current) finish(false);
      },
    });
    pressing = press;
  };

  /** Space picks up a sorted item; Space or Enter a carried one, whose row does nothing else. */
  const starts = (kind: DragKind, event: KeyboardEvent<HTMLElement>) =>
    event.key === " " || (kind === "drop" && event.key === "Enter");

  const keyDown = (kind: DragKind, id: string, event: KeyboardEvent<HTMLElement>) => {
    const current = drag;
    const list = scroller;
    if (current === null) {
      if (!starts(kind, event) || event.repeat || event.target !== event.currentTarget) return;
      event.preventDefault();
      keyTaken = true;
      pressing?.cancel();
      const handle = event.currentTarget;
      const started = begin(kind, id, handle, false, null);
      if (started === null || list === null) return;
      watchPage(started.listening.signal, handle, () => {
        if (drag === started) finish(false);
      });
      list.addEventListener("scroll", () => started.motion.scrolled(list.scrollTop), {
        signal: started.listening.signal,
      });
      return;
    }
    if (current.via !== "keyboard" || current.kind !== kind || current.id !== id) return;
    const key = carryKey(event);
    if (key === null) return;
    event.preventDefault();
    if (key === "drop" || key === "cancel") {
      if (event.key === " " || event.key === "Enter") keyTaken = true;
      finish(key === "drop");
      return;
    }
    if (list === null) return;
    const scrollTop = list.scrollTop;
    const shown = current.motion.step(key === "down" ? 1 : -1, scrollTop);
    // Scrolls just far enough for the new place to be in view.
    const delta = shown === null ? 0 : revealDelta(shown, current.frame.view);
    if (delta !== 0) list.scrollTop = scrollTop + delta;
  };

  return {
    announcer,
    carrying,
    configure: (next: RailDragConfig) => {
      config = next;
    },
    /**
     * After every render of the rail. A drag that can no longer go on, as
     * when someone else reorders the lists or the carried person leaves the
     * call, ends; one whose lists only changed size is measured again.
     */
    committed: () => {
      const current = drag;
      if (current !== null && !current.motion.committed()) finish(false);
    },
    dispose: () => {
      const current = drag;
      drag = null;
      pressing?.cancel();
      if (current === null) return;
      stop(current);
      if (scroller !== null) delete scroller.dataset.dragging;
      current.motion.clear();
    },
    container: (node: HTMLElement | null) => {
      scroller = node;
    },
    list: (id: string) => cached(`list:${id}`, () => registry(lists, id)),
    item: (id: string) => cached(`item:${id}`, () => registry(items, id)),
    /** Where React puts the copy of what is carried onto a zone. */
    copy: (node: HTMLElement | null) => {
      drag?.motion.copy?.(node);
    },
    handleRef: (id: string) =>
      cached(`handle:${id}`, () => (node: HTMLElement | null) => {
        if (node === null) return;
        handles.set(id, node);
        const release = holdFingerOn(node, () => drag?.finger === true);
        return () => {
          release();
          if (handles.get(id) === node) handles.delete(id);
        };
      }),
    pressPointer,
    keyDown,
    keyUp: (event: KeyboardEvent<HTMLElement>) => {
      if ((event.key !== " " && event.key !== "Enter") || !keyTaken) return;
      keyTaken = false;
      event.preventDefault();
    },
    /** Keys go to the focused handle, so a carry it can no longer hear ends. */
    blur: (id: string) => {
      if (drag?.via === "keyboard" && drag.id === id) finish(false);
    },
  };
}

export type RailDrag = ReturnType<typeof createRailDrag>;
