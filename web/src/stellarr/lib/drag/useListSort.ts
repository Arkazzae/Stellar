import {
  useEffect,
  useId,
  useLayoutEffect,
  useState,
  type KeyboardEvent,
  type PointerEvent,
  type RefCallback,
} from "react";
import { flushSync } from "react-dom";

import {
  createListSort,
  type SortList,
  type SortMessages,
  type SortMove,
} from "./listSort";

export type { SortList, SortMessages, SortMove, SortPlace } from "./listSort";

export interface ListSortOptions {
  /** Every list, in display order. */
  lists: readonly SortList[];
  onMove: (move: SortMove) => void;
  messages: SortMessages;
  /** Read out in place of the handle's own role. */
  roleDescription: string;
  /** Where focus goes after a keyboard drop into a closed list. */
  focusClosed?: (list: string) => HTMLElement | null;
}

/** What a sort handle spreads onto its element, beside its own label and role. */
export interface SortHandleProps {
  ref: RefCallback<HTMLElement>;
  onPointerDown: (event: PointerEvent<HTMLElement>) => void;
  onKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
  onKeyUp: (event: KeyboardEvent<HTMLElement>) => void;
  onBlur: () => void;
  "aria-describedby": string;
  "aria-roledescription": string;
}

/**
 * Lists sorted as one, by pointer or keyboard; see `createListSort`.
 *
 * `container` goes on the scrolling element, `list` on each list's own
 * element, `item` on the element of an item that moves (a row with whatever
 * it carries) and `handle` on the part that picks it up. The handle can be
 * the row's main button: a press has to travel a few pixels before it is a
 * drag, so a click still clicks. Space picks an item up, the arrows move it
 * through every list of its kind in turn, Space drops it and Escape puts it
 * back.
 */
export function useListSort({
  lists,
  onMove,
  messages,
  roleDescription,
  focusClosed,
}: ListSortOptions) {
  const instructionsId = useId();
  const [sort] = useState(createListSort);

  useLayoutEffect(() => {
    sort.configure({
      lists,
      messages,
      focusClosed,
      // The drop is rendered before the item settles into it and before
      // focus is put back on it, both of which need its new place.
      commit: (move) => flushSync(() => onMove(move)),
    });
    sort.committed();
  });

  useEffect(() => () => sort.dispose(), [sort]);

  return {
    announcer: sort.announcer,
    instructionsId,
    container: sort.container,
    list: sort.list,
    item: sort.item,
    handle: (id: string): SortHandleProps => ({
      ref: sort.handleRef(id),
      onPointerDown: (event) => sort.pressPointer(id, event),
      onKeyDown: (event) => sort.keyDown(id, event),
      onKeyUp: sort.keyUp,
      onBlur: () => sort.blur(id),
      "aria-describedby": instructionsId,
      "aria-roledescription": roleDescription,
    }),
  };
}

export type ListSortHandle = ReturnType<typeof useListSort>;
