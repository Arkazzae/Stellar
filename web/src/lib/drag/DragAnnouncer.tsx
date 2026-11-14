"use client";

export interface DragAnnouncerProps {
  /** Referenced by every handle's `aria-describedby`. */
  id: string;
  /** How to drag with the keyboard, read once when a handle is focused. */
  instructions: string;
  /** The latest pick-up, move, drop or cancel. */
  announcement: string;
}

/**
 * The screen-reader side of a drag list: the instructions its handles point
 * at, and a live region that says what each step did. Each list renders its
 * own, so a drag in one never speaks over the rail's move status.
 */
export function DragAnnouncer({ id, instructions, announcement }: DragAnnouncerProps) {
  return (
    <>
      <span id={id} hidden>
        {instructions}
      </span>
      <span className="sr-only" role="status" aria-live="assertive" aria-atomic="true">
        {announcement}
      </span>
    </>
  );
}
