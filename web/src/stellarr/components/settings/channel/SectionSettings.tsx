import { useEffect, useId, useRef, useState } from "react";

import { Button } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { deleteSection, sectionNotEmpty, sectionSubject } from "@/components/confirmations";
import { useConfirm } from "@/components/primitives/ConfirmDialog";
import { Modal } from "@/components/primitives/Modal";
import {
  groupCaption,
  PreviewCaption,
  PreviewRow,
} from "@/components/settings/channel/ChannelFormPanes";
import { railNeighbours, type RailGroup } from "@/components/settings/channel/channelSettingsModel";
import {
  PermissionOverrideEditor,
  type PermissionOverridePatch,
} from "@/components/settings/channel/PermissionOverrideEditor";
import { countedNoun } from "@/components/settings/paneFormat";
import { SettingsSaveBar } from "@/components/settings/SettingsSaveBar";
import { SettingsSection } from "@/components/settings/SettingsSection";
import {
  SettingsFooterAction,
  SettingsShell,
  type SettingsShellGroup,
} from "@/components/settings/SettingsShell";
import { SettingsGuardProvider, useSettingsGuardHost } from "@/components/settings/settingsGuard";
import type { Translator } from "@/i18n";
import type { ChannelKind, PermissionOverride, Role } from "@/ports";
import { useLanguage } from "@/state/hooks";

const SAVED_FLASH_MS = 1500;

type PaneKey = "overview" | "permissions";

export interface SectionSettingsVM {
  id: string;
  name: string;
  /** What sits under it, in rail order: only an empty section can be deleted. */
  channels: readonly { id: string; name: string; kind: ChannelKind }[];
  /** The rail's groups, for the preview that shows the section among its neighbours. */
  rail?: readonly RailGroup[];
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

interface NavDetail {
  label: string;
  detail: string;
}

/** Grouped and worded like the channel's settings, so the shells match. */
function navigationFor(t: Translator): {
  groups: readonly SettingsShellGroup<PaneKey>[];
  details: Record<PaneKey, NavDetail>;
} {
  const details: Record<PaneKey, NavDetail> = {
    overview: { label: t("server.overview"), detail: t("section.overviewDetail") },
    permissions: { label: t("channel.permissions"), detail: t("section.permissionsDetail") },
  };
  return {
    details,
    groups: [
      {
        caption: t("server.general"),
        items: [{ key: "overview", label: details.overview.label, icon: "list" }],
      },
      {
        caption: t("server.management"),
        items: [{ key: "permissions", label: details.permissions.label, icon: "shield" }],
      },
    ],
  };
}

/**
 * Deleting a section, alone at the foot of the navigation. One that holds
 * channels cannot be deleted, so the card says so before anything is sent;
 * the server's refusal leads to the same card.
 */
function SectionDeleteAction({
  section,
  supported,
  onDelete,
}: {
  section: SectionSettingsVM;
  supported: boolean;
  onDelete: () => Promise<"deleted" | "blocked">;
}) {
  const { t } = useLanguage();
  const confirm = useConfirm();

  const askToDelete = () => {
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
    <SettingsFooterAction icon="trash" label={t("section.delete")} disabled={!supported} onClick={askToDelete} />
  );
}

/**
 * The section where it lives, while it is being renamed: its caption, which
 * follows the name being typed, with its channels under it and the groups
 * around it dimmed; and beside it what it holds and how that changes.
 */
function SectionPreview({ section, name }: { section: SectionSettingsVM; name: string }) {
  const { locale, t } = useLanguage();
  const rail = section.rail ?? [];
  const index = rail.findIndex((group) => group.section?.id === section.id);
  const { before, after } = railNeighbours(rail, index);
  const count = section.channels.length;

  const neighbour = (group: RailGroup | undefined, edge: "first" | "last") => {
    const channel = edge === "first" ? group?.channels[0] : group?.channels[group.channels.length - 1];
    if (group === undefined || channel === undefined) return null;
    return (
      <>
        <PreviewCaption text={groupCaption(t, group)} state="dim" />
        <PreviewRow kind={channel.kind} name={channel.name} state="dim" />
      </>
    );
  };

  return (
    // A picture of the form below, which assistive tech reads instead.
    <div className="pv" aria-hidden="true">
      <div className="pv-rail wide">
        {neighbour(before, "last")}
        <PreviewCaption text={name.trim() || section.name} state="on" />
        {count === 0 ? (
          <div className="ch-place-row empty">{t("section.emptyYet")}</div>
        ) : (
          section.channels.map((channel) => <PreviewRow key={channel.id} kind={channel.kind} name={channel.name} />)
        )}
        {neighbour(after, "first")}
      </div>
      <div className="pv-side">
        <b>{countedNoun(t, locale, "section.channelCount", count)}</b>
        <span>{t("section.previewDetail")}</span>
      </div>
    </div>
  );
}

/** The cog on a rail section: its name and place, and who may see its channels. */
export function SectionSettings({ open, onClose, ...content }: SectionSettingsProps) {
  const titleId = useId();
  // Escape and a press outside close through the same guard as the cross, so
  // an unsaved name is never thrown away by any of them.
  const guard = useSettingsGuardHost();

  // The modal mounts its children only while it is open, so the layout below
  // starts on the overview every time the dialog is reopened.
  return (
    <Modal open={open} onClose={() => guard.attempt(onClose)} labelledBy={titleId} className="sv-wrap">
      <SettingsGuardProvider guard={guard}>
        <SectionLayout titleId={titleId} onClose={onClose} {...content} />
      </SettingsGuardProvider>
    </Modal>
  );
}

type LayoutProps = Omit<SectionSettingsProps, "open"> & { titleId: string };

function SectionLayout({
  titleId,
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
}: LayoutProps) {
  const { t } = useLanguage();
  const [active, setActive] = useState<PaneKey>("overview");
  const { groups, details } = navigationFor(t);
  const current = details[active];

  return (
    <SettingsShell
      titleId={titleId}
      brand={{
        name: section?.name ?? t("section.title"),
        detail: t("section.settings"),
        ...(section === null ? {} : { label: t("settings.editSection", { name: section.name }) }),
      }}
      title={current.label}
      detail={current.detail}
      closeLabel={t("section.close")}
      onClose={onClose}
      navLabel={t("section.settingsSections")}
      groups={groups}
      active={active}
      onSelect={setActive}
      className="ch-settings"
      footer={
        section === null ? undefined : (
          <SectionDeleteAction section={section} supported={supported} onDelete={onDelete} />
        )
      }
    >
      {supported ? null : <p className="set-note">{t("channel.settingsUnsupported")}</p>}
      {active === "overview" ? (
        section === null ? null : (
          <SectionOverviewPane key={section.id} section={section} supported={supported} onSave={onSave} />
        )
      ) : (
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
      )}
    </SettingsShell>
  );
}

/** The name, as a draft with the save bar, under the preview of where it stands. */
function SectionOverviewPane({
  section,
  supported,
  onSave,
}: {
  section: SectionSettingsVM;
  supported: boolean;
  onSave: (patch: { name: string }) => Promise<void>;
}) {
  const { t } = useLanguage();
  const nameFieldId = useId();
  const [name, setName] = useState(section.name);
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState(false);
  const [saved, setSaved] = useState(false);
  const savedTimer = useRef<number | undefined>(undefined);

  // Re-seeded during render when the saved name changes underneath, as it does
  // once the rail has the new one, so the draft is never stale.
  const [seed, setSeed] = useState(section.name);
  if (seed !== section.name) {
    setSeed(section.name);
    setName(section.name);
  }

  useEffect(() => () => window.clearTimeout(savedTimer.current), []);

  const trimmed = name.trim();
  const dirty = trimmed !== section.name;

  const save = async () => {
    if (saving || trimmed.length === 0 || !dirty) return;
    setSaving(true);
    setFailed(false);
    try {
      await onSave({ name: trimmed });
      setSaved(true);
      window.clearTimeout(savedTimer.current);
      savedTimer.current = window.setTimeout(() => setSaved(false), SAVED_FLASH_MS);
    } catch {
      setFailed(true);
    } finally {
      setSaving(false);
    }
  };

  return (
    <>
      <SectionPreview section={section} name={name} />
      <SettingsSection title={t("section.title")} bodyClassName="ch-settings-form">
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
        {failed ? (
          <p className="set-note sound-error ov-save-error" role="alert">
            {t("section.saveFailed")}
          </p>
        ) : null}
      </SettingsSection>
      <SettingsSaveBar
        dirty={dirty}
        saving={saving}
        saved={saved}
        saveDisabled={!supported || trimmed.length === 0}
        onSave={() => void save()}
        onReset={() => setName(section.name)}
      />
    </>
  );
}
