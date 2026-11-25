import { useState } from "react";

import { Icon } from "@stellarr/icons";

import {
  AccountPanel,
  type AccountPanelProps,
} from "@/components/account/AccountPanel";
import { Avatar } from "@/components/primitives/Avatar";
import { IconButton } from "@/components/primitives/IconButton";
import { Popover } from "@/components/primitives/Popover";
import { CustomStatusText } from "@/components/profile/ProfileBio";
import type { PersonVM } from "@/components/view-models";
import { useVoiceSeat } from "@/components/voice/useVoiceSeat";
import { SELF_PRESENCE_LABEL } from "@/ports";
import { useLanguage } from "@/state/hooks";

export interface AccountRowProps {
  user: PersonVM;
  onOpenSettings: () => void;
  /** Everything the panel above the row needs; the row only opens it. */
  panel: AccountPanelProps;
}

/** The signed-in user at the foot of the rail, and the handle of the account panel. */
export function AccountRow({
  user,
  onOpenSettings,
  panel,
}: AccountRowProps) {
  const { t } = useLanguage();
  const [open, setOpen] = useState(false);

  /** Signing out ends the visit; a status or a presence keeps the panel up. */
  const closing = (action: () => void) => () => {
    action();
    setOpen(false);
  };

  const presenceLabel = t(SELF_PRESENCE_LABEL[user.presence]);
  const hasCustomStatus = panel.customStatus.trim().length > 0;
  // In a call, this row is where you see whether you are heard: the dock
  // above shows the others.
  const seat = useVoiceSeat(user.id);
  const micOff = seat?.muted === true;

  return (
    <div className="account">
      {/* The panel slides out of the row, anchored inside the rail rather than
          floating beside it. */}
      <Popover
        open={open}
        onOpenChange={setOpen}
        className="acc-pop"
        renderTrigger={(trigger) => (
          <button {...trigger} type="button" className="acc-open">
            <span className={micOff ? "acc-av muted" : "acc-av"}>
              <Avatar
                id={user.id}
                displayName={user.displayName}
                initials={user.initials}
                avatarUrl={user.avatarUrl}
                presence={user.presence}
                speaking={seat?.speaking === true && !micOff}
              />
              {micOff ? (
                <span className="acc-mic" aria-hidden="true">
                  <Icon name="micSlash" size={10} />
                </span>
              ) : null}
            </span>
            <span className="who">
              <b>{user.displayName}</b>
              <span
                className={micOff ? "astate mic-off" : "astate"}
                title={micOff ? t("shell.micOff") : panel.customStatus || presenceLabel}
              >
                {micOff ? (
                  t("shell.micOff")
                ) : hasCustomStatus ? (
                  <CustomStatusText
                    text={panel.customStatus}
                    customEmoji={panel.customEmoji}
                  />
                ) : (
                  presenceLabel
                )}
              </span>
            </span>
          </button>
        )}
      >
        <AccountPanel
          {...panel}
          // Picking a status leaves the panel open: the dot beside the name
          // confirms it, and the next thing may be a line of status text.
          onLogout={closing(panel.onLogout)}
        />
      </Popover>
      <IconButton
        className="account-settings"
        icon="gear"
        size={16}
        label={t("common.openSettings")}
        title={t("common.openSettings")}
        onClick={onOpenSettings}
      />
    </div>
  );
}
