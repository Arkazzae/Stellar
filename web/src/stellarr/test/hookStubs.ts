import { vi } from "vitest";

import { IDLE_SOUNDBOARD } from "@/components/voice/soundboardLive";
import type { Ports } from "@/ports";
import type {
  AccountValue,
  ActivitiesValue,
  ChatValue,
  DirectoryValue,
  EmojiValue,
  EncryptionValue,
  FriendsValue,
  ProfileValue,
  SessionValue,
  UiValue,
  VoiceLive,
  VoiceValue,
} from "@/state/hooks";
import { useLanguage } from "@/state/LanguageProvider";

/**
 * Replaces `@/state/hooks` for container tests, so a container can be rendered
 * against hand-written state without composing the providers:
 *
 *     vi.mock("@/state/hooks", async () => (await import("@/test/hookStubs")).hooksModule);
 *
 * `stubs` holds what each hook returns. The defaults describe an empty server
 * with one signed-in user; a test patches the pieces it cares about with
 * `setStubs` and calls `resetStubs` between tests.
 */
export interface HookStubs {
  account: AccountValue;
  activities: ActivitiesValue;
  chat: ChatValue;
  directory: DirectoryValue;
  emoji: EmojiValue;
  encryption: EncryptionValue;
  friends: FriendsValue;
  ports: Ports;
  profile: ProfileValue;
  session: SessionValue;
  ui: UiValue;
  voice: VoiceValue;
  voiceLive: VoiceLive;
}

/** The stores the containers read are far wider than any one test needs. */
function stub<T>(members: object): T {
  return members as T;
}

/** One object for the whole run: `useSyncExternalStore` needs a stable snapshot. */
const VOICE_SETTINGS = { soundboardVolume: 50, noiseSuppression: "off", opusProfile: "auto" };

function defaults(): HookStubs {
  return {
    account: stub<AccountValue>({
      account: { customStatus: "" },
      preferences: {
        desktopNotifications: true,
        mentionPopups: true,
        callRingtone: true,
        developerMode: false,
        mutedChannelIds: [],
      },
      update: vi.fn(async () => {}),
      setPreferences: vi.fn(async () => {}),
    }),
    activities: stub<ActivitiesValue>({
      live: new Map(),
      voice: null,
      personal: null,
      joining: null,
      joinFailure: null,
      ended: null,
      picker: null,
      openPicker: vi.fn(),
      closePicker: vi.fn(),
      join: vi.fn(),
      launch: vi.fn(async () => {}),
      leave: vi.fn(),
      end: vi.fn(async () => {}),
      lose: vi.fn(),
      slots: { voice: null, personal: null },
      publishSlot: { voice: vi.fn(), personal: vi.fn() },
    }),
    chat: stub<ChatValue>({
      thread: null,
      activeThreadId: null,
      threadsByChannel: {},
      openingThreadId: null,
      openThread: vi.fn(),
      closeThread: vi.fn(),
      unreadByChannel: {},
      markChannelRead: vi.fn(),
    }),
    directory: stub<DirectoryValue>({
      server: { id: "srv-orbit", name: "Orbit" },
      servers: [],
      textChannels: [],
      voiceChannels: [],
      sections: [],
      members: [],
      people: [],
      inVoice: [],
      canAccessServerSettings: false,
      canManageServer: false,
      canManageChannels: false,
      canModerateMembers: false,
      canManageBans: false,
      moveChannel: vi.fn(async () => {}),
      moveChannelToSection: vi.fn(async () => {}),
      reorderSections: vi.fn(async () => {}),
      selectServer: vi.fn(),
    }),
    emoji: stub<EmojiValue>({ emoji: [] }),
    encryption: stub<EncryptionValue>({
      status: {
        support: "supported",
        accountKey: null,
        backup: null,
        unlocked: false,
        deviceId: null,
        deviceLimit: false,
        revision: 0,
      },
      dialog: null,
      openDialog: vi.fn(),
      closeDialog: vi.fn(),
      switching: null,
      switchError: null,
      switchOn: vi.fn(),
      switchOff: vi.fn(),
      runSwitch: vi.fn(),
      dismissedRequests: new Set<string>(),
      dismissRequest: vi.fn(),
    }),
    friends: stub<FriendsValue>({
      friends: [],
      requests: [],
      blocked: [],
      conversations: [],
      incoming: 0,
      unread: 0,
      active: null,
      encryptionLines: { liveRequestId: null, declinedIds: new Set<string>() },
      openAttachment: vi.fn(async () => null),
      messagePerson: vi.fn(),
      callConversation: vi.fn(),
      declineCall: vi.fn(),
      ringing: false,
      addFriend: vi.fn(async () => "sent"),
      accept: vi.fn(),
      decline: vi.fn(),
      cancel: vi.fn(),
      remove: vi.fn(),
      block: vi.fn(),
      unblock: vi.fn(),
    }),
    ports: stub<Ports>({
      admin: {
        supported: { channels: false, moderation: false, sections: false },
        kick: vi.fn(async () => {}),
        ban: vi.fn(async () => {}),
        deleteChannel: vi.fn(async () => {}),
        listMembers: vi.fn(async () => []),
        listRoles: vi.fn(async () => []),
      },
      voiceSettings: {
        getSnapshot: () => VOICE_SETTINGS,
        subscribe: () => () => {},
        update: vi.fn(),
      },
      soundboard: {
        live: IDLE_SOUNDBOARD,
      },
      encryption: {
        listDevices: vi.fn(async () => []),
        safetyCode: vi.fn(async () => null),
        verification: vi.fn(async () => null),
      },
    }),
    profile: stub<ProfileValue>({ open: null, toggle: vi.fn(), close: vi.fn() }),
    session: stub<SessionValue>({
      user: { id: "u-maja", username: "maja", displayName: "Maja", presence: "online" },
      presence: "online",
      setPresence: vi.fn(),
      reload: vi.fn(),
      logout: vi.fn(async () => {}),
    }),
    ui: stub<UiValue>({
      space: "server",
      roomView: "text",
      activeChannelId: null,
      activeDmId: null,
      railOpen: false,
      rosterOpen: true,
      setRailOpen: vi.fn(),
      setRosterOpen: vi.fn(),
      openModal: vi.fn(),
      openChannelSettings: vi.fn(),
      openCreateChannel: vi.fn(),
      openSectionSettings: vi.fn(),
      openFriends: vi.fn(),
      newConversation: vi.fn(),
      friendsSearchRequested: false,
      takeFriendsSearch: vi.fn(),
      profilePanelOpen: false,
      setProfilePanelOpen: vi.fn(),
      toggleProfilePanel: vi.fn(),
      callChatOpen: false,
      setCallChatOpen: vi.fn(),
      toggleCallChat: vi.fn(),
      inConversationCall: false,
      openDm: vi.fn(),
      activityOpen: false,
      openActivity: vi.fn(),
      openServer: vi.fn(),
      setFriendsTab: vi.fn(),
      selectTextChannel: vi.fn(),
      joinVoiceChannel: vi.fn(),
      leaveCall: vi.fn(),
      returnToChannel: vi.fn(),
    }),
    voice: stub<VoiceValue>({
      call: null,
      channelId: null,
      sounds: [],
      soundboard: {
        sounds: [],
        settings: { everyoneCanPlay: true, rateLimited: true },
        canManage: false,
        canPlay: true,
        supported: true,
        loading: false,
        error: null,
      },
      soundboardEnabled: false,
      cameraSupported: false,
      applySpeaking: vi.fn((people: unknown[]) => people),
      participantVolumes: new Map(),
      participantGates: new Map(),
      participantSuppressions: new Map(),
      setParticipantVolume: vi.fn(),
      setParticipantGate: vi.fn(),
      setParticipantSuppression: vi.fn(),
      toggleMute: vi.fn(),
      toggleDeafen: vi.fn(),
      toggleCamera: vi.fn(),
      toggleShare: vi.fn(),
      playSound: vi.fn(),
    }),
    voiceLive: stub<VoiceLive>({
      cloneLocalCamera: () => undefined,
      subscribe: () => () => undefined,
      participantVolumeOf: () => undefined,
      participantGateOf: () => undefined,
      participantSuppressionOf: () => undefined,
      setParticipantVolume: vi.fn(),
      setParticipantGate: vi.fn(),
      setParticipantSuppression: vi.fn(),
    }),
  };
}

export const stubs: HookStubs = defaults();

/** Back to the defaults, with fresh spies. */
export function resetStubs(): void {
  Object.assign(stubs, defaults());
}

/** Merges members into the named stores; the rest of each store is kept. */
export function setStubs(patch: {
  [K in keyof HookStubs]?: Partial<HookStubs[K]>;
}): void {
  for (const key of Object.keys(patch) as (keyof HookStubs)[]) {
    stubs[key] = { ...stubs[key], ...patch[key] } as never;
  }
}

export const hooksModule = {
  useAccount: () => stubs.account,
  useActivities: () => stubs.activities,
  useActivitySlot: (slot: "voice" | "personal") => stubs.activities.publishSlot[slot],
  useChat: () => stubs.chat,
  // Each slice of the chat is a part of its whole value.
  useChatRail: () => stubs.chat,
  useChatRoom: () => stubs.chat,
  useChatTranscript: () => stubs.chat,
  useDirectory: () => stubs.directory,
  useEmoji: () => stubs.emoji,
  useEncryption: () => stubs.encryption,
  useFriends: () => stubs.friends,
  usePorts: () => stubs.ports,
  useProfile: () => stubs.profile,
  useSession: () => stubs.session,
  useUi: () => stubs.ui,
  useVoice: () => stubs.voice,
  // The dock's slice is a part of the call's value.
  useVoiceCall: () => stubs.voice,
  useVoiceLive: () => stubs.voiceLive,
  useLanguage,
};
