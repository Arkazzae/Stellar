import { Button } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { channelSubject, deleteChannel } from "@/components/confirmations";
import { useConfirm } from "@/components/primitives/ConfirmDialog";
import { Switch } from "@/components/primitives/Switch";
import {
  PermissionOverrideEditor,
  type PermissionOverridePatch,
} from "@/components/settings/channel/PermissionOverrideEditor";
import { SettingsSection } from "@/components/settings/SettingsSection";
import type { ChannelSettingsVM } from "@/components/settings/channel/channelSettingsModel";
import type { PermissionOverride, Role } from "@/ports";
import { useLanguage } from "@/state/hooks";

export interface NotificationsPaneProps {
  muted: boolean;
  onMutedChange: (muted: boolean) => void;
}

/** This member's own mute for the channel; it saves as soon as it flips. */
export function NotificationsPane({ muted, onMutedChange }: NotificationsPaneProps) {
  const { t } = useLanguage();
  return (
    <SettingsSection title={t("channel.notifications")}>
      <div className="frm ch-frm-flush">
        <div className="frm-row">
          <span className="frm-lab">
            {t("channel.muteNotifications")}
            <span>{t("channel.muteNotificationsDetail")}</span>
          </span>
          <div className="frm-ctl frm-switch">
            <Switch
              checked={muted}
              onChange={onMutedChange}
              label={t("channel.muteNotifications")}
            />
          </div>
        </div>
      </div>
    </SettingsSection>
  );
}

export interface PermissionsPaneProps {
  roles: readonly Role[];
  permissionsSupported: boolean;
  permissionOverrides: readonly PermissionOverride[];
  onSavePermissionOverride: (roleId: string, patch: PermissionOverridePatch) => Promise<void>;
  onOpenRoles: () => void;
}

export function PermissionsPane({
  roles,
  permissionsSupported,
  permissionOverrides,
  onSavePermissionOverride,
  onOpenRoles,
}: PermissionsPaneProps) {
  const { t } = useLanguage();
  return (
    <SettingsSection
      title={t("channel.channelPermissions")}
      action={
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className="ch-roles-link"
          onClick={onOpenRoles}
        >
          <Icon name="shield" size={14} />
          {t("channel.openServerRoles")}
        </Button>
      }
    >
      <p className="fld-note">{t("channel.permissionsDetailText")}</p>
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
    </SettingsSection>
  );
}

export interface DangerPaneProps {
  channel: ChannelSettingsVM;
  supported: boolean;
  onDelete: () => Promise<void>;
}

/**
 * The server's danger zone, for one channel. Deleting asks on the question
 * card the rail's menu asks with, and the card runs the deletion.
 */
export function DangerPane({ channel, supported, onDelete }: DangerPaneProps) {
  const { t } = useLanguage();
  const confirm = useConfirm();

  const askToDelete = () => {
    const subject = channelSubject(t, channel);
    void confirm({ ...deleteChannel(t, subject), run: onDelete });
  };

  return (
    <SettingsSection title={t("server.danger")} className="danger-section">
      <div className="set-row danger">
        <span className="set-lab">
          <b>{t("channel.delete")}</b>
          <span>{t("channel.deleteWarning")}</span>
        </span>
        <span className="set-ctl">
          <Button type="button" variant="danger" disabled={!supported} onClick={askToDelete}>
            {t("channel.delete")}
          </Button>
        </span>
      </div>
    </SettingsSection>
  );
}
