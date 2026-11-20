import { useState, type CSSProperties } from "react";

import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { Avatar } from "@/components/primitives/Avatar";
import { useConfirm, usePrompt } from "@/components/primitives/ConfirmDialog";
import { banPerson, kickPerson } from "@/components/confirmations";
import { countedNoun, formatShortDate } from "@/components/settings/paneFormat";
import { SettingsSection } from "@/components/settings/SettingsSection";
import { RoleMark } from "@/components/settings/server/panes/RolesPane";
import { useAsyncAction } from "@/hooks/useAsyncAction";
import type { AdminMember, Role } from "@/ports";
import { useLanguage } from "@/state/hooks";

export interface MembersPaneProps {
  /** Whether this member may give others a role. */
  supported: boolean;
  members: readonly AdminMember[];
  /** Looked up by `member.roleId`; the pill's shield takes the role's colour. */
  roles: readonly Role[];
  onAssignRole: (userId: string, roleId: string) => Promise<void>;
  /** Who is reading, whose own row has no menu. */
  viewerId?: string;
  /** Given when the reader may kick: the same action as the member menu's. */
  onKick?: (userId: string) => Promise<void>;
  /** Given when the reader may ban; the reason is optional. */
  onBan?: (userId: string, reason?: string) => Promise<void>;
}

/**
 * Everyone on the server with their role. The role is a pill that opens the
 * list of roles; kicking and banning sit in the menu beside it, with the
 * questions the member menu asks.
 */
export function MembersPane({
  supported,
  members,
  roles,
  onAssignRole,
  viewerId,
  onKick,
  onBan,
}: MembersPaneProps) {
  const { locale, t } = useLanguage();
  const confirm = useConfirm();
  const prompt = usePrompt();
  const assign = useAsyncAction();
  const moderate = useAsyncAction();
  const [query, setQuery] = useState("");
  const roleById = new Map(roles.map((role) => [role.id, role]));
  const needle = query.trim().toLocaleLowerCase();
  const shown = needle === ""
    ? members
    : members.filter((member) => member.user.displayName.toLocaleLowerCase().includes(needle));

  const kick = async (member: AdminMember) => {
    if (onKick === undefined) return;
    if (await confirm(kickPerson(t, member.user.displayName))) await moderate.run(() => onKick(member.user.id));
  };
  const ban = async (member: AdminMember) => {
    if (onBan === undefined) return;
    const reason = await prompt(banPerson(t, member.user.displayName));
    if (reason !== null) await moderate.run(() => onBan(member.user.id, reason === "" ? undefined : reason));
  };

  return (
    <>
      {supported ? null : <p className="set-note">{t("settings.membersUnsupported")}</p>}
      <SettingsSection
        title={t("srv.nav.members")}
        count={members.length}
        countLabel={countedNoun(t, locale, "settings.member", members.length)}
        bodyClassName="sec-list ss-list"
      >
        {assign.failed || moderate.failed ? (
          <p className="set-note sound-error" role="alert">
            {t("settings.memberActionFailed")}
          </p>
        ) : null}
        {members.length === 0 ? (
          <p className="acc-security-item acc-security-empty">{t("settings.noMembers")}</p>
        ) : (
          <>
            <div className="ss-search">
              <span aria-hidden="true">
                <Icon name="search" size={16} />
              </span>
              <input
                className="set-input"
                type="search"
                value={query}
                placeholder={t("srv.members.search")}
                aria-label={t("srv.members.search")}
                autoComplete="off"
                onChange={(event) => setQuery(event.target.value)}
              />
            </div>
            {shown.map((member) => {
              const role = roleById.get(member.roleId);
              const self = member.user.id === viewerId;
              // The owner's role is fixed: handing the server over is a transfer.
              const fixed = role?.isOwner === true;
              const canMenu = !self && !fixed && (onKick !== undefined || onBan !== undefined);
              return (
                <div className="sec-row ss-member" key={member.user.id}>
                  <Avatar
                    id={member.user.id}
                    displayName={member.user.displayName}
                    avatarUrl={member.user.avatarUrl}
                    size="sm"
                  />
                  <span className="sec-copy">
                    <b>{member.user.displayName}</b>
                    <small>{t("srv.members.joined", { date: formatShortDate(member.joinedAt, locale) })}</small>
                  </span>
                  {self ? <span className="ss-tag">{t("srv.members.you")}</span> : null}
                  {role === undefined ? null : fixed || !supported || self ? (
                    <span className="ss-role-pill">
                      <RoleMark role={role} size={14} />
                      <span className="ss-pill-label">{role.name}</span>
                    </span>
                  ) : (
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <button
                          type="button"
                          className="ss-role-pill"
                          disabled={assign.pending}
                          aria-label={t("srv.members.role", { name: member.user.displayName, role: role.name })}
                        >
                          <RoleMark role={role} size={14} />
                          <span className="ss-pill-label">{role.name}</span>
                          <span className="ss-pill-caret" aria-hidden="true">
                            <Icon name="caret" size={12} />
                          </span>
                        </button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        {roles
                          .filter((item) => !item.isOwner)
                          .map((item) => (
                            <DropdownMenuItem
                              key={item.id}
                              onSelect={() => {
                                if (item.id !== member.roleId) {
                                  void assign.run(() => onAssignRole(member.user.id, item.id));
                                }
                              }}
                            >
                              <span className="ss-menu-tick" aria-hidden="true">
                                {item.id === member.roleId ? <Icon name="tick" size={14} /> : null}
                              </span>
                              <span className="ss-role-ico" style={{ "--rc": item.color } as CSSProperties} aria-hidden="true">
                                <Icon name="shield" size={15} />
                              </span>
                              {item.name}
                            </DropdownMenuItem>
                          ))}
                      </DropdownMenuContent>
                    </DropdownMenu>
                  )}
                  {canMenu ? (
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          type="button"
                          variant="quiet"
                          size="icon-sm"
                          disabled={moderate.pending}
                          aria-label={t("srv.members.more", { name: member.user.displayName })}
                        >
                          <Icon name="dots" size={16} />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        {onKick === undefined ? null : (
                          <DropdownMenuItem variant="destructive" onSelect={() => void kick(member)}>
                            <Icon name="signout" size={16} />
                            {t("common.kickFromServer")}
                          </DropdownMenuItem>
                        )}
                        {onBan === undefined ? null : (
                          <DropdownMenuItem variant="destructive" onSelect={() => void ban(member)}>
                            <Icon name="ban" size={16} />
                            {t("common.ban")}
                          </DropdownMenuItem>
                        )}
                      </DropdownMenuContent>
                    </DropdownMenu>
                  ) : (
                    <span className="ss-more-gap" aria-hidden="true" />
                  )}
                </div>
              );
            })}
            {shown.length === 0 ? <p className="ss-none">{t("srv.members.none")}</p> : null}
          </>
        )}
      </SettingsSection>
    </>
  );
}
