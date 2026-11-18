import type { ReactNode } from "react";

import { Icon } from "@stellarr/icons";
import { IconButton } from "@/components/primitives/IconButton";
import { useLanguage } from "@/state/LanguageProvider";

export interface DmChatPanelProps {
  /** Shut, the panel is out of the layout or an off-screen drawer. */
  open: boolean;
  onClose: () => void;
  /** The conversation's messages and composer. */
  children: ReactNode;
}

/**
 * The conversation beside its call, in the profile's column and the roster's
 * shape: the same header, a drawer where the roster is one.
 *
 * The messages are mounted only while it is open. A transcript lines itself
 * up with its newest message when it mounts, which it cannot do in a panel
 * that has no width, and the room has no second copy of it during a call.
 */
export function DmChatPanel({ open, onClose, children }: DmChatPanelProps) {
  const { t } = useLanguage();

  return (
    // `inert` is how a closed panel is hidden, as on the roster.
    <aside className="roster dm-chat" aria-label={t("dm.chat")} inert={!open}>
      <div className="roster-inner">
        <div className="roster-head">
          <span className="glyph" aria-hidden="true">
            <Icon name="thread" size={18} />
          </span>
          <h3>{t("dm.chat")}</h3>
          <IconButton
            icon="x"
            size={17}
            label={t("dm.closeChat")}
            title={t("dm.closeChat")}
            onClick={onClose}
          />
        </div>
        <div className="dm-chat-body">{open ? children : null}</div>
      </div>
    </aside>
  );
}
