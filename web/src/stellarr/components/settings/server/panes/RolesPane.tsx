import { useEffect, useId, useRef, useState, type CSSProperties } from "react";

import { Button } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { Avatar } from "@/components/primitives/Avatar";
import { useConfirm } from "@/components/primitives/ConfirmDialog";
import { Switch } from "@/components/primitives/Switch";
import { countedNoun } from "@/components/settings/paneFormat";
import { SettingsSaveBar } from "@/components/settings/SettingsSaveBar";
import { SettingsSection } from "@/components/settings/SettingsSection";
import { RoleMark } from "@/components/settings/server/panes/RoleMark";
import { useSettingsGuard } from "@/components/settings/settingsGuard";
import { ACCENT_SWATCHES } from "@/design/accentSwatches";
import { useAsyncAction } from "@/hooks/useAsyncAction";
import type { MessageKey } from "@/i18n";
import { SERVER_PERMISSION_VALUES } from "@/ports";
import type { AdminMember, Role, RoleInput, ServerPermission } from "@/ports";
import { useLanguage } from "@/state/hooks";

const NEW_ROLE_COLOR = "#8b7cf6";
const SAVED_FLASH_MS = 1500;

export const PERMISSION_LABEL: Record<ServerPermission, MessageKey> = {
  manageServer: "settings.permission.manageServer",
  createEvents: "settings.permission.createEvents",
  manageEvents: "settings.permission.manageEvents",
  manageChannels: "settings.permission.manageChannels",
  manageRoles: "settings.permission.manageRoles",
  manageMembers: "settings.permission.manageMembers",
  manageMessages: "settings.permission.manageMessages",
  moderateMembers: "settings.permission.moderateMembers",
  manageBans: "settings.permission.manageBans",
  viewAuditLog: "settings.permission.viewAuditLog",
  administrator: "settings.permission.administrator",
  sendMessages: "settings.permission.sendMessages",
  attachFiles: "settings.permission.attachFiles",
  addReactions: "settings.permission.addReactions",
  connect: "settings.permission.connect",
  createInvite: "settings.permission.createInvite",
  viewChannel: "settings.permission.viewChannel",
};

const PERMISSION_DETAIL: Record<ServerPermission, MessageKey> = {
  manageServer: "srv.perm.manageServer",
  createEvents: "srv.perm.createEvents",
  manageEvents: "srv.perm.manageEvents",
  manageChannels: "srv.perm.manageChannels",
  manageRoles: "srv.perm.manageRoles",
  manageMembers: "srv.perm.manageMembers",
  manageMessages: "srv.perm.manageMessages",
  moderateMembers: "srv.perm.moderateMembers",
  manageBans: "srv.perm.manageBans",
  viewAuditLog: "srv.perm.viewAuditLog",
  administrator: "srv.perm.administrator",
  sendMessages: "srv.perm.sendMessages",
  attachFiles: "srv.perm.attachFiles",
  addReactions: "srv.perm.addReactions",
  connect: "srv.perm.connect",
  createInvite: "srv.perm.createInvite",
  viewChannel: "srv.perm.viewChannel",
};

/** The permissions in the groups a reader looks for them in. */
const PERMISSION_GROUPS: readonly { caption: MessageKey; permissions: readonly ServerPermission[] }[] = [
  { caption: "srv.roles.groupServer", permissions: ["manageServer", "manageChannels", "viewChannel"] },
  { caption: "srv.roles.groupPeople", permissions: ["manageRoles", "manageMembers", "createInvite"] },
  { caption: "srv.roles.groupModeration", permissions: ["manageMessages", "moderateMembers", "manageBans", "viewAuditLog"] },
  { caption: "srv.roles.groupText", permissions: ["sendMessages", "attachFiles", "addReactions"] },
  { caption: "srv.roles.groupVoice", permissions: ["connect"] },
  { caption: "srv.roles.groupEvents", permissions: ["createEvents", "manageEvents"] },
  { caption: "srv.roles.groupAdvanced", permissions: ["administrator"] },
];

export interface RolesPaneProps {
  supported: boolean;
  roles: readonly Role[];
  /** Who holds which role, for the people a role is given to. */
  members?: readonly AdminMember[];
  onCreate: (input: RoleInput) => Promise<Role | void>;
  onUpdate: (roleId: string, patch: RoleInput) => Promise<void>;
  onDelete: (roleId: string) => Promise<void>;
}

/**
 * The roles as Discord lists them: the default role on its own, then a table
 * of the others with a shield in each one's colour and how many hold it. A
 * role opens in place of the list, as a draft saved through the save bar.
 */
export function RolesPane({ supported, roles, members = [], onCreate, onUpdate, onDelete }: RolesPaneProps) {
  const { t } = useLanguage();
  const [editing, setEditing] = useState<string | null>(null);
  const create = useAsyncAction();
  // A role that is not in the list (made a moment ago and not fetched yet, or
  // deleted elsewhere) shows the list until it is.
  const role = roles.find((item) => item.id === editing);

  if (role !== undefined) {
    return (
      <RoleEditor
        key={role.id}
        role={role}
        supported={supported}
        holders={members.filter((member) => member.roleId === role.id)}
        onUpdate={onUpdate}
        onDelete={onDelete}
        onBack={() => setEditing(null)}
      />
    );
  }

  const fallback = roles.find((item) => item.isSystem && !item.isOwner);
  const others = roles.filter((item) => item !== fallback);

  const createRole = async () => {
    let made: Role | void = undefined;
    const done = await create.run(async () => {
      made = await onCreate({ name: t("settings.newRole"), color: NEW_ROLE_COLOR, permissions: [] });
    });
    const id = (made as Role | undefined)?.id;
    if (done && id !== undefined) setEditing(id);
  };

  return (
    <>
      {supported ? null : <p className="set-note">{t("settings.rolesUnsupported")}</p>}
      {create.failed ? (
        <p className="set-note sound-error" role="alert">
          {t("settings.roleActionFailed")}
        </p>
      ) : null}
      {fallback === undefined ? null : (
        <SettingsSection title={t("srv.roles.default")} bodyClassName="sec-list ss-list">
          <RoleRow role={fallback} detail={t("srv.roles.defaultDetail")} onOpen={() => setEditing(fallback.id)} />
        </SettingsSection>
      )}
      <SettingsSection
        title={t("server.roles")}
        count={others.length}
        countLabel={t("settings.rolesCount", { count: others.length })}
        bodyClassName="sec-list ss-list"
      >
        <div className="ss-roles-head" aria-hidden="true">
          <span>{t("srv.roles.colRole")}</span>
          <span className="ss-col-count">{t("srv.roles.colMembers")}</span>
          <span className="ss-col-edit" />
        </div>
        {others.map((item) => (
          <RoleRow key={item.id} role={item} onOpen={() => setEditing(item.id)} />
        ))}
        <div className="sec-row ss-foot">
          <span className="ss-foot-note">{t("srv.roles.note")}</span>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            disabled={!supported || create.pending}
            onClick={() => void createRole()}
          >
            <Icon name="plus" size={15} />
            {t("settings.newRole")}
          </Button>
        </div>
      </SettingsSection>
    </>
  );
}

function RoleRow({ role, detail, onOpen }: { role: Role; detail?: string; onOpen: () => void }) {
  const { t } = useLanguage();
  return (
    <button
      type="button"
      className={detail === undefined ? "sec-row ss-role-row" : "sec-row ss-role-row ss-role-default"}
      aria-label={t("srv.roles.edit", { name: role.name })}
      onClick={onOpen}
    >
      {detail === undefined ? (
        <>
          <RoleMark role={role} size={20} />
          <span className="ss-role-name">{role.name}</span>
        </>
      ) : (
        <>
          <span className="ss-role-ico" aria-hidden="true">
            <Icon name="users" size={20} />
          </span>
          <span className="sec-copy">
            <b>{role.name}</b>
            <small>{detail}</small>
          </span>
        </>
      )}
      <span className="ss-role-count">
        <Icon name="users" size={16} />
        <b>{role.memberCount}</b>
      </span>
      <span className="ss-role-edit" aria-hidden="true">
        <Icon name={detail === undefined ? "edit" : "caretR"} size={16} />
      </span>
    </button>
  );
}

const sameDraft = (a: RoleInput, b: RoleInput) =>
  a.name === b.name &&
  a.color === b.color &&
  a.permissions.length === b.permissions.length &&
  a.permissions.every((permission) => b.permissions.includes(permission));

interface RoleEditorProps {
  role: Role;
  supported: boolean;
  holders: readonly AdminMember[];
  onUpdate: (roleId: string, patch: RoleInput) => Promise<void>;
  onDelete: (roleId: string) => Promise<void>;
  onBack: () => void;
}

/**
 * One role, in place of the list. Name, colour and permissions are a draft:
 * the server wants a fresh second step for a role change, so they go in one
 * save rather than one per switch.
 */
function RoleEditor({ role, supported, holders, onUpdate, onDelete, onBack }: RoleEditorProps) {
  const { locale, t } = useLanguage();
  const confirm = useConfirm();
  const guard = useSettingsGuard();
  // A draft holds the editor: going back waits until it is saved or reset.
  const leave = () => (guard === null ? onBack() : guard.attempt(onBack));
  const save = useAsyncAction();
  const removal = useAsyncAction();
  const [saved, setSaved] = useState(false);
  const savedTimer = useRef<number | undefined>(undefined);
  const backRef = useRef<HTMLButtonElement>(null);
  const nameId = useId();
  const locked = !supported || role.isOwner;

  const stored: RoleInput = { name: role.name, color: role.color, permissions: [...role.permissions] };
  const [draft, setDraft] = useState(stored);
  const [seed, setSeed] = useState(stored);
  if (!sameDraft(seed, stored)) {
    setSeed(stored);
    setDraft(stored);
  }

  useEffect(() => backRef.current?.focus(), []);
  useEffect(() => () => window.clearTimeout(savedTimer.current), []);

  const dirty = !sameDraft(
    { ...draft, name: draft.name.trim() },
    { ...stored, name: stored.name.trim() },
  );

  const commit = async () => {
    const done = await save.run(() => onUpdate(role.id, { ...draft, name: draft.name.trim() }));
    if (!done) return;
    setSaved(true);
    window.clearTimeout(savedTimer.current);
    savedTimer.current = window.setTimeout(() => setSaved(false), SAVED_FLASH_MS);
  };

  const togglePermission = (permission: ServerPermission, on: boolean) =>
    setDraft({
      ...draft,
      permissions: on
        ? [...draft.permissions, permission]
        : draft.permissions.filter((item) => item !== permission),
    });

  const remove = async () => {
    const yes = await confirm({
      title: t("srv.roles.deleteTitle"),
      description:
        holders.length === 0
          ? t("srv.roles.deleteNobody")
          : t("srv.roles.deleteFallback", { people: countedNoun(t, locale, "settings.member", holders.length) }),
      subject: { kind: "role", name: role.name, color: role.color, isOwner: role.isOwner },
      confirmLabel: t("settings.deleteRole"),
      danger: true,
    });
    if (yes && (await removal.run(() => onDelete(role.id)))) onBack();
  };

  const swatches = ACCENT_SWATCHES.map((swatch) => ({ hex: swatch.hex, label: t(`theme.accent.${swatch.id}`) }));
  const known = swatches.some((swatch) => swatch.hex.toLowerCase() === draft.color.toLowerCase());
  const shown = holders.slice(0, 5);
  const named = holders.slice(0, 3).map((member) => member.user.displayName).join(", ");

  return (
    <>
      <button type="button" className="ss-back" ref={backRef} onClick={leave}>
        <Icon name="arrowLeft" size={16} />
        {t("srv.roles.back")}
      </button>
      {role.isOwner ? <p className="set-note ss-lock">{t("srv.roles.ownerLock")}</p> : null}
      {role.isSystem && !role.isOwner ? <p className="set-note ss-lock">{t("srv.roles.defaultLock")}</p> : null}
      {!supported ? <p className="set-note ss-lock">{t("settings.rolesUnsupported")}</p> : null}

      <section className="sc settings-section" aria-label={draft.name || role.name}>
        <header className="sc-h">
          <RoleMark role={{ color: draft.color, isOwner: role.isOwner }} size={18} />
          <span>{draft.name.trim() || role.name}</span>
        </header>
        <div className="sc-b">
          <div className="frm">
            <div className="frm-row">
              <label className="frm-lab" htmlFor={nameId}>
                {t("settings.roleName")}
                <span>{t("srv.roles.nameDetail")}</span>
              </label>
              <div className="frm-ctl">
                <input
                  id={nameId}
                  className="set-input"
                  type="text"
                  maxLength={32}
                  value={draft.name}
                  disabled={locked}
                  onChange={(event) => setDraft({ ...draft, name: event.target.value })}
                />
              </div>
            </div>
            <div className="frm-row ap-row">
              <span className="frm-lab">
                {t("srv.roles.color")}
                <span>{t("srv.roles.colorDetail")}</span>
              </span>
              <div className="frm-ctl ap-end ap-accents">
                <div className="ap-swatches" role="radiogroup" aria-label={t("srv.roles.color")}>
                  {swatches.map((swatch) => {
                    const on = swatch.hex.toLowerCase() === draft.color.toLowerCase();
                    return (
                      <button
                        key={swatch.hex}
                        type="button"
                        role="radio"
                        aria-checked={on}
                        aria-label={swatch.label}
                        title={swatch.label}
                        className={on ? "ap-acc on" : "ap-acc"}
                        style={{ "--sw": swatch.hex } as CSSProperties}
                        disabled={locked}
                        onClick={() => setDraft({ ...draft, color: swatch.hex })}
                      />
                    );
                  })}
                  {known ? null : (
                    <button
                      type="button"
                      role="radio"
                      aria-checked
                      aria-label={draft.color}
                      title={draft.color}
                      className="ap-acc on"
                      style={{ "--sw": draft.color } as CSSProperties}
                      disabled={locked}
                    />
                  )}
                </div>
                <label className="ap-acc custom" title={t("srv.roles.customColor")} style={{ "--sw": draft.color } as CSSProperties}>
                  <input
                    className="sr-only"
                    type="color"
                    aria-label={t("srv.roles.customColor")}
                    value={/^#[0-9a-f]{6}$/i.test(draft.color) ? draft.color : "#8b7cf6"}
                    disabled={locked}
                    onChange={(event) => setDraft({ ...draft, color: event.target.value })}
                  />
                </label>
              </div>
            </div>
            <div className="frm-row">
              <span className="frm-lab">
                {t("srv.roles.people")}
                <span>
                  {holders.length === 0
                    ? t("srv.roles.nobody")
                    : t("srv.roles.holders", { people: countedNoun(t, locale, "settings.member", holders.length) })}
                </span>
              </span>
              <div className="frm-ctl">
                <div className="ss-people">
                  {holders.length === 0 ? (
                    <span className="ss-people-txt">{t("srv.roles.giveInMembers")}</span>
                  ) : (
                    <>
                      <span className="ss-stack">
                        {shown.map((member) => (
                          <Avatar
                            key={member.user.id}
                            id={member.user.id}
                            displayName={member.user.displayName}
                            avatarUrl={member.user.avatarUrl}
                            size="sm"
                          />
                        ))}
                      </span>
                      <span className="ss-people-txt">
                        {holders.length > 3
                          ? t("srv.roles.andMore", { names: named, count: holders.length - 3 })
                          : named}
                      </span>
                    </>
                  )}
                </div>
              </div>
            </div>
          </div>
        </div>
      </section>

      <SettingsSection title={t("srv.roles.permissions")}>
        <div className="frm">
          {PERMISSION_GROUPS.map((group) => (
            <div key={group.caption} role="group" aria-label={t(group.caption)}>
              <div className="ss-subcap">{t(group.caption)}</div>
              {group.permissions
                .filter((permission) => SERVER_PERMISSION_VALUES.includes(permission))
                .map((permission) => (
                  <div className="frm-row" key={permission}>
                    <span className="frm-lab">
                      {t(PERMISSION_LABEL[permission])}
                      <span>{t(PERMISSION_DETAIL[permission])}</span>
                    </span>
                    <div className="frm-ctl frm-switch">
                      <Switch
                        checked={role.isOwner || draft.permissions.includes(permission)}
                        onChange={(on) => togglePermission(permission, on)}
                        label={t(PERMISSION_LABEL[permission])}
                        disabled={locked}
                      />
                    </div>
                  </div>
                ))}
            </div>
          ))}
        </div>
      </SettingsSection>

      {save.failed ? (
        <p className="set-note sound-error" role="alert">
          {t("settings.roleActionFailed")}
        </p>
      ) : null}

      {role.isSystem ? null : (
        <SettingsSection title={t("srv.roles.deleteSection")} bodyClassName="sec-list">
          <div className="sec-row">
            <span className="sec-copy">
              <b>{t("srv.roles.deleteRow")}</b>
              <small>
                {holders.length === 0
                  ? t("srv.roles.deleteNobody")
                  : t("srv.roles.deleteFallback", { people: countedNoun(t, locale, "settings.member", holders.length) })}
              </small>
            </span>
            <Button
              type="button"
              variant="danger"
              size="sm"
              disabled={!supported || removal.pending}
              onClick={() => void remove()}
            >
              {t("settings.deleteRole")}
            </Button>
          </div>
          {removal.failed ? (
            <p className="set-note sound-error" role="alert">
              {t("settings.roleActionFailed")}
            </p>
          ) : null}
        </SettingsSection>
      )}

      <SettingsSaveBar
        dirty={dirty && !locked}
        saving={save.pending}
        saved={saved}
        saveDisabled={draft.name.trim() === ""}
        onSave={() => void commit()}
        onReset={() => setDraft(stored)}
      />
    </>
  );
}
