/**
 * The composition root: the one place that knows which implementation backs
 * which port. Components, containers and tests above it see only `Ports`.
 *
 * Every port is backed by the real HTTP, SignalR or WebRTC implementation. A
 * platform that brings its own transport (the desktop client) passes it in
 * `RuntimeOptions`; nothing above this file changes.
 */

import {
  createHttpApiClient,
  type AdminApiClient,
  type ApiClient,
  type SocialApiClient,
} from "@/lib/api/client";
import { encryptionCalls, type EncryptionApiClient } from "@/lib/api/calls/encryption";
import { isUnauthorized } from "@/lib/api/errors";
import type { Bootstrap, RtcConfig } from "@/lib/api/types";
import { createPasskey, getPasskey, passkeysSupported } from "@/lib/auth/webauthn";
import { DeferredRealtimeClient } from "@/lib/realtime/deferred";
import type { RealtimeClient } from "@/lib/realtime/types";
import { VoiceSettingsStore } from "@/lib/voice/settings";
import type { Account, Ports, User } from "@/ports";
import type { DesktopPort } from "@/ports/desktop";

import { createAccountAdapter } from "./account";
import { createServerAdminAdapter } from "./admin";
import { createChatAdapter, createTypingAdapter } from "./chat";
import { createDirectoryAdapter } from "./directory";
import { callKeySource, createEncryption } from "./encryption";
import { createFriendsAdapter } from "./friends";
import { createMessageActionsAdapter, ReactionStore } from "./messageActions";
import { createNotificationDeliveryAdapter } from "./notificationDelivery";
import { createNotificationAdapter } from "./notifications";
import { createConnectionAdapter, createPresenceAdapter } from "./presence";
import { ChannelReadStore } from "./readState";
import { createSoundboardAdapter } from "./soundboard";
import { DirectoryStore } from "./store";
import { loadedSignedInCode, loadSignedInCode } from "./signedInLoader";
import { createVoiceSettingsAdapter } from "./voiceSettings";

export interface Runtime {
  ports: Ports;
  /** Seeds every shared store from the boot payload. */
  hydrate(bootstrap: Bootstrap): Promise<void>;
  /** The raw realtime client, so the session can start and stop it. */
  realtime: RealtimeClient;
  dispose(): void;
}

export interface RuntimeOptions {
  /**
   * A platform transport. The browser uses same-origin HTTP, while the desktop
   * client supplies its native broker without changing any UI-facing port.
   */
  api?: ApiClient & SocialApiClient & AdminApiClient;
  /** Browser SignalR by default; the desktop client keeps its socket in Rust. */
  realtime?: RealtimeClient;
  /** Operating-system preferences, supplied by the desktop shell only. */
  desktop?: DesktopPort;
  /** The encryption routes; same-origin HTTP by default. */
  encryption?: EncryptionApiClient;
}

/** Copies the profile fields of the signed-in account onto their directory row. */
function withProfileOf(member: User, account: Account): User {
  const { avatarUrl: _avatar, bannerUrl: _banner, ...rest } = member;
  return {
    ...rest,
    displayName: account.displayName,
    customStatus: account.customStatus,
    bio: account.bio,
    pronouns: account.pronouns,
    ...(account.avatarUrl === undefined ? {} : { avatarUrl: account.avatarUrl }),
    ...(account.bannerUrl === undefined ? {} : { bannerUrl: account.bannerUrl }),
  };
}

/**
 * Keeps the directory in step with what the hub announces about the server and
 * the people on it. Returns one function that cancels every subscription.
 */
function followHub(realtime: RealtimeClient, store: DirectoryStore): () => void {
  const isCurrent = (serverId: string): boolean =>
    store.getSnapshot().server?.id === serverId;
  const subscriptions = [
    realtime.on("UserUpdated", (user) => store.setUser(user)),
    realtime.on("ServerUpdated", (server) => store.setServer(server)),
    realtime.on("ChannelLayoutChanged", ({ serverId, channels }) => {
      if (isCurrent(serverId)) store.setChannelLayout(channels);
    }),
    realtime.on("SectionOrderChanged", ({ serverId, sectionIds }) => {
      if (isCurrent(serverId)) store.setSectionOrder(sectionIds);
    }),
    realtime.on("PermissionsChanged", ({ serverId, permissions }) => {
      if (isCurrent(serverId)) store.setPermissions(permissions);
    }),
    realtime.on("ServerMemberRemoved", ({ serverId, userId }) => {
      if (isCurrent(serverId)) store.removeMember(userId);
    }),
  ];
  return () => {
    for (const unsubscribe of subscriptions) unsubscribe();
  };
}

/**
 * Brings the directory back in step whenever the hub comes back. The hub does
 * not replay what it sent while the socket was down, so a seat, a presence or
 * a channel that changed during an outage would stay wrong until a reload;
 * the bootstrap says how things stand now. The first connection is left
 * alone, since the bootstrap that hydrated the runtime is current then.
 */
function resyncAfterReconnects(
  realtime: RealtimeClient,
  store: DirectoryStore,
  bootstrap: () => Promise<Bootstrap>,
): () => void {
  let status = realtime.getStatus();
  let connectedBefore = status === "connected";
  let running = false;
  let again = false;

  const resync = async (): Promise<void> => {
    if (running) {
      again = true;
      return;
    }
    running = true;
    try {
      do {
        again = false;
        const mark = store.mark();
        try {
          store.resync(await bootstrap(), mark);
        } catch {
          // The hub's own events carry on, and the next reconnect tries again.
        }
      } while (again);
    } finally {
      running = false;
    }
  };

  return realtime.subscribeStatus(() => {
    const next = realtime.getStatus();
    const reconnected = next === "connected" && status !== "connected" && connectedBefore;
    if (next === "connected") connectedBefore = true;
    status = next;
    if (reconnected) void resync();
  });
}

/** The browser's socket, fetched with the signed-in code and not before it is needed. */
const loadRealtime = () => loadSignedInCode().then((code) => code.createSignalRClient());

export function createRuntime(options: RuntimeOptions = {}): Runtime {
  const api = options.api ?? createHttpApiClient();
  const realtime = options.realtime ?? new DeferredRealtimeClient(loadRealtime);
  const store = new DirectoryStore();
  const voiceSettingsStore = new VoiceSettingsStore();
  const reactionStore = new ReactionStore();
  const readStore = new ChannelReadStore();
  let rtcConfig: RtcConfig | undefined;

  // The voice client comes with the signed-in code, which `loadApp` fetches
  // along with the app, so it is there whenever a call can be started.
  const createVoice: Ports["createVoice"] = (voiceOptions) =>
    loadedSignedInCode().createWebRtcVoice(
      {
        api,
        realtime,
        rtcConfig,
        settings: voiceSettingsStore,
        // A private call's room is its conversation, whose keys it uses.
        callEncryption: callKeySource(encryption.client),
      },
      voiceOptions,
    );

  const account = createAccountAdapter(api);
  const notificationDelivery = createNotificationDeliveryAdapter(api);
  const encryption = createEncryption(options.encryption ?? encryptionCalls(), realtime);
  const friends = createFriendsAdapter(api, realtime, encryption.client);
  const soundboard = createSoundboardAdapter(api, realtime, voiceSettingsStore.getSnapshot);
  const messageActions = createMessageActionsAdapter(api, realtime, reactionStore);
  const typing = createTypingAdapter(realtime);
  const chat = createChatAdapter(api, realtime, reactionStore, readStore);
  const admin = createServerAdminAdapter(
    api,
    () => store.getSnapshot().permissions,
    (iconUrl) => store.setServerIcon(iconUrl),
    (channelId, patch) => store.setChannelSettings(channelId, patch),
    (server) => store.setServer(server),
    (channel) => store.addChannel(channel),
    (channelId) => store.removeChannel(channelId),
    (section) => store.addSection(section),
    (section) => store.setSection(section),
    (sectionId) => store.removeSection(sectionId),
  );

  const stopFollowingHub = followHub(realtime, store);
  const stopResyncing = resyncAfterReconnects(realtime, store, () => api.bootstrap());
  // A profile edit reaches the directory at once, without waiting for the
  // hub's echo, so your own row never lags behind your account card.
  const stopMirroringAccount = account.onChanged(() => {
    const current = account.getAccount();
    const member = store.getSnapshot().members.find((item) => item.id === current.id);
    if (member !== undefined) store.setUser(withProfileOf(member, current));
  });

  const ports: Ports = {
    ...(options.desktop === undefined ? {} : { desktop: options.desktop }),
    session: {
      setupRequired: () => api.setupRequired(),
      instanceName: () => api.instanceName(),
      invitePreview: (code) => api.invitePreview(code),
      signInProviders: () => api.signInProviders(),
      pendingIdentity: () => api.pendingIdentity(),
      dropPendingIdentity: () => api.dropPendingIdentity(),
      bootstrap: () => api.bootstrap(),
      login: (input) => api.login(input),
      register: (input) => api.register(input),
      requestPasswordReset: (email) => api.requestPasswordReset(email),
      resetPassword: (token, newPassword) => api.resetPassword(token, newPassword),
      confirmEmailChange: (token) => api.confirmEmailChange(token),
      logout: () => api.logout(),
      check: async () => (await api.session()) !== null,
      passkeysSupported: () => passkeysSupported(),
      passkeySignInAvailable: () => api.passkeySignInAvailable(),
      signInWithPasskey: async ({ keepSignedIn }) => {
        const response = await getPasskey(await api.passkeySignInOptions());
        return api.signInWithPasskey({ response, ...(keepSignedIn === false ? { keepSignedIn } : {}) });
      },
      challenge: {
        async pending() {
          try {
            return await api.pendingChallenge();
          } catch (error) {
            // Nothing pending, or it expired: the visitor starts over.
            if (isUnauthorized(error)) return null;
            throw error;
          }
        },
        cancel: () => api.cancelChallenge(),
        verify: (input) => api.verifyChallenge(input),
        sendEmailCode: (challengeId, methodId) => api.sendChallengeEmailCode(challengeId, methodId),
        startTotp: (challengeId, name) => api.startTotpEnrollment(challengeId, name),
        confirmTotp: (input) => api.confirmTotpEnrollment(input),
        startEmail: (challengeId, name) => api.startEmailEnrollment(challengeId, name),
        confirmEmail: (input) => api.confirmEmailEnrollment(input),
        redeemRecovery: (credential, keepSignedIn) => api.redeemRecoveryCredential(credential, keepSignedIn),
        answerWithPasskey: async ({ challengeId, rememberDevice }) => {
          const response = await getPasskey(await api.passkeyChallengeOptions(challengeId));
          return api.verifyPasskey({ challengeId, response, ...(rememberDevice ? { rememberDevice } : {}) });
        },
        createPasskey: async ({ challengeId, name, rememberDevice }) => {
          const response = await createPasskey(await api.startPasskeyEnrollment(challengeId));
          return api.confirmPasskeyEnrollment({
            challengeId,
            name,
            response,
            ...(rememberDevice ? { rememberDevice } : {}),
          });
        },
      },
    },
    account,
    notificationDelivery,
    security: {
      get: () => api.getAccountSecurity(),
      startEnrollment: (password) => api.startMfaEnrollment(password),
      startReverification: () => api.startMfaReverification(),
      removeMethod: (methodId, password) => api.removeMfaMethod(methodId, password),
      regenerateRecoveryCodes: (password) => api.regenerateRecoveryCodes(password),
      revokeTrustedDevice: (deviceId) => api.revokeTrustedDevice(deviceId),
      listSessions: () => api.listOwnSessions(),
      revokeSession: (sessionId) => api.revokeOwnSession(sessionId),
      revokeOtherSessions: () => api.revokeOtherSessions(),
      startIdentityLink: (providerId, password) => api.startIdentityLink(providerId, password),
      removeIdentity: (identityId, password) => api.removeIdentity(identityId, password),
      startReauthentication: (providerId) => api.startReauthentication(providerId),
      setPassword: (newPassword) => api.setPassword(newPassword),
    },
    chat,
    messageActions,
    typing,
    presence: createPresenceAdapter(store, realtime),
    connection: createConnectionAdapter(realtime),
    directory: createDirectoryAdapter(
      store,
      (orderedIds, move) => api.reorderChannels(orderedIds, move),
      (orderedIds) => api.reorderSections(orderedIds),
      {
        create: (input) => api.createServer(input),
        discover: () => api.discoverServers(),
        join: (serverId) => api.joinServer(serverId),
        joinInvite: (code) => api.joinServerInvite(code),
      },
    ),
    admin,
    friends,
    encryption: encryption.port,
    notifications: createNotificationAdapter(api, realtime),
    soundboard,
    voiceSettings: createVoiceSettingsAdapter(voiceSettingsStore),
    createVoice,
  };

  return {
    ports,
    realtime,
    async hydrate(bootstrap) {
      rtcConfig = bootstrap.rtc;
      realtime.configureRtc?.(rtcConfig);
      // Device settings are local but not shared between accounts on one
      // browser profile; this also migrates the old unscoped entry once.
      voiceSettingsStore.setOwner(bootstrap.user.id);
      store.hydrate(bootstrap);
      reactionStore.setSelfUserId(bootstrap.user.id);
      reactionStore.ingest(bootstrap.messages);
      readStore.hydrate(bootstrap.channelReads);
      await account.hydrate(bootstrap.user);
      // Not awaited: only an encrypted message waits for the keys, and the
      // friends adapter decodes through the same client, which waits for them.
      void encryption.client.start(bootstrap.user.id);
      await Promise.all([friends.load(), soundboard.load()]);
    },
    dispose() {
      stopMirroringAccount();
      stopFollowingHub();
      stopResyncing();
      encryption.dispose();
      friends.dispose?.();
      soundboard.dispose();
      messageActions.dispose?.();
      typing.dispose?.();
      chat.dispose?.();
      reactionStore.reset();
      readStore.reset();
      rtcConfig = undefined;
      store.reset();
      void realtime.stop();
    },
  };
}

let cached: Runtime | null = null;

/** One runtime per browser session. */
export function getRuntime(): Runtime {
  cached ??= createRuntime();
  return cached;
}

/** Tests build their own runtime; this clears the module-level cache. */
export function __resetRuntime(): void {
  cached = null;
}
