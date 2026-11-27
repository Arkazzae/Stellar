import { ScheduledDialog } from "@/components/scheduled/ScheduledDialog";

import { goesTo, useScheduledItems, useScheduledListActions, type ScheduledPlace } from "./useScheduled";

export interface ScheduledDialogContainerProps {
  place: ScheduledPlace;
  /** When it opened, which "today" and "tomorrow" in the list are read against. */
  now: Date;
  zone: string;
  onClose(): void;
}

/** "Zaplanowane" for one field's place, from the bar over it. Mounted only while open. */
export function ScheduledDialogContainer({ place, now, zone, onClose }: ScheduledDialogContainerProps) {
  const items = useScheduledItems();
  const actions = useScheduledListActions();
  return (
    <ScheduledDialog
      items={items.filter((item) => goesTo(item, place))}
      now={now}
      zone={zone}
      {...actions}
      onClose={onClose}
    />
  );
}
