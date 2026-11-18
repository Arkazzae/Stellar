import type { ReactNode } from "react";

import { Sticker, type StickerName } from "@/components/primitives/MascotArt";

export interface FriendsSectionProps {
  caption: string;
  count: number;
  /** Shown instead of the list when there is nothing in it. */
  empty: ReactNode;
  children: ReactNode;
}

/** A caption with its count, then the list, or the panel saying why there is none. */
export function FriendsSection({ caption, count, empty, children }: FriendsSectionProps) {
  return (
    <section className="fr-section">
      <h3 className="group-caption">
        {caption}
        <span className="count">{count}</span>
      </h3>
      {count === 0 ? empty : <ul className="fr-list">{children}</ul>}
    </section>
  );
}

export interface FriendsEmptyProps {
  /** The mascot marking the moment, from `MESSAGES_STICKERS`. */
  sticker: StickerName;
  title: string;
  detail: string;
  /** The button that fills the list, when there is one. */
  action?: ReactNode;
}

/**
 * An empty list in the shape the rest of the app uses for one: a mark, what is
 * empty, a sentence about it and the action that fixes it. A bare grey line
 * would read as missing content rather than as a state. The mark is the
 * mascot, centred over the text.
 */
export function FriendsEmpty({ sticker, title, detail, action }: FriendsEmptyProps) {
  return (
    <div className="fr-empty">
      <span className="fr-empty-art">
        <Sticker name={sticker} size="main" align="center" />
      </span>
      <b>{title}</b>
      <span className="fr-empty-detail">{detail}</span>
      {action === undefined ? null : <span className="fr-empty-act">{action}</span>}
    </div>
  );
}
