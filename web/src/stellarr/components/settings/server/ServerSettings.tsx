import { useId, useState } from "react";

import { Icon, type IconName } from "@stellarr/icons";
import { usePrompt } from "@/components/primitives/ConfirmDialog";
import { Modal } from "@/components/primitives/Modal";
import { SettingsShell } from "@/components/settings/SettingsShell";
import { SettingsGuardProvider, useSettingsGuardHost } from "@/components/settings/settingsGuard";
import { AuditPane, type AuditPaneProps } from "@/components/settings/server/panes/AuditPane";
import { BansPane, type BansPaneProps } from "@/components/settings/server/panes/BansPane";
import { ChannelsPane, type ChannelsPaneProps } from "@/components/settings/server/panes/ChannelsPane";
import { EmojiPane, type EmojiPaneProps } from "@/components/settings/server/panes/EmojiPane";
import { InvitesPane, type InvitesPaneProps } from "@/components/settings/server/panes/InvitesPane";
import { MembersPane, type MembersPaneProps } from "@/components/settings/server/panes/MembersPane";
import { OverviewPane, type OverviewPaneProps } from "@/components/settings/server/panes/OverviewPane";
import { RolesPane, type RolesPaneProps } from "@/components/settings/server/panes/RolesPane";
import { ServerMark } from "@/components/settings/server/panes/ServerMark";
import { SoundboardPane, type SoundboardPaneProps } from "@/components/settings/server/panes/SoundboardPane";
import { deleteServer, serverSubject } from "@/components/confirmations";
import type { MessageKey } from "@/i18n";
import { useLanguage } from "@/state/hooks";

/** The panes' `data-pane` keys, kept from the first version so links into a pane still land. */
export type SettingsPaneKey =
  | "prz"
  | "kan"
  | "emoji"
  | "board"
  | "ludzie"
  | "role"
  | "zapr"
  | "bany"
  | "audyt";

interface NavItem {
  label: MessageKey;
  icon: IconName;
  /** The sentence under the title in the bar. */
  detail: MessageKey;
}

/** One table feeds both the navigation and the bar's title, so they cannot disagree. */
const ITEMS: Record<SettingsPaneKey, NavItem> = {
  prz: { label: "srv.nav.overview", icon: "server", detail: "srv.detail.overview" },
  kan: { label: "srv.nav.channels", icon: "hash2", detail: "srv.detail.channels" },
  emoji: { label: "srv.nav.emoji", icon: "smiley", detail: "srv.detail.emoji" },
  board: { label: "srv.nav.soundboard", icon: "board", detail: "srv.detail.soundboard" },
  ludzie: { label: "srv.nav.members", icon: "users", detail: "srv.detail.members" },
  role: { label: "srv.nav.roles", icon: "shield", detail: "srv.detail.roles" },
  zapr: { label: "srv.nav.invites", icon: "link", detail: "srv.detail.invites" },
  bany: { label: "srv.nav.bans", icon: "ban", detail: "srv.detail.bans" },
  audyt: { label: "srv.nav.audit", icon: "clipboard", detail: "srv.detail.audit" },
};

/**
 * Two groups, as user settings have: what the server is, then who is on it
 * and what they may do.
 */
const GROUPS: readonly { caption: MessageKey; panes: readonly SettingsPaneKey[] }[] = [
  { caption: "srv.group.server", panes: ["prz", "kan", "emoji", "board"] },
  { caption: "srv.group.people", panes: ["ludzie", "role", "zapr", "bany", "audyt"] },
];

export interface ServerSettingsProps {
  open: boolean;
  /** The pane the dialog opens on; the overview unless asked otherwise. */
  initialPane?: SettingsPaneKey;
  onClose: () => void;
  /** Server name, in the sidebar brand and as the dialog's accessible name. */
  name: string;
  /** Two-letter mark, shown when the server has no icon. */
  mark: string;
  iconUrl?: string;
  overview: OverviewPaneProps;
  channels: ChannelsPaneProps;
  roles: RolesPaneProps;
  members: MembersPaneProps;
  invites: InvitesPaneProps;
  emoji: EmojiPaneProps;
  soundboard: SoundboardPaneProps;
  audit: AuditPaneProps;
  bans: BansPaneProps;
  /**
   * Deletes the server; given to the owner only. It sits alone at the bottom
   * of the navigation, the way Log out does in user settings.
   */
  onDeleteServer?: () => Promise<void>;
}

/** The full-screen `.sv-wrap` settings dialog. */
export function ServerSettings({ open, onClose, ...content }: ServerSettingsProps) {
  const titleId = useId();
  // Escape and a press outside close through the same guard as the cross, so
  // an unsaved draft is never thrown away by any of them.
  const guard = useSettingsGuardHost();

  // The modal mounts its children only while it is open, so the layout below
  // starts on the overview every time the dialog is reopened.
  return (
    <Modal
      open={open}
      onClose={() => guard.attempt(onClose)}
      labelledBy={titleId}
      className="sv-wrap"
    >
      <SettingsGuardProvider guard={guard}>
        <SettingsLayout titleId={titleId} onClose={onClose} {...content} />
      </SettingsGuardProvider>
    </Modal>
  );
}

type LayoutProps = Omit<ServerSettingsProps, "open"> & { titleId: string };

function SettingsLayout({
  titleId,
  initialPane = "prz",
  onClose,
  name,
  mark,
  iconUrl,
  overview,
  channels,
  roles,
  members,
  invites,
  emoji,
  soundboard,
  audit,
  bans,
  onDeleteServer,
}: LayoutProps) {
  const { locale, t } = useLanguage();
  const prompt = usePrompt();
  const [active, setActive] = useState<SettingsPaneKey>(initialPane);
  const current = ITEMS[active];

  /**
   * Only the visible pane is mounted, so nine live data lists do not sit behind
   * `display: none`.
   */
  const pane = () => {
    switch (active) {
      case "prz":
        return <OverviewPane {...overview} />;
      case "kan":
        return <ChannelsPane {...channels} />;
      case "emoji":
        return <EmojiPane {...emoji} />;
      case "board":
        return <SoundboardPane {...soundboard} />;
      case "ludzie":
        return <MembersPane {...members} />;
      case "role":
        return <RolesPane {...roles} />;
      case "zapr":
        return <InvitesPane {...invites} />;
      case "bany":
        return <BansPane {...bans} />;
      case "audyt":
        return <AuditPane {...audit} />;
    }
  };

  const groups = GROUPS.map((group) => ({
    caption: t(group.caption),
    items: group.panes.map((key) => ({ key, label: t(ITEMS[key].label), icon: ITEMS[key].icon })),
  }));

  // The card runs the deletion itself, so a refusal is said there and not on
  // the pane behind it.
  const askToDelete = () => {
    if (onDeleteServer === undefined) return;
    const subject = serverSubject(t, locale, {
      name,
      mark,
      ...(iconUrl === undefined ? {} : { iconUrl }),
      memberCount: overview.memberCount,
      channelCount: overview.channelCount,
    });
    void prompt({ ...deleteServer(t, subject, name), run: () => onDeleteServer() });
  };

  return (
    <SettingsShell
      titleId={titleId}
      brand={{ name, detail: t("server.settings"), mark: <ServerMark mark={mark} iconUrl={iconUrl} /> }}
      title={t(current.label)}
      detail={t(current.detail)}
      closeLabel={t("server.close")}
      onClose={onClose}
      navLabel={t("server.sections")}
      groups={groups}
      active={active}
      onSelect={setActive}
      footer={
        onDeleteServer === undefined ? undefined : (
          <button
            type="button"
            className="sv-item danger"
            title={t("srv.delete.button")}
            onClick={askToDelete}
          >
            <span className="sv-ico" aria-hidden="true">
              <Icon name="trash" size={18} />
            </span>
            {t("srv.delete.button")}
          </button>
        )
      }
    >
      {pane()}
    </SettingsShell>
  );
}
