import { useSyncExternalStore } from "react";

import type { Announcer } from "./announcer";

export interface DragAnnouncerProps {
  /** How to drag each kind of handle with the keyboard, each read once when such a handle is focused. */
  instructions: readonly { id: string; text: string }[];
  /** The latest pick-up, move, drop or cancel. */
  announcer: Announcer;
}

/**
 * The screen-reader side of a drag: the instructions its handles point at,
 * and a live region that says what each step did. It reads the announcer
 * itself, so a step re-renders only this.
 */
export function DragAnnouncer({ instructions, announcer }: DragAnnouncerProps) {
  const announcement = useSyncExternalStore(announcer.subscribe, announcer.read, announcer.read);
  return (
    <>
      {instructions.map(({ id, text }) => (
        <span key={id} id={id} hidden>
          {text}
        </span>
      ))}
      <span
        className="sr-only live-contained"
        role="status"
        aria-live="assertive"
        aria-atomic="true"
      >
        {announcement}
      </span>
    </>
  );
}
