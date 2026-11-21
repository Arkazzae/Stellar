import { Fragment, useId, useState } from "react";

import { Button } from "@stellarr/ui";
import { Icon, type IconName } from "@stellarr/icons";
import { deleteSection, sectionNotEmpty, sectionSubject } from "@/components/confirmations";
import { useConfirm } from "@/components/primitives/ConfirmDialog";
import { DialogClose } from "@/components/primitives/DialogClose";
import { Modal } from "@/components/primitives/Modal";
import {
  PermissionOverrideEditor,
  type PermissionOverridePatch,
} from "@/components/settings/channel/PermissionOverrideEditor";
import { SettingsSection } from "@/components/settings/SettingsSection";
import type { Translator } from "@/i18n";
import type { ChannelKind, PermissionOverride, Role } from "@/ports";
import { useLanguage } from "@/state/hooks";

type PaneKey = "general" | "permissions" | "danger";

export interface SectionSettingsVM {
  id: string;
  name: string;
  /** What sits under it, in rail order: only an empty section can be deleted. */
  channels: readonly { id: string; name: string; kind: ChannelKind }[];
}

export interface SectionSettingsProps {
  open: boolean;
  onClose: () => void;
  /** Null while the dialog is closing and the rail has moved on. */
  section: SectionSettingsVM | null;
  /** False when this member may not manage channels: the form stays legible and read-only. */
  supported: boolean;
  /** False when this member may not manage roles, so overrides are read-only. */
  permissionsSupported: boolean;
  roles: readonly Role[];
  permissionOverrides: readonly PermissionOverride[];
  onSave: (patch: { name: string }) => Promise<void>;
  onSavePermissionOverride: (roleId: string, patch: PermissionOverridePatch) => Promise<void>;
  /**
   * Deletes the section. "blocked" when the server refused because channels
   * still sit under it, which the dialog then says the same way it says it
   * when it knows so beforehand.
   */
  onDelete: () => Promise<"deleted" | "blocked">;
  onOpenRoles: () => void;
}

interface NavItem {
  key: PaneKey;
  label: string;
  icon: IconName;
  detail: string;
  danger?: boolean;
}

interface NavGroup {
  caption: string;
  items: readonly NavItem[];
}

/** Grouped like the channel and server dialogs, so the three shells match. */
function navigationFor(t: Translator): readonly NavGroup[] {
  return [
    {
      caption: t("section.title"),
      items: [
        {
          key: "general",
          label: t("settings.general"),
          icon: "folder",
          detail: t("section.nameDetail"),
        },
      ],
    },
    {
      caption: t("settings.access"),
      items: [
        {
          key: "permissions",
          label: t("channel.permissions"),
          icon: "shield",
          detail: t("section.permissionsDetail"),
        },
      ],
    },
    {
      caption: t("settings.other"),
      items: [
        {
          key: "danger",
          label: t("section.delete"),
          icon: "trash",
          detail: t("section.deleteDetail"),
          danger: true,
        },
      ],
    },
  ];
}

/**
 * Deleting a section. One that holds channels cannot be deleted, so the card
 * says so before anything is sent; the server's refusal leads to the same card.
 */
function SectionDeleteButton({
  section,
  supported,
  onDelete,
}: {
  section: SectionSettingsVM | null;
  supported: boolean;
  onDelete: () => Promise<"deleted" | "blocked">;
}) {
  const { t } = useLanguage();
  const confirm = useConfirm();

  const askToDelete = () => {
    if (section === null) return;
    const subject = sectionSubject(t, section);
    if (section.channels.length > 0) {
      void confirm(sectionNotEmpty(t, subject));
      return;
    }
    void confirm({
      ...deleteSection(t, subject),
      run: async () => {
        if ((await onDelete()) !== "blocked") return;
        // The directory had not heard of the channels, so the card names none.
        void confirm(sectionNotEmpty(t, sectionSubject(t, { name: section.name, channels: null })));
      },
    });
  };

  return (
    <Button type="button" variant="danger" size="sm" disabled={!supported} onClick={askToDelete}>
      <Icon name="trash" size={15} />
      {t("section.delete")}
    </Button>
  );
}

export function SectionSettings({
  open,
  onClose,
  section,
  supported,
  permissionsSupported,
  roles,
  permissionOverrides,
  onSave,
  onSavePermissionOverride,
  onDelete,
  onOpenRoles,
}: SectionSettingsProps) {
  const { t } = useLanguage();
  const titleId = useId();
  const nameFieldId = useId();
  const [name, setName] = useState(section?.name ?? "");
  const [active, setActive] = useState<PaneKey>("general");
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  // Reset while rendering, not in an effect, so reopening the dialog (on the
  // same section or another) never lands on whichever pane was left active.
  const [previous, setPrevious] = useState({ section, open });
  if (previous.section !== section || previous.open !== open) {
    setPrevious({ section, open });
    setName(section?.name ?? "");
    setActive("general");
    setBusy(false);
    setActionError(null);
  }

  const navigation = navigationFor(t);
  const current = navigation.flatMap((group) => group.items).find((item) => item.key === active);
  const trimmed = name.trim();
  const canSave =
    supported && !busy && section !== null && trimmed.length > 0 && trimmed !== section.name;

  const save = async () => {
    if (!canSave) return;
    setBusy(true);
    setActionError(null);
    try {
      await onSave({ name: trimmed });
    } catch {
      setActionError(t("section.saveFailed"));
    } finally {
      setBusy(false);
    }
  };


  const renderPane = () => {
    switch (active) {
      case "general":
        return (
          <SettingsSection title={t("section.name")} bodyClassName="ch-settings-form">
            <div className="frm">
              <div className="frm-row">
                <label className="frm-lab" htmlFor={nameFieldId}>
                  {t("section.name")}
                  <span>{t("section.nameDetail")}</span>
                </label>
                <div className="frm-ctl">
                  <input
                    id={nameFieldId}
                    className="set-input"
                    type="text"
                    value={name}
                    maxLength={60}
                    disabled={!supported}
                    onChange={(event) => setName(event.target.value)}
                  />
                </div>
              </div>
            </div>
          </SettingsSection>
        );
      case "permissions":
        return (
          <SettingsSection title={t("channel.channelPermissions")}>
            <p className="fld-note">{t("section.permissionsDetailText")}</p>
            {roles.length === 0 ? (
              <p className="fld-note">{t("channel.noRoles")}</p>
            ) : (
              <PermissionOverrideEditor
                supported={permissionsSupported}
                roles={roles}
                overrides={permissionOverrides}
                onSave={onSavePermissionOverride}
              />
            )}
            <Button type="button" variant="secondary" size="sm" onClick={onOpenRoles}>
              <Icon name="shield" size={15} />
              {t("channel.openServerRoles")}
            </Button>
          </SettingsSection>
        );
      case "danger":
        return (
          <SettingsSection title={t("section.delete")} className="danger-section">
            <p className="fld-note">{t("section.deleteWarning")}</p>
            <SectionDeleteButton section={section} supported={supported} onDelete={onDelete} />
          </SettingsSection>
        );
    }
  };

  return (
    <Modal open={open} onClose={onClose} labelledBy={titleId} className="sv-wrap">
      <div className="sv ch-settings">
        <div className="sv-bar">
          <div className="sv-bar-txt">
            <h2>{current?.label}</h2>
            <p>{current?.detail}</p>
          </div>
          <DialogClose label={t("common.close")} onClose={onClose} />
        </div>

        <aside className="sv-nav">
          <div className="sv-brand ch-settings-brand">
            <span className="ch-settings-mark" aria-hidden="true">
              <Icon name="folder" size={20} />
            </span>
            <span className="sv-brand-txt">
              <b
                id={titleId}
                aria-label={
                  section === null ? undefined : t("settings.editSection", { name: section.name })
                }
              >
                {section?.name ?? t("section.title")}
              </b>
              <span>{t("section.title")}</span>
            </span>
          </div>
          <nav className="sv-nav-scroll" aria-label={t("section.settingsSections")}>
            {navigation.map((group) => (
              <Fragment key={group.caption}>
                <div className="sv-cap">{group.caption}</div>
                {group.items.map((item) => (
                  <button
                    key={item.key}
                    type="button"
                    className={`sv-item${item.key === active ? " on" : ""}${item.danger ? " danger" : ""}`}
                    aria-current={item.key === active ? "page" : undefined}
                    onClick={() => setActive(item.key)}
                  >
                    <span className="sv-ico" aria-hidden="true">
                      <Icon name={item.icon} size={18} />
                    </span>
                    {item.label}
                  </button>
                ))}
              </Fragment>
            ))}
          </nav>
        </aside>

        <div className="sv-main">
          <div className="sv-panes">
            <section className="sv-pane on" data-pane={active} key={active}>
              {supported ? null : <p className="set-note">{t("channel.settingsUnsupported")}</p>}
              {renderPane()}
            </section>
          </div>
          <footer className="ch-settings-foot">
            {actionError === null ? null : (
              <span className="frm-detail-error" role="alert">
                {actionError}
              </span>
            )}
            <Button type="button" variant="secondary" onClick={onClose}>
              <Icon name="x" size={15} />
              {t("common.close")}
            </Button>
            <Button
              type="button"
              variant="primary"
              disabled={!canSave}
              onClick={() => void save()}
            >
              <Icon name="tick" size={15} />
              {busy ? t("common.saving") : t("channel.settingsSave")}
            </Button>
          </footer>
        </div>
      </div>
    </Modal>
  );
}
