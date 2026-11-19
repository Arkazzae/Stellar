import type { ReactNode, RefCallback } from "react";

import { Icon } from "@stellarr/icons";
import { CollapsibleGroup } from "@/components/shell/rail/CollapsibleGroup";
import type { DragHandleProps } from "@/lib/drag/useRailDrag";
import { useLanguage } from "@/state/hooks";

export interface RailSectionProps {
  /** Id of the body; the caption button points at it with `aria-controls`. */
  id: string;
  name: string;
  open: boolean;
  onToggle: () => void;
  /** The section holds no channel; says so instead of showing nothing. */
  empty: boolean;
  /**
   * The section's own name, permissions and deletion. The only action on the
   * caption: reordering and creating live in server settings, and a 248 px
   * caption keeps just the way in.
   */
  onOpenSettings?: () => void;
  /**
   * The section's lists, rendered even when empty: an empty list still marks
   * where a channel dragged into the section would go.
   */
  children: ReactNode;
  /** The section as the rail sorts it, caption and channels together. */
  itemRef?: RefCallback<HTMLElement>;
  /**
   * Makes the caption the handle the rail sorts sections by: a press has to
   * travel a few pixels before it is a drag, so a click still opens or
   * closes the section.
   */
  drag?: DragHandleProps;
}

/**
 * A named, collapsible group of channels, under the same caption as the text
 * and voice groups: one look for every group in the rail. The wrapper is what
 * the rail sorts, caption and channels together.
 */
export function RailSection({
  id,
  name,
  open,
  onToggle,
  empty,
  onOpenSettings,
  children,
  itemRef,
  drag,
}: RailSectionProps) {
  const { t } = useLanguage();
  return (
    <div className="rail-section" ref={itemRef}>
      <CollapsibleGroup
        id={id}
        // Its own element, so a long name can end in an ellipsis.
        caption={<span className="rail-section-name">{name}</span>}
        open={open}
        onToggle={onToggle}
        drag={drag}
        actions={
          onOpenSettings === undefined ? null : (
            <button
              type="button"
              className="rail-section-cog"
              title={t("settings.sectionSettings")}
              aria-label={t("section.settingsNamed", { name })}
              onClick={onOpenSettings}
            >
              <Icon name="gear" size={14} />
            </button>
          )
        }
      >
        {children}
        {empty ? <p className="rail-section-empty">{t("section.empty")}</p> : null}
      </CollapsibleGroup>
    </div>
  );
}
