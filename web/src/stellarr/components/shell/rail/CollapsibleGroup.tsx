import type { ReactNode } from "react";

import { Icon } from "@stellarr/icons";
import type { DragHandleProps } from "@/lib/drag/useRailDrag";

export interface CollapsibleGroupProps {
  /** Id of the body; the caption button points at it with `aria-controls`. */
  id: string;
  caption: ReactNode;
  captionTitle?: string;
  open: boolean;
  onToggle: () => void;
  /** Controls beside the caption, such as the "+" that creates a channel. */
  actions?: ReactNode;
  /** Makes the caption the handle its group is sorted by; a click still toggles it. */
  drag?: DragHandleProps;
  children: ReactNode;
}

/**
 * The caption and collapsible body shared by every rail group: text channels,
 * voice channels, and each section that wraps a pair of them.
 *
 * CSS collapses the body to a zero-height grid row, which leaves its rows
 * focusable, so `inert` is what takes a closed group out of the tab order.
 */
export function CollapsibleGroup({
  id,
  caption,
  captionTitle,
  open,
  onToggle,
  actions,
  drag,
  children,
}: CollapsibleGroupProps) {
  return (
    <>
      <div className="rail-cap">
        <button
          type="button"
          className="group-caption"
          aria-expanded={open}
          aria-controls={id}
          title={captionTitle}
          {...drag}
          onClick={onToggle}
        >
          {caption}{" "}
          <span className="caret">
            <Icon name="caret" size={10} />
          </span>
        </button>
        {actions}
      </div>
      <div id={id} className={open ? "collapsible" : "collapsible closed"} inert={!open}>
        <div className="collapsible-body">{children}</div>
      </div>
    </>
  );
}
