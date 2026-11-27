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
 * Kicking and banning as the member menu offers them: who may, and the deeds
 * the question card runs. The composer's /kick and /ban ask through the same
 * cards with these, so the two doors cannot disagree about who may do what.
 * Both halves are needed: the member must hold the permission, and the
 * adapter must have a persisted endpoint behind it.
 */
export function useMemberModeration() {
  const { members, canModerateMembers, canManageBans } = useDirectory();
  const { admin } = usePorts();
  return {
    canKick: canModerateMembers && admin.supported.moderation,
    canBan: canManageBans && admin.supported.moderation,
    // Each deed hands its promise back, so the question card waits for the
    // server and says so when it refuses.
    onKick: (userId: string) => admin.kick(userId),
    onBan: (userId: string, reason: string) =>
      admin.ban(userId, reason.length === 0 ? undefined : reason),
    usernameOf: (userId: string) => members.find((member) => member.id === userId)?.username,
  };
}

/**
 * What a right-click on a person does wherever people are listed.
 *
 * The profile card, a message, the listener's own strip in the current call and
 * moderation are the same everywhere; the roster adds the friendship items on
 * top; moderation is `useMemberModeration`'s.
 *
 * The listener's strip is read from the call when the menu asks, and the open
 * menu watches it, so the lists that offer this menu do not render for every
 * snapshot of the call.
 */
export function useMemberMenu(): UserMenuActions {
  const { voiceChannels } = useDirectory();
  const { admin } = usePorts();
  const moderation = useMemberModeration();
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
    canKick: moderation.canKick,
    canBan: moderation.canBan,
    ...(admin.kickFromChannel === undefined
      ? {}
      : {
          canKickFromChannel: (userId: string) => seatOf(userId) !== undefined,
          onKickFromChannel: async (userId: string) => {
            const seat = seatOf(userId);
            if (seat !== undefined) await admin.kickFromChannel?.(seat.id, userId);
          },
        }),
    onKick: moderation.onKick,
    onBan: moderation.onBan,
    usernameOf: moderation.usernameOf,
    developerMode: preferences.developerMode,
  };
}
