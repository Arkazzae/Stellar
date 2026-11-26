import type { CSSProperties } from "react";

import { initials } from "@/lib/format/format";

/** A hue for an activity without art, the same for the same install everywhere. */
export function toneOf(install: string): number {
  let hash = 0;
  for (const char of install) hash = (hash * 31 + (char.codePointAt(0) ?? 0)) >>> 0;
  return hash % 360;
}

export interface ActivityIconProps {
  install: string;
  name: string;
  icon: string | null;
  /** The square's side in pixels. */
  size?: number;
  className?: string;
}

/**
 * An activity's mark wherever it is named: its own icon, or its initials on
 * a flat tone of its own. Decorative, since its name is always beside it.
 */
export function ActivityIcon({ install, name, icon, size = 20, className }: ActivityIconProps) {
  const style = { "--act-size": `${size}px`, "--act-hue": toneOf(install) } as CSSProperties;
  const classes = className === undefined ? "act-icon" : `act-icon ${className}`;
  return icon === null ? (
    <span className={`${classes} act-icon-tone`} style={style} aria-hidden="true">
      {initials(name)}
    </span>
  ) : (
    <img className={classes} style={style} src={icon} alt="" loading="lazy" />
  );
}
