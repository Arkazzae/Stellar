import { useRef, type MouseEvent, type ReactNode } from "react";

import { MessageToolbar, type MessageToolbarProps } from "./MessageToolbar";
import { pointerEntered, pointerLeft, presenceIn, type ToolbarPresence } from "./toolbarPresence";

function holds(container: Element, target: EventTarget | null): boolean {
  return target instanceof Node && container.contains(target);
}

export interface MessageEntryProps {
  actions: Omit<MessageToolbarProps, "presence">;
  onContextMenu?(event: MouseEvent<HTMLDivElement>): void;
  /** The message itself. */
  children: ReactNode;
}

/**
 * One message in a block, with its actions.
 *
 * Whether the pointer or focus is on it is read from where the nodes are, as
 * `:hover` and `:focus-within` read it: the picker and the sheet are this
 * entry's children in React but not in the document, and the style sheet,
 * which decides when the toolbar shows, does not count them either. None of it
 * is React state, so the pointer crossing a message draws nothing again in the
 * transcript; see `ToolbarPresence`.
 */
export function MessageEntry({ actions, onContextMenu, children }: MessageEntryProps) {
  const presence = useRef<ToolbarPresence | null>(null);

  return (
    <div
      className="msg-entry"
      onContextMenu={onContextMenu}
      onMouseOver={(event) => {
        if (holds(event.currentTarget, event.target)) pointerEntered(presenceIn(presence));
      }}
      onMouseOut={(event) => {
        if (!holds(event.currentTarget, event.relatedTarget)) pointerLeft(presenceIn(presence));
      }}
      // Focus can move while React is committing, as a field that focuses
      // itself does, so these draw the toolbar when React is done; a focus
      // change is drawn before the next frame either way.
      onFocus={(event) => {
        if (!holds(event.currentTarget, event.target)) return;
        const toolbar = presenceIn(presence);
        toolbar.focused = true;
        toolbar.update(false);
      }}
      onBlur={(event) => {
        const entry = event.currentTarget;
        if (holds(entry, event.relatedTarget)) return;
        const settle = () => {
          const toolbar = presenceIn(presence);
          toolbar.focused = holds(entry, document.activeElement);
          toolbar.update(false);
        };
        // The window lost focus, which keeps the page's focus where it was,
        // or focus went nowhere: what it settles on decides.
        if (event.relatedTarget === null) window.setTimeout(settle, 0);
        else settle();
      }}
    >
      <MessageToolbar {...actions} presence={presence} />
      {children}
    </div>
  );
}
