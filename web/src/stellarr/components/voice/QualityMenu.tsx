import { Fragment, useId, type ReactNode } from "react";

import { Icon } from "@stellarr/icons";
import { Popover, type PopoverTriggerProps } from "@/components/primitives/Popover";
import { ProgressRing } from "@/components/primitives/ProgressRing";

/**
 * One choice. It carries its own handler instead of a value handed back to the
 * menu, so one menu can hold settings of different types without a cast.
 */
export interface QualityOption {
  id: string;
  label: string;
  /** The short right-hand column, such as "720p · 30 kl./s" or "≈64 kb/s". */
  detail?: string;
  selected: boolean;
  disabled?: boolean;
  /**
   * Its files on their way, as whole percent or null while the total is
   * unknown: the row ends in a ring in place of the tick.
   */
  download?: { percent: number | null };
  onSelect: () => void;
}

/** A heading and the choices under it. */
export interface QualityGroup {
  caption: string;
  options: readonly QualityOption[];
  /** One faint line under the choices, about the one that is chosen. */
  hint?: string;
}

export interface QualityMenuProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  renderTrigger: (props: PopoverTriggerProps) => ReactNode;
  title: string;
  /** What is in force now, shown under the title. */
  summary: string;
  groups?: readonly QualityGroup[];
  /** Explains why the body is empty or limited. */
  note?: string;
  /** The way out to the full settings dialog. */
  footer?: { label: string; onSelect: () => void };
  /** Extra class on the panel, such as `mic-menu`. */
  className?: string;
}

function optionDetail(option: QualityOption): string | undefined {
  const percent = option.download?.percent;
  return percent === undefined || percent === null ? option.detail : `${percent}%`;
}

/**
 * The drawer behind the small arrow next to the microphone, camera and screen
 * buttons: a title over what is in force, one radio group per `QualityGroup`
 * and an optional way out to the full settings. The rows are the account
 * panel's: the choice in the accent with a tick at the end of its row, a
 * hairline between groups.
 */
export function QualityMenu({
  open,
  onOpenChange,
  renderTrigger,
  title,
  summary,
  groups = [],
  note,
  footer,
  className,
}: QualityMenuProps) {
  const radioName = useId();

  return (
    <Popover
      open={open}
      onOpenChange={onOpenChange}
      className={className ? `vc-menu ${className}` : "vc-menu"}
      renderTrigger={renderTrigger}
    >
      <div className="qm-head">
        <b>{title}</b>
        <span>{summary}</span>
      </div>
      {note === undefined ? null : <p className="qm-note">{note}</p>}
      {groups.map((group, index) => (
        <Fragment key={group.caption}>
          {index === 0 ? null : <hr className="qm-sep" />}
          <div className="qm-group" role="radiogroup" aria-label={group.caption}>
            <div className="qm-caption">{group.caption}</div>
            {group.options.map((option) => {
              const detail = optionDetail(option);
              return (
                <label
                  key={option.id}
                  className={["qm-option", option.selected && "on", option.disabled && "disabled"]
                    .filter(Boolean)
                    .join(" ")}
                >
                  <input
                    className="qm-radio"
                    type="radio"
                    name={`${radioName}-${index}`}
                    checked={option.selected}
                    disabled={option.disabled}
                    aria-busy={option.download ? true : undefined}
                    onChange={option.onSelect}
                  />
                  <span className="qm-label">{option.label}</span>
                  {detail === undefined ? null : <span className="qm-detail">{detail}</span>}
                  {/* Every row keeps the slot, so the details line up. */}
                  <span className="qm-marker" aria-hidden="true">
                    {option.download ? (
                      <ProgressRing percent={option.download.percent} size={14} />
                    ) : option.selected ? (
                      <Icon name="check" size={14} />
                    ) : null}
                  </span>
                </label>
              );
            })}
          </div>
          {group.hint === undefined ? null : <p className="qm-note qm-hint">{group.hint}</p>}
        </Fragment>
      ))}
      {footer === undefined ? null : (
        <>
          <hr className="qm-sep" />
          <button
            type="button"
            className="qm-more"
            onClick={() => {
              onOpenChange(false);
              footer.onSelect();
            }}
          >
            <span className="qm-more-icon" aria-hidden="true">
              <Icon name="sliders" size={16} />
            </span>
            {footer.label}
          </button>
        </>
      )}
    </Popover>
  );
}
