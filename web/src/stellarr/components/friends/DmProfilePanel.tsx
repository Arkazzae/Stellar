import type { ReactNode } from "react";

import { Icon, type IconName } from "@stellarr/icons";
import { blockPerson, removeFriend } from "@/components/confirmations";
import { Avatar } from "@/components/primitives/Avatar";
import { useConfirm, type ConfirmRequest } from "@/components/primitives/ConfirmDialog";
import { IconButton } from "@/components/primitives/IconButton";
import type { PersonVM, ProfileRelation } from "@/components/view-models";
import { useLanguage } from "@/state/LanguageProvider";

export interface ProfileActionProps {
  icon: IconName;
  label: string;
  /** A second line under the label, which turns the label into a title. */
  detail?: ReactNode;
  /** Ends something: the wash and the label turn to the danger colour. */
  danger?: boolean;
  /**
   * A control at the end of the row, such as a switch. The row is then not a
   * button itself, since a control cannot sit inside one; the control is what
   * is pressed.
   */
  trailing?: ReactNode;
  /** Receives the row, for whatever has to hang off it. */
  onClick?: (anchor: HTMLElement) => void;
}

/** One row of the profile panel's list of things to do. */
export function ProfileAction({ icon, label, detail, danger, trailing, onClick }: ProfileActionProps) {
  const className = [
    "pp-row",
    detail !== undefined && "two",
    danger && "danger",
    trailing !== undefined && "has-trailing",
  ]
    .filter(Boolean)
    .join(" ");

  const body = (
    <>
      <Icon name={icon} size={16} />
      <span className="pp-row-text">
        <span className="pp-row-label">{label}</span>
        {detail === undefined ? null : <span className="pp-row-detail">{detail}</span>}
      </span>
    </>
  );

  if (trailing !== undefined) {
    return (
      <div className={className}>
        {body}
        {trailing}
      </div>
    );
  }

  return (
    <button type="button" className={className} onClick={(event) => onClick?.(event.currentTarget)}>
      {body}
    </button>
  );
}

export interface DmProfilePanelProps {
  /** Shut, the panel is a zero-width track or an off-screen drawer. */
  open: boolean;
  person: PersonVM;
  /** Without the "@". */
  username: string;
  customStatus?: string;
  /** Formatted date the friendship began; absent for anyone who is not a friend. */
  friendsSince?: string;
  relation: ProfileRelation;
  onClose: () => void;
  /** Raises the full profile card beside the row that asked for it. */
  onOpenProfile: (anchor: HTMLElement) => void;
  onRemoveFriend: () => void;
  onBlock: () => void;
  onUnblock: () => void;
}

/**
 * Who you are talking to, beside the conversation.
 *
 * It takes the roster's column and the roster's shape (the same header, the
 * same width, a drawer where the roster is one), because it answers the same
 * question the roster answers on a channel: who is here. Below the person are
 * the facts the client already knows and what can be done about the
 * relationship; the two that end it ask first, in the words every other door
 * to them uses.
 */
export function DmProfilePanel({
  open,
  person,
  username,
  customStatus,
  friendsSince,
  relation,
  onClose,
  onOpenProfile,
  onRemoveFriend,
  onBlock,
  onUnblock,
}: DmProfilePanelProps) {
  const { t } = useLanguage();
  const confirm = useConfirm();
  const name = person.displayName;

  const confirmThen = (request: ConfirmRequest, act: () => void) => {
    void confirm(request).then((confirmed) => {
      if (confirmed) act();
    });
  };

  return (
    // `inert` is how a closed panel is hidden, as on the roster.
    <aside className="roster dm-profile" aria-label={t("common.profile")} inert={!open}>
      <div className="roster-inner">
        <div className="roster-head">
          <span className="glyph" aria-hidden="true">
            <Icon name="user" size={18} />
          </span>
          <h3>{t("common.profile")}</h3>
          <IconButton
            icon="x"
            size={17}
            label={t("dm.closeProfile")}
            title={t("dm.closeProfile")}
            onClick={onClose}
          />
        </div>

        <div className="pp-body">
          <div className="pp-who">
            <Avatar
              id={person.id}
              displayName={name}
              initials={person.initials}
              avatarUrl={person.avatarUrl}
              size="xl"
              presence={person.presence}
            />
            <h4>{name}</h4>
            <span className="pp-handle">@{username}</span>
            {customStatus === undefined ? null : <p className="pp-status">{customStatus}</p>}
          </div>

          {friendsSince === undefined ? null : (
            <dl className="pp-facts">
              <div>
                <dt>{t("dm.friendsSince")}</dt>
                <dd>{friendsSince}</dd>
              </div>
            </dl>
          )}

          <div className="pp-actions">
            <ProfileAction icon="user" label={t("dm.fullProfile")} onClick={onOpenProfile} />
            {relation === "friend" ? (
              <ProfileAction
                icon="userMinus"
                label={t("common.removeFriend")}
                onClick={() => confirmThen(removeFriend(t, name), onRemoveFriend)}
              />
            ) : null}
            {relation === "blocked" ? (
              <ProfileAction icon="ban" label={t("common.unblock")} onClick={onUnblock} />
            ) : (
              <ProfileAction
                icon="ban"
                label={t("common.block")}
                danger
                onClick={() => confirmThen(blockPerson(t, name), onBlock)}
              />
            )}
          </div>
        </div>
      </div>
    </aside>
  );
}
