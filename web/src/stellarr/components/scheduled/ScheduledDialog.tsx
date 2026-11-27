import { useId } from "react";

import { cx } from "@stellarr/ui";
import { Modal } from "@/components/primitives/Modal";

import { ScheduledList, type ScheduledListProps } from "./ScheduledList";
import css from "./ScheduledDialog.module.css";

export interface ScheduledDialogProps extends Omit<ScheduledListProps, "onClose" | "titleId"> {
  onClose(): void;
}

/** "Zaplanowane" for one place, opened from the bar over its field. */
export function ScheduledDialog({ onClose, ...list }: ScheduledDialogProps) {
  const titleId = useId();
  return (
    <Modal open onClose={onClose} labelledBy={titleId} className={cx(css.wrap)} exitMs={200}>
      <div className={css.card}>
        <ScheduledList {...list} titleId={titleId} onClose={onClose} />
      </div>
    </Modal>
  );
}
