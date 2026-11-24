import { Icon } from "@stellarr/icons";
import type { CSSProperties, ReactNode } from "react";

import { imageSource } from "@/components/primitives/imageSource";
import { initials } from "@/lib/format/format";

import styles from "./AuthControls.module.css";

/**
 * A face for someone the account pages name before anyone is signed in: the
 * colour the server gave the account, the initials, and the uploaded picture
 * over them when there is one. Without a name yet it is an empty outline.
 */
export function Face({
  name,
  color,
  imageUrl = null,
  small = false,
}: {
  name: string;
  color: string;
  imageUrl?: string | null;
  small?: boolean;
}) {
  const src = imageSource(imageUrl);
  const classes = [styles.face, small ? styles.small : null, name.trim() === "" ? styles.empty : null];
  return (
    <span
      className={classes.filter(Boolean).join(" ")}
      style={{ "--face": color } as CSSProperties}
      aria-hidden="true"
    >
      {name.trim() === "" ? <Icon name="user" size={small ? 12 : 18} /> : initials(name)}
      {src === undefined ? null : <img src={src} alt="" />}
    </span>
  );
}

/**
 * A server's tile as the server rail draws the open server: its icon, or the
 * mark the rail shows (`serverMark`). Null for a server that has no name yet,
 * which draws an empty tile.
 */
export function ServerTile({
  mark,
  iconUrl = null,
  large = false,
}: {
  mark: string | null;
  iconUrl?: string | null;
  large?: boolean;
}) {
  const src = imageSource(iconUrl);
  const empty = mark === null && src === undefined;
  const classes = [styles.tile, large ? styles.large : null, empty ? styles.empty : null];
  return (
    <span className={classes.filter(Boolean).join(" ")} aria-hidden="true">
      {src !== undefined ? <img src={src} alt="" /> : empty ? <Icon name="server" size={20} /> : mark}
    </span>
  );
}

/** One account or server in a line: its face, a name, what goes under it, and an action or a tag. */
export function Chip({
  face,
  name,
  detail,
  end,
  placeholder = false,
  className,
}: {
  face: ReactNode;
  name: string;
  detail: ReactNode;
  end?: ReactNode;
  /** The name is a stand-in until something is typed. */
  placeholder?: boolean;
  className?: string;
}) {
  return (
    <div className={[styles.chip, className].filter(Boolean).join(" ")}>
      {face}
      <span className={styles.who}>
        <strong className={placeholder ? styles.placeholder : undefined}>{name}</strong>
        <small>{detail}</small>
      </span>
      {end}
    </div>
  );
}
