import type { UserMenuActions } from "@/components/roster/UserMenu";
import {
  useAccount,
  useDirectory,
  useFriends,
  usePorts,
  useProfile,
  useVoiceLive,
} from "@/state/hooks";

/**
 * What a right-click on a person does wherever people are listed.
 *
 * The profile card, a message, the listener's own strip in the current call and
 * moderation are the same everywhere; the roster adds the friendship items on
 * top. Moderation needs both halves: the member must hold the permission, and
 * the adapter must have a persisted endpoint behind it.
 *
 * The listener's strip is read from the call when the menu asks, and the open
 * menu watches it, so the lists that offer this menu do not render for every
 * snapshot of the call.
 */
export function useMemberMenu(): UserMenuActions {
  const { voiceChannels, canModerateMembers, canManageBans } = useDirectory();
  const { admin } = usePorts();
  const voice = useVoiceLive();
  const profile = useProfile();
  const friends = useFriends();
  const { preferences } = useAccount();

  const seatOf = (userId: string) =>
    voiceChannels.find((channel) =>
      channel.occupants.some((person) => person.id === userId),
    );

  return {
    onOpenProfile: profile.toggle,
    onMessage: (userId) => friends.messagePerson(userId),
    participantVolumeOf: voice.participantVolumeOf,
    onParticipantVolumeChange: voice.setParticipantVolume,
    participantGateOf: voice.participantGateOf,
    onParticipantGateChange: voice.setParticipantGate,
    participantSuppressionOf: voice.participantSuppressionOf,
    onParticipantSuppressionChange: voice.setParticipantSuppression,
    watchParticipants: voice.subscribe,
    canKick: canModerateMembers && admin.supported.moderation,
    canBan: canManageBans && admin.supported.moderation,
    ...(admin.kickFromChannel === undefined
      ? {}
      : {
          canKickFromChannel: (userId: string) => seatOf(userId) !== undefined,
          onKickFromChannel: (userId: string) => {
            const seat = seatOf(userId);
            if (seat !== undefined) void admin.kickFromChannel?.(seat.id, userId);
          },
        }),
    onKick: (userId) => void admin.kick(userId),
    onBan: (userId, reason) =>
      void admin.ban(userId, reason.length === 0 ? undefined : reason),
    developerMode: preferences.developerMode,
  };
}
