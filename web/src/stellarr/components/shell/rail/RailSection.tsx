import type { ReactNode } from "react";

import { Icon } from "@stellarr/icons";
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
}

/**
 * A named, collapsible group of channels.
 *
 * Not a `CollapsibleGroup`: the caption holds the folder mark, the name and
 * the caret as direct children of its button.
 */
export function RailSection({
  id,
  name,
  open,
  onToggle,
  empty,
  onOpenSettings,
  children,
}: RailSectionProps) {
  const { t } = useLanguage();
  return (
    <div className="rail-section">
      <div className="rail-cap rail-section-cap">
        <button
          type="button"
          className="group-caption"
          aria-expanded={open}
          aria-controls={id}
          onClick={onToggle}
        >
          <span className="rail-section-mark" aria-hidden="true">
            <Icon name="folder" size={12} />
          </span>
          <span className="rail-section-name">{name}</span>
          <span className="caret">
            <Icon name="caret" size={10} />
          </span>
        </button>
        {onOpenSettings === undefined ? null : (
          <button
            type="button"
            className="rail-section-cog"
            title={t("settings.sectionSettings")}
            aria-label={t("section.settingsNamed", { name })}
            onClick={onOpenSettings}
          >
            <Icon name="gear" size={14} />
          </button>
        )}
      </div>
      <div id={id} className={open ? "collapsible" : "collapsible closed"} inert={!open}>
        <div className="collapsible-body">
          {children}
          {empty ? <p className="rail-section-empty">{t("section.empty")}</p> : null}
        </div>
      </div>
    </div>
  );
}
