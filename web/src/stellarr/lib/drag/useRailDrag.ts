import {
  useEffect,
  useLayoutEffect,
  useState,
  type KeyboardEvent,
  type PointerEvent,
  type RefCallback,
} from "react";
import { flushSync } from "react-dom";

import {
  createRailDrag,
  type DragKind,
  type DropConfig,
  type SortConfig,
  type SortMove,
} from "./railDrag";

export type {
  DropConfig,
  DropMessages,
  SortList,
  SortMessages,
  SortMove,
  SortPlace,
} from "./railDrag";

export interface RailDragOptions extends Omit<SortConfig, "commit"> {
  onMove: (move: SortMove) => void;
  drops?: DropConfig;
}

/** How a handle is announced: its role, and the element with its instructions. */
export interface HandleDescription {
  roleDescription: string;
  instructionsId: string;
}

/** What a drag handle spreads onto its element, beside its own label and role. */
export interface DragHandleProps {
  ref: RefCallback<HTMLElement>;
  onPointerDown: (event: PointerEvent<HTMLElement>) => void;
  onKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
  onKeyUp: (event: KeyboardEvent<HTMLElement>) => void;
  onBlur: () => void;
  "aria-describedby": string;
  "aria-roledescription": string;
}

/**
 * Every drag of the rail; see `createRailDrag`.
 *
 * `container` goes on the scrolling element, `list` on each list's own
 * element, `item` on the element of an item that moves (a row with whatever
 * it carries), `handle` on the part that picks it up, `carry` on something
 * carried onto items, and `copy` on the copy of it that follows the pointer,
 * rendered while `carrying` names it. A sort handle can be a row's main
 * button: a press has to travel a few pixels before it is a drag, so a click
 * still clicks. Space picks a sorted item up, the arrows move it through
 * every list of its kind in turn, Space drops it and Escape puts it back.
 */
export function useRailDrag({ lists, onMove, messages, focusClosed, drops }: RailDragOptions) {
  const [drag] = useState(createRailDrag);

  useLayoutEffect(() => {
    drag.configure({
      lists,
      messages,
      ...(focusClosed === undefined ? {} : { focusClosed }),
      ...(drops === undefined ? {} : { drops }),
      // The drop is rendered before the item settles into it and before
      // focus is put back on it, both of which need its new place.
      commit: (move) => flushSync(() => onMove(move)),
    });
    drag.committed();
  });

  useEffect(() => () => drag.dispose(), [drag]);

  const handleOf =
    (kind: DragKind) =>
    (id: string, { roleDescription, instructionsId }: HandleDescription): DragHandleProps => ({
      ref: drag.handleRef(id),
      onPointerDown: (event) => drag.pressPointer(kind, id, event),
      onKeyDown: (event) => drag.keyDown(kind, id, event),
      onKeyUp: drag.keyUp,
      onBlur: () => drag.blur(id),
      "aria-describedby": instructionsId,
      "aria-roledescription": roleDescription,
    });

  return {
    announcer: drag.announcer,
    carrying: drag.carrying,
    container: drag.container,
    list: drag.list,
    item: drag.item,
    copy: drag.copy,
    handle: handleOf("sort"),
    carry: handleOf("drop"),
  };
}
