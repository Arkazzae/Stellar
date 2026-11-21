import type { IconName } from "@stellarr/icons";
import type { SettingsShellGroup, SettingsShellItem } from "@/components/settings/SettingsShell";
import type { Translator } from "@/i18n";
import type { ChannelKind, OpusProfileId } from "@/ports";

/** The editable fields of one channel, as the dialog's form sees them. */
export interface ChannelSettingsVM {
  id: string;
  kind: ChannelKind;
  name: string;
  topic: string;
  slowModeSeconds: number;
  hideAfterInactivityHours: number;
  /** Voice only; the codec profile shared by the call. */
  audioProfile: OpusProfileId;
  userLimit: number;
  /** The section the channel sits in, shown beside its name; none when unsectioned. */
  sectionName?: string;
  /** The rail's groups, for the preview that shows the channel among its neighbours. */
  rail?: readonly RailGroup[];
}

export type ChannelSettingsPaneKey = "overview" | "notifications" | "permissions";

export interface ChannelSettingsNavItem extends SettingsShellItem<ChannelSettingsPaneKey> {
  /** The sentence under the pane's name in the bar. */
  detail: string;
}

export interface ChannelSettingsNavGroup extends SettingsShellGroup<ChannelSettingsPaneKey> {
  items: readonly ChannelSettingsNavItem[];
}

/** A channel as a slice of the rail draws it. */
export interface RailChannel {
  id: string;
  name: string;
  kind: ChannelKind;
  sectionId?: string;
}

/**
 * The channels under one section in the order the rail lists them: its text
 * channels, then its voice ones, each in rail order.
 */
export function sectionChannels(
  channels: readonly RailChannel[],
  sectionId: string,
): { id: string; name: string; kind: ChannelKind }[] {
  const of = (kind: ChannelKind) =>
    channels
      .filter((channel) => channel.sectionId === sectionId && channel.kind === kind)
      .map(({ id, name }) => ({ id, name, kind }));
  return [...of("text"), ...of("voice")];
}

/**
 * One group of the rail as a slice of it draws it: the text or the voice
 * channels outside sections, captioned by their kind, or a section.
 */
export interface RailGroup {
  key: string;
  /** The section it is; absent for the two groups of unsectioned channels. */
  section?: { id: string; name: string };
  /** The kind an unsectioned group holds; a section holds both. */
  kind: ChannelKind | null;
  channels: readonly { id: string; name: string; kind: ChannelKind }[];
}

/** The rail's groups in its order: unsectioned text, unsectioned voice, then each section. */
export function railGroups(
  channels: readonly RailChannel[],
  sections: readonly { id: string; name: string }[],
): RailGroup[] {
  const loose = (kind: ChannelKind) =>
    channels
      .filter((channel) => channel.kind === kind && channel.sectionId === undefined)
      .map(({ id, name }) => ({ id, name, kind }));
  return [
    { key: "text", kind: "text", channels: loose("text") },
    { key: "voice", kind: "voice", channels: loose("voice") },
    ...sections.map((section) => ({
      key: `section:${section.id}`,
      section: { id: section.id, name: section.name },
      kind: null,
      channels: sectionChannels(channels, section.id),
    })),
  ];
}

/**
 * Where a group stands among the others: the nearest group before and after
 * it that has a channel to draw, for a preview that shows its neighbours.
 */
export function railNeighbours(
  groups: readonly RailGroup[],
  index: number,
): { before?: RailGroup; after?: RailGroup } {
  const drawn = (group: RailGroup | undefined) =>
    group !== undefined && group.channels.length > 0 ? group : undefined;
  const before = groups.slice(0, Math.max(index, 0)).reverse().find((group) => drawn(group));
  const after = groups.slice(index + 1).find((group) => drawn(group));
  return {
    ...(before === undefined ? {} : { before }),
    ...(after === undefined ? {} : { after }),
  };
}

/** The mark of a channel kind, used wherever a channel is drawn. */
export const channelIcon = (kind: ChannelKind): IconName => (kind === "voice" ? "speaker" : "hash");

/** A text channel is written with its hash, a voice channel without one. */
export const channelLabel = (kind: ChannelKind, name: string): string =>
  `${kind === "voice" ? "" : "#"}${name}`;

/**
 * The groups and words of the server settings: a general group with the
 * channel's own panes, then management with access. Deletion is not a pane:
 * it sits alone at the foot of the navigation, as it does for a server.
 *
 * A member who cannot manage the channel only gets notifications, and only on
 * a text channel; the dialog does not open for them on a voice channel.
 */
export function navigationFor(
  kind: ChannelKind,
  canManageChannel: boolean,
  t: Translator,
): readonly ChannelSettingsNavGroup[] {
  const voice = kind === "voice";
  const notifications: ChannelSettingsNavItem = {
    key: "notifications",
    label: t("channel.notifications"),
    icon: "bell",
    detail: t("channel.notificationsDetail"),
  };

  if (!canManageChannel && !voice) {
    return [{ caption: t("server.general"), items: [notifications] }];
  }

  const overview: ChannelSettingsNavItem = {
    key: "overview",
    label: t("server.overview"),
    icon: channelIcon(kind),
    detail: voice ? t("channel.nameLimitVisibility") : t("channel.nameTopicVisibility"),
  };

  return [
    {
      caption: t("server.general"),
      items: voice ? [overview] : [overview, notifications],
    },
    {
      caption: t("server.management"),
      items: [
        {
          key: "permissions",
          label: t("channel.permissions"),
          icon: "shield",
          detail: t("channel.permissionsDetail"),
        },
      ],
    },
  ];
}

/**
 * Whether the dialog offers deletion: wherever it offers management, which is
 * where the danger pane used to be.
 */
export function offersDeletion(kind: ChannelKind, canManageChannel: boolean): boolean {
  return canManageChannel || kind === "voice";
}

export function initialPane(
  channel: ChannelSettingsVM | null,
  canManageChannel: boolean,
): ChannelSettingsPaneKey {
  return channel?.kind === "text" && !canManageChannel ? "notifications" : "overview";
}
