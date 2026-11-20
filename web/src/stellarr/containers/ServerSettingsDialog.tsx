import { useMemo, useSyncExternalStore } from "react";

import { ServerSettings, type SettingsPaneKey } from "@/components/settings/server/ServerSettings";
import { routeAudioOutput } from "@/lib/voice/playback/playback";
import {
  useDirectory,
  useEmoji,
  useLanguage,
  usePorts,
  useSession,
  useUi,
  useVoice,
} from "@/state/hooks";

import { withFreshSecondStep } from "./freshSecondStep";
import type { ServerAdmin } from "./useServerAdmin";

interface ServerSettingsDialogProps extends ServerAdmin {
  open: boolean;
  initialPane?: SettingsPaneKey;
}

/** Waits for a mutation, then reloads the admin lists behind it. */
async function reloadAfter(mutation: Promise<unknown>, reload: () => void) {
  await mutation;
  reload();
}

/** Waits for a mutation, then for the emoji set it changed. */
async function refreshAfter(mutation: Promise<unknown>, refresh: () => Promise<void>) {
  await mutation;
  await refresh();
}

/** The server settings dialog, wired to the admin port. */
export function ServerSettingsDialog({ open, initialPane, data, reload }: ServerSettingsDialogProps) {
  const { admin, soundboard, voiceSettings } = usePorts();
  const directory = useDirectory();
  const { user } = useSession();
  const ui = useUi();
  const voice = useVoice();
  const { t } = useLanguage();
  const { emoji, refresh: refreshEmoji } = useEmoji();
  // The listener's own playback preferences, for the soundboard preview: the
  // speaker calls are routed to, and the level a clip is mixed at.
  const playback = useSyncExternalStore(
    voiceSettings.subscribe,
    voiceSettings.getSnapshot,
    voiceSettings.getSnapshot,
  );

  const channels = useMemo(
    () =>
      directory.channels.map((channel) => ({
        id: channel.id,
        name: channel.name,
        kind: channel.kind,
        ...(channel.sectionId === undefined ? {} : { sectionId: channel.sectionId }),
      })),
    [directory.channels],
  );

  const { server } = directory;
  const { supported } = admin;
  const roleById = new Map(data.roles.map((role) => [role.id, role]));
  const ownerMember = data.members.find((member) => roleById.get(member.roleId)?.isOwner === true);
  const viewerId = user?.id;
  const viewerIsOwner = ownerMember !== undefined && viewerId !== undefined && ownerMember.user.id === viewerId;

  return (
    <ServerSettings
      open={open}
      initialPane={initialPane}
      onClose={ui.closeModal}
      name={server.name}
      mark={directory.serverMark}
      {...(server.iconUrl === undefined ? {} : { iconUrl: server.iconUrl })}
      overview={{
        supported: supported.server,
        iconSupported: supported.serverIcon,
        name: server.name,
        mark: directory.serverMark,
        ...(server.iconUrl === undefined ? {} : { iconUrl: server.iconUrl }),
        createdAt: server.createdAt,
        memberCount: directory.members.length,
        channelCount: directory.channels.length,
        description: server.description ?? "",
        isPublic: server.isPublic ?? false,
        onSave: (patch) => admin.updateServer(patch),
        onPickIcon: async (file) => {
          await admin.uploadServerIcon(file);
        },
        onRemoveIcon: () => admin.removeServerIcon(),
        ...(ownerMember === undefined
          ? {}
          : {
              owner: {
                id: ownerMember.user.id,
                displayName: ownerMember.user.displayName,
                ...(ownerMember.user.avatarUrl === undefined ? {} : { avatarUrl: ownerMember.user.avatarUrl }),
              },
            }),
        viewerIsOwner,
        ...(viewerIsOwner && supported.server
          ? {
              transfer: {
                candidates: data.members
                  .filter((member) => member.user.id !== ownerMember?.user.id)
                  .map((member) => ({
                    id: member.user.id,
                    name: member.user.displayName,
                    ...(roleById.get(member.roleId) === undefined ? {} : { role: roleById.get(member.roleId)!.name }),
                  })),
                onTransfer: async (userId: string) => {
                  await reloadAfter(withFreshSecondStep(() => admin.transferOwnership(userId), t), reload);
                },
              },
            }
          : {}),
      }}
      channels={{
        supported: supported.channels,
        channels,
        onAdd: () => ui.openCreateChannel("text"),
        onEdit: ui.openChannelSettings,
        sectionsSupported: supported.sections,
        sections: directory.sections,
        onAddSection: ui.openCreateSection,
        onEditSection: ui.openSectionSettings,
        onMoveChannel: (move) => reloadAfter(directory.moveChannel(move), reload),
        onMoveSection: (order) => reloadAfter(directory.reorderSections(order), reload),
      }}
      roles={{
        supported: supported.roles,
        roles: data.roles,
        members: data.members,
        onCreate: async (input) => {
          const role = await withFreshSecondStep(() => admin.createRole(input), t);
          reload();
          return role;
        },
        onUpdate: (roleId, patch) =>
          reloadAfter(withFreshSecondStep(() => admin.updateRole(roleId, patch), t), reload),
        onDelete: (roleId) => reloadAfter(admin.deleteRole(roleId), reload),
      }}
      members={{
        supported: supported.members,
        members: data.members,
        roles: data.roles,
        onAssignRole: (userId, roleId) =>
          reloadAfter(withFreshSecondStep(() => admin.assignRole(userId, roleId), t), reload),
        ...(viewerId === undefined ? {} : { viewerId }),
        ...(directory.canModerateMembers && supported.moderation
          ? { onKick: (userId: string) => reloadAfter(admin.kick(userId), reload) }
          : {}),
        ...(directory.canManageBans && supported.moderation
          ? { onBan: (userId: string, reason?: string) => reloadAfter(admin.ban(userId, reason), reload) }
          : {}),
      }}
      invites={{
        supported: supported.invites,
        invites: data.invites,
        onCreate: () => reloadAfter(admin.createInvite(), reload),
        onRevoke: (inviteId) => reloadAfter(admin.revokeInvite(inviteId), reload),
      }}
      emoji={{
        supported: supported.emoji,
        emoji,
        onUpload: (shortcode, file) =>
          refreshAfter(admin.uploadEmoji(shortcode, file), refreshEmoji),
        onDelete: (emojiId) => refreshAfter(admin.deleteEmoji(emojiId), refreshEmoji),
      }}
      soundboard={{
        supported:
          voice.soundboard.supported && voice.soundboard.canManage && supported.soundboard,
        sounds: voice.sounds,
        everyoneCanPlay: voice.soundboard.settings.everyoneCanPlay,
        onEveryoneCanPlayChange: (everyoneCanPlay) => {
          void soundboard.updateSettings({ everyoneCanPlay }).catch(() => {});
        },
        rateLimited: voice.soundboard.settings.rateLimited,
        onRateLimitedChange: (rateLimited) => {
          void soundboard.updateSettings({ rateLimited }).catch(() => {});
        },
        loading: voice.soundboard.loading,
        error: voice.soundboard.error === null ? null : t(voice.soundboard.error),
        onCreate: (input) => soundboard.create(input),
        onUpdate: (soundId, patch) => soundboard.update(soundId, patch),
        onDelete: (soundId) => soundboard.delete(soundId),
        onReorder: (soundIds) => soundboard.reorderSounds(soundIds),
        // The preview plays on the same speaker and at the same level a
        // played clip does, so auditioning one is what hearing it will be.
        playback: {
          soundboardVolume: playback.soundboardVolume,
          route: (element) => routeAudioOutput(element, playback.outputDeviceId),
        },
      }}
      audit={{
        supported: supported.audit,
        entries: data.audit,
        onLoadOlder: (before) => admin.listAuditEntries(before),
        onShow: reload,
      }}
      bans={{
        supported: supported.bans,
        bans: data.bans,
        onUnban: (userId) => reloadAfter(admin.unban(userId), reload),
      }}
      {...(viewerIsOwner && supported.server
        ? {
            onDeleteServer: async () => {
              // Refused for want of a fresh second step, the page leaves to confirm it instead.
              const deleted = await withFreshSecondStep(async () => {
                await admin.deleteServer();
                return true;
              }, t);
              if (deleted === true) window.location.reload();
            },
          }
        : {})}
    />
  );
}
