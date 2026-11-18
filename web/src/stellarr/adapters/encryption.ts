import type { EncryptionApiClient } from "@/lib/api/calls/encryption";
import { createE2eeClient, type E2eeClient } from "@/lib/e2ee/client";
import { createIndexedDbKeyStore, createMemoryKeyStore } from "@/lib/e2ee/store";
import { probeE2eeSupport } from "@/lib/e2ee/support";
import { describeUserAgent } from "@/lib/platform/userAgent";
import type { RealtimeClient } from "@/lib/realtime/types";
import type { EncryptionPort } from "@/ports";

/** "Chrome, macOS", in no language in particular: the server stores it as it is. */
function deviceLabel(): string {
  const agent = typeof navigator === "undefined" ? "" : navigator.userAgent;
  const { browser, system } = describeUserAgent(agent);
  return [browser, system].filter((part) => part !== undefined).join(", ") || "Browser";
}

/** Runs `task` while no other tab holds `name`, where the browser has Web Locks. */
function webLock<T>(name: string, task: () => Promise<T>): Promise<T> {
  const locks = typeof navigator === "undefined" ? undefined : navigator.locks;
  return locks === undefined ? task() : locks.request(name, task);
}

export interface EncryptionRuntime {
  client: E2eeClient;
  port: EncryptionPort;
  dispose(): void;
}

/**
 * The encryption client, wired to the hub, and the port the interface uses.
 * The friends adapter gets the same client to decode and send through.
 */
export function createEncryption(api: EncryptionApiClient, realtime: RealtimeClient): EncryptionRuntime {
  const client = createE2eeClient({
    api,
    store: typeof indexedDB === "undefined" ? createMemoryKeyStore() : createIndexedDbKeyStore(),
    probe: probeE2eeSupport,
    deviceLabel,
    withLock: webLock,
  });

  let lastStatus = realtime.getStatus();
  const subscriptions = [
    realtime.on("DirectEncryptionChanged", ({ conversationId, encryption }) =>
      client.encryptionChanged(conversationId, encryption),
    ),
    realtime.on("DirectEncryptionEpochCreated", ({ conversationId, epoch }) =>
      client.epochCreated(conversationId, epoch),
    ),
    realtime.on("DirectEncryptionKeysChanged", ({ conversationId }) => client.keysChanged(conversationId)),
    realtime.on("EncryptionAccountChanged", () => {
      void client.sync().catch(() => undefined);
    }),
    // Events are not replayed after an outage, so the account is read again.
    realtime.subscribeStatus(() => {
      const status = realtime.getStatus();
      const reconnected = status === "connected" && lastStatus !== "connected";
      lastStatus = status;
      if (reconnected) void client.sync().catch(() => undefined);
    }),
  ];

  const port: EncryptionPort = {
    getStatus: client.getStatus,
    subscribe: (listener) =>
      client.subscribe((event) => {
        if (event.type === "status") listener();
      }),
    setUp: (pin) => client.setUp(pin),
    unlock: (pin) => client.unlock(pin),
    prepareNewPin: (pin) => client.prepareNewPin(pin),
    changePin: (prepared, password) => client.changePin(prepared, password),
    prepareReset: (pin) => client.prepareReset(pin),
    reset: (prepared, password) => client.reset(prepared, password),
    listDevices: () => client.listDevices(),
    removeDevice: (deviceId) => client.removeDevice(deviceId),
    switchConversation: (conversationId, action) => client.switchConversation(conversationId, action),
    safetyCode: (conversationId) => client.safetyCode(conversationId),
    async verification(peerId) {
      const stored = await client.verification(peerId);
      return stored === null ? null : { code: stored.code, at: stored.at };
    },
    setVerified: (peerId, code) => client.setVerified(peerId, code),
    clearVerified: (peerId) => client.clearVerified(peerId),
    forget: () => client.forget(),
  };

  return {
    client,
    port,
    dispose() {
      for (const unsubscribe of subscriptions) unsubscribe();
    },
  };
}
