import type { EncryptionApiClient } from "@/lib/api/calls/encryption";
import { parseConversationEncryption } from "@/lib/api/encryptionParsers";
import { ApiError } from "@/lib/api/errors";
import type {
  Attachment,
  ConversationEncryption,
  ConversationKeys,
  EncryptionAccount,
  EncryptionEpoch,
  EncryptionSwitchAction,
  EpochEnvelope,
  KeyBackupInput,
  Message,
  RecipientKey,
  SealedReason,
} from "@/lib/api/types";
import type { UploadMediaMetadata } from "@/lib/media/probe";

import { decryptFile, encryptFile, OPAQUE_TYPE, shownType } from "./attachment";
import { concatBytes, fromBase64, sameBytes, toBase64, type Bytes } from "./bytes";
import { deriveAccountEpoch, newEpochKey, unwrapEpochKey, unwrapEpochMaterial, wrapEpochKey, type OpenedEpoch } from "./epoch";
import {
  exportPrivateKey,
  exportPublicKey,
  generateKeyPair,
  importPrivateKey,
  KEY_ALGORITHM,
  openLocally,
  privateKeyMatches,
  sealLocally,
} from "./keys";
import { decryptPayload, encryptPayload, type MessagePayload, type PayloadAttachment } from "./message";
import { derivePinKeys, isPin, openBackup, PIN_ITERATIONS, sealBackup, type SealedBackup } from "./pin";
import { decodeKeyBackup, encodeKeyBackup, normalizeRecoveryCode, recoveryCredential, type ArchivedKey } from "./recovery";
import { safetyCode } from "./safetyCode";
import type { KeyStore, StoredVerification } from "./store";
import type { E2eeSupport } from "./support";

/**
 * The browser side of the encryption protocol: keys, account key backup, devices,
 * epochs, and turning direct messages into plaintext and back.
 *
 * One instance per runtime. The friends adapter decodes every direct message
 * through it and sends through it while a conversation is encrypted; the
 * encryption port gives the interface the account-level actions. Nothing it
 * returns holds ciphertext: a message comes back as plaintext, as a sealed
 * placeholder, or untouched when it was never encrypted.
 */

export interface E2eeStatus {
  support: E2eeSupport | "checking";
  /** The account's public key on the server; null until encryption is set up. */
  accountKey: { keyId: string; publicKey: string } | null;
  /** The current key backup on the server, if present. */
  backup: { remainingAttempts: number } | null;
  /** This browser holds the account private key. */
  unlocked: boolean;
  /** This account still needs to move its history off the old PIN. */
  migrationRequired?: boolean;
  /** This session's registered device, once there is one. */
  deviceId: string | null;
  /** The account already holds keys in as many browsers as it may. */
  deviceLimit: boolean;
  /** Counts key changes, so a view derived from keys (the safety code) knows to look again. */
  revision: number;
}

export type E2eeEvent =
  | { type: "status" }
  /** Keys arrived or changed: a message that was sealed may open now. */
  | { type: "keys"; conversationId?: string }
  /** A message's files finished decrypting; decode it again to show them. */
  | { type: "attachments"; messageId: string }
  /** The conversation's switch moved; a call in it may have to follow. */
  | { type: "switch"; conversationId: string; state: ConversationEncryption["state"] };

export type E2eeFailure =
  | "unsupported"
  | "no-account-key"
  | "no-device"
  | "locked"
  | "encryption-off"
  | "recipient-key-missing"
  | "key-changed"
  | "gave-up";

export class E2eeError extends Error {
  readonly reason: E2eeFailure;
  constructor(reason: E2eeFailure) {
    super(`Encryption: ${reason}`);
    this.name = "E2eeError";
    this.reason = reason;
  }
}

export type UnlockResult =
  | { kind: "unlocked" }
  | { kind: "wrong-code"; remainingAttempts: number }
  /** The last incorrect attempt deleted the backup. */
  | { kind: "backup-deleted" }
  | { kind: "invalid-code" }
  | { kind: "backup-missing" }
  /** Refused before trying: too many attempts lately. No attempt was spent. */
  | { kind: "rate-limited" }
  /** The decrypted backup failed account key validation. */
  | { kind: "backup-unusable" };

/** One encrypted upload, ready to go into a message. */
export interface EncryptedUpload {
  /** The file's upload id and its poster's or preview's, all of which the message must list. */
  attachmentIds: string[];
  payload: PayloadAttachment;
}

export interface PreparedRecoveryCode {
  accountKeyId: string;
  migration?: { publicKey: string; pkcs8: Bytes; archived: ArchivedKey };
  backup: KeyBackupInput;
}

export interface PreparedReset extends PreparedRecoveryCode {
  publicKey: string;
  pkcs8: Bytes;
}

export interface SafetyCodeView {
  peerId: string;
  groups: string[];
}

export interface E2eeClientOptions {
  api: EncryptionApiClient;
  store: KeyStore;
  probe: () => Promise<E2eeSupport>;
  /** "Chrome, macOS": how this browser describes itself in the device list. */
  deviceLabel: () => string;
  pinIterations?: number;
  /** Runs `task` while no other tab of this browser holds the same lock. */
  withLock?: <T>(name: string, task: () => Promise<T>) => Promise<T>;
}

/** A link to a file that is decrypted only when it is opened. */
export const LAZY_ATTACHMENT_PREFIX = "e2ee:";

/** Pictures and recordings up to this size are decrypted as soon as they are shown. */
const EAGER_LIMIT = 25 * 1024 * 1024;
const MAX_ATTEMPTS = 4;
const NETWORK_RETRIES = 2;
const SEND_CODES = new Set(["epoch-not-current", "epoch-outdated"]);
const ROTATE_CODES = new Set(["epoch-stale", "epoch-coverage", "epoch-exists"]);

const codeOf = (error: unknown) => (error instanceof ApiError ? error.code : undefined);

/** Whether two recipient lists name the same keys. */
export function sameRecipients(left: readonly RecipientKey[], right: readonly RecipientKey[]): boolean {
  const a = new Set(left.map((key) => key.keyId));
  const b = new Set(right.map((key) => key.keyId));
  return a.size === b.size && [...a].every((id) => b.has(id));
}

function backupInput(backup: SealedBackup): KeyBackupInput {
  return {
    version: 2,
    salt: toBase64(backup.salt),
    iterations: backup.iterations,
    verifier: toBase64(backup.verifier),
    iv: toBase64(backup.iv),
    ciphertext: toBase64(backup.ciphertext),
  };
}

function withoutCiphertext(message: Message): Message {
  const { encrypted: _encrypted, ...rest } = message;
  return rest;
}

interface ConversationState {
  keys: ConversationKeys | null;
  keysStale: boolean;
  keysLoad: Promise<ConversationKeys> | null;
  epochs: Map<string, EncryptionEpoch>;
  /** The highest sequence the listing has read; only the listing moves it, so no epoch is skipped. */
  listedSequence: number;
  epochsLoad: Promise<void> | null;
  /** Epoch ids looked for and not found since the last change. */
  missing: Set<string>;
  legacyKeys: Map<string, OpenedEpoch>;
  accountEpochs: Map<string, {
    privateKey: CryptoKey;
    envelope: EpochEnvelope;
    peerId: string;
    peerPublicKey: Bytes;
    opened: OpenedEpoch;
  }>;
  /** Rotation is decided one send at a time, so two sends never race to create epochs. */
  queue: Promise<unknown>;
}

/** The epoch the next message or call frame goes out under. */
interface SendingEpoch {
  epochId: string;
  sequence: number;
  opened: OpenedEpoch;
}

interface LocalAccountKey {
  keyId: string;
  publicKey: Bytes;
  privateKey: CryptoKey;
}

interface LocalDevice {
  privateKey: CryptoKey;
  publicKey: Bytes;
  id: string | null;
}

interface LazyFile {
  url: string;
  key: string;
  iv: string;
  type: string;
}

export function createE2eeClient(options: E2eeClientOptions) {
  const { api, store } = options;
  const iterations = options.pinIterations ?? PIN_ITERATIONS;
  const withLock = options.withLock ?? (<T>(_name: string, task: () => Promise<T>) => task());
  const listeners = new Set<(event: E2eeEvent) => void>();

  let userId: string | null = null;
  let support: E2eeSupport | "checking" = "checking";
  let ready: Promise<void> = Promise.resolve();
  let account: EncryptionAccount = { accountKey: null, backup: null, device: null };
  let accountKey: LocalAccountKey | null = null;
  let archivedKey: LocalAccountKey | null = null;
  let device: LocalDevice | null = null;
  let deviceRefreshed = false;
  let deviceLimit = false;
  let revision = 0;
  let status: E2eeStatus = snapshot();

  const conversations = new Map<string, ConversationState>();
  /** Decrypted message plaintext by message id; plaintext never changes. */
  const payloads = new Map<string, MessagePayload>();
  /** Object URLs of decrypted files by upload id. */
  const objectUrls = new Map<string, string>();
  const decrypting = new Map<string, Promise<string | null>>();
  const failedFiles = new Set<string>();
  const lazyFiles = new Map<string, LazyFile>();

  function snapshot(): E2eeStatus {
    return {
      support,
      accountKey:
        account.accountKey === null
          ? null
          : { keyId: account.accountKey.keyId, publicKey: account.accountKey.publicKey },
      backup: account.backup === null ? null : { remainingAttempts: account.backup.remainingAttempts },
      migrationRequired: account.accountKey !== null && account.recoveryVersion === 1,
      unlocked: accountKey !== null && accountKey.keyId === account.accountKey?.keyId,
      deviceId: device?.id ?? null,
      deviceLimit,
      revision,
    };
  }

  const emit = (event: E2eeEvent) => {
    for (const listener of listeners) listener(event);
  };

  const publish = () => {
    status = snapshot();
    emit({ type: "status" });
  };

  const keysChanged = (conversationId?: string) => {
    revision += 1;
    status = snapshot();
    emit({ type: "status" });
    emit(conversationId === undefined ? { type: "keys" } : { type: "keys", conversationId });
  };

  const conversation = (id: string): ConversationState => {
    let state = conversations.get(id);
    if (state === undefined) {
      state = {
        keys: null,
        keysStale: true,
        keysLoad: null,
        epochs: new Map(),
        listedSequence: 0,
        epochsLoad: null,
        missing: new Set(),
        legacyKeys: new Map(),
        accountEpochs: new Map(),
        queue: Promise.resolve(),
      };
      conversations.set(id, state);
    }
    return state;
  };

  const requireUser = (): string => {
    if (userId === null) throw new E2eeError("no-account-key");
    return userId;
  };

  /* ------------------------------------------------------------ account */

  async function adoptAccountKey(user: string, keyId: string, publicKey: Bytes, pkcs8: Bytes, archived?: ArchivedKey): Promise<void> {
    const sealed = await sealLocally(pkcs8);
    const old = archived && { keyId: archived.keyId, publicKey: archived.publicKey, ...await sealLocally(archived.pkcs8) };
    await store.putAccountKey({ userId: user, keyId, publicKey, ...sealed, ...(old && { archived: old }) });
    archivedKey = archived ? { keyId: archived.keyId, publicKey: archived.publicKey, privateKey: await importPrivateKey(archived.pkcs8, false) } : null;
    accountKey = { keyId, publicKey, privateKey: await importPrivateKey(pkcs8, false) };
  }

  async function loadLocalAccountKey(user: string): Promise<void> {
    const stored = await store.getAccountKey(user);
    const server = account.accountKey;
    if (stored === null) {
      archivedKey = null;
      accountKey = null;
      return;
    }
    // A reset elsewhere left this copy behind: it opens nothing new.
    if (server === null || stored.keyId !== server.keyId) {
      await store.deleteAccountKey(user);
      archivedKey = null;
      accountKey = null;
      return;
    }
    if (accountKey?.keyId === stored.keyId) return;
    const old = stored.archived;
    archivedKey = old ? { keyId: old.keyId, publicKey: old.publicKey, privateKey: await importPrivateKey(await openLocally(old), false) } : null;
    accountKey = {
      keyId: stored.keyId,
      publicKey: stored.publicKey,
      privateKey: await importPrivateKey(await openLocally(stored), false),
    };
  }

  async function ensureDevice(user: string): Promise<void> {
    if (account.accountKey === null) return;
    await withLock(`stellarr-e2ee-device-${user}`, async () => {
      const local = await store.getDevice(user);
      const matches = (candidate: EncryptionAccount["device"]) =>
        local !== null && candidate !== null && sameBytes(local.publicKey, fromBase64(candidate.publicKey));
      // Another tab of this browser may have registered a key while this one
      // waited for the lock: the server's answer from before is read again.
      if (local !== null && !matches(account.device)) account = await api.getEncryptionAccount();
      const registered = account.device;
      if (local !== null && registered !== null && matches(registered)) {
        device = { privateKey: local.privateKey, publicKey: local.publicKey, id: registered.id };
        if (!deviceRefreshed) {
          deviceRefreshed = true;
          // Refreshes the label and the last-seen time; the same key is a 200.
          await api.registerDevice({ publicKey: toBase64(local.publicKey), label: options.deviceLabel() }).catch(() => undefined);
        }
        return;
      }
      // A new session, or a browser that lost its key: a new key, a new device.
      const pair = await generateKeyPair(false);
      const publicKey = await exportPublicKey(pair.publicKey);
      try {
        const created = await api.registerDevice({ publicKey: toBase64(publicKey), label: options.deviceLabel() });
        await store.putDevice({ userId: user, privateKey: pair.privateKey, publicKey });
        device = { privateKey: pair.privateKey, publicKey, id: created.id };
        account = { ...account, device: created };
        deviceRefreshed = true;
        deviceLimit = false;
      } catch (error) {
        const code = codeOf(error);
        if (code === "device-limit") {
          deviceLimit = true;
          return;
        }
        if (code === "account-key-missing") return;
        throw error;
      }
    });
  }

  /** Reads the account from the server and brings this browser in line with it. */
  async function sync(): Promise<void> {
    const user = userId;
    if (user === null || support !== "supported") return;
    account = await api.getEncryptionAccount();
    if (userId !== user) return;
    await loadLocalAccountKey(user);
    if (account.device === null) device = null;
    await ensureDevice(user);
    publish();
  }

  /* ------------------------------------------------------------ epochs */

  async function keysOf(conversationId: string, fresh = false): Promise<ConversationKeys> {
    const state = conversation(conversationId);
    if (!fresh && !state.keysStale && state.keys !== null) return state.keys;
    state.keysLoad ??= api
      .getConversationKeys(conversationId)
      .then(async (keys) => {
        const user = requireUser();
        if (keys.conversationId !== conversationId || keys.self.userId !== user || keys.peer.userId === user ||
            (support === "supported" && !await store.bindPeer(user, conversationId, keys.peer.userId))) {
          throw new E2eeError("key-changed");
        }
        if (keys.encryption.state === "on" && support === "supported") {
          await store.setPlaintextAllowed(user, keys.peer.userId, false);
        }
        state.keys = keys;
        state.keysStale = false;
        if (keys.currentEpoch !== null) remember(state, keys.currentEpoch);
        return keys;
      })
      .finally(() => {
        state.keysLoad = null;
      });
    return state.keysLoad;
  }

  function remember(state: ConversationState, epoch: EncryptionEpoch): void {
    const known = state.epochs.get(epoch.id);
    // The keys route and the event carry different envelopes; keep them all.
    const envelopes = new Map<string, EpochEnvelope>();
    for (const envelope of [...(known?.envelopes ?? []), ...epoch.envelopes]) {
      envelopes.set(envelope.recipientKeyId, envelope);
    }
    state.epochs.set(epoch.id, { ...epoch, envelopes: [...envelopes.values()] });
    state.missing.delete(epoch.id);
  }

  async function loadEpochs(conversationId: string): Promise<void> {
    const state = conversation(conversationId);
    state.epochsLoad ??= (async () => {
      let hasMore = true;
      while (hasMore) {
        const page = await api.listConversationEpochs(conversationId, state.listedSequence);
        for (const epoch of page.epochs) {
          remember(state, epoch);
          state.listedSequence = Math.max(state.listedSequence, epoch.sequence);
        }
        hasMore = page.hasMore && page.epochs.length > 0;
      }
    })().finally(() => {
      state.epochsLoad = null;
    });
    return state.epochsLoad;
  }

  async function epochFor(conversationId: string, epochId: string): Promise<EncryptionEpoch | null> {
    const state = conversation(conversationId);
    const known = state.epochs.get(epochId);
    if (known !== undefined) return known;
    if (state.missing.has(epochId)) return null;
    try {
      await loadEpochs(conversationId);
    } catch {
      return null;
    }
    const found = state.epochs.get(epochId) ?? null;
    if (found === null) state.missing.add(epochId);
    return found;
  }

  function accountPair(keys: ConversationKeys) {
    const own = accountKey;
    if (own === null) throw new E2eeError("locked");
    const peer = keys.peer.accountKey;
    if (!peer || peer.algorithm !== KEY_ALGORITHM) throw new E2eeError("recipient-key-missing");
    if (keys.self.accountKey?.keyId !== own.keyId || keys.self.accountKey.publicKey !== toBase64(own.publicKey) ||
        peer.keyId === own.keyId || peer.publicKey === toBase64(own.publicKey)) throw new E2eeError("key-changed");
    return {
      self: { ...own, userId: requireUser() },
      peer: { keyId: peer.keyId, publicKey: fromBase64(peer.publicKey), userId: keys.peer.userId },
    };
  }

  type AccountPair = ReturnType<typeof accountPair>;

  async function checkVerification(self: AccountPair["self"], peer: AccountPair["peer"], historical = false) {
    const verified = await store.getVerification(self.userId, peer.userId);
    if (verified === null) return;
    const code = (await safetyCode(self, peer)).join(" ");
    if (code === verified.code || (historical && verified.previousCodes?.includes(code))) return;
    // The peer can be unchanged even though our own half of the fingerprint rotated.
    if (historical && self.keyId === archivedKey?.keyId && accountKey) {
      const active = { ...accountKey, userId: self.userId };
      const codeWithActiveKey = (await safetyCode(active, peer)).join(" ");
      if (codeWithActiveKey === verified.code || verified.previousCodes?.includes(codeWithActiveKey)) return;
    }
    throw new E2eeError("key-changed");
  }

  async function openAccountEpoch(conversationId: string, epoch: EncryptionEpoch, current?: AccountPair) {
    if (epoch.version !== 2 || accountKey === null) return null;
    const user = requireUser();
    const accounts = epoch.recipients.filter((r) => r.kind === "account");
    // The retired PIN-era key is available exclusively to the history reader.
    const own = current === undefined && archivedKey && accounts.some((r) => r.userId === user && r.keyId === archivedKey?.keyId)
      ? archivedKey : accountKey;
    const self = { ...own, userId: user };
    const other = accounts.find((r) => r.userId !== self.userId);
    const wire = epoch.envelopes.find((e) => e.recipientKeyId === self.keyId);
    if (accounts.length !== 2 || !other || !wire ||
        !accounts.some((r) => r.userId === self.userId && r.keyId === self.keyId)) return null;
    try {
      const cache = conversation(conversationId).accountEpochs;
      let entry = cache.get(epoch.id);
      if (!entry || entry.privateKey !== self.privateKey || entry.peerId !== other.userId ||
          entry.envelope.ciphertext !== wire.ciphertext || entry.envelope.iv !== wire.iv ||
          entry.envelope.ephemeralPublicKey !== wire.ephemeralPublicKey) {
        const material = await unwrapEpochMaterial({
          recipientKeyId: self.keyId, ephemeralPublicKey: fromBase64(wire.ephemeralPublicKey),
          iv: fromBase64(wire.iv), ciphertext: fromBase64(wire.ciphertext),
        }, self.privateKey, { conversationId, epochId: epoch.id });
        if (material.length !== 64) return null;
        const peer = { userId: other.userId, publicKey: material.slice(32) };
        const opened = await deriveAccountEpoch(material.slice(0, 32), self, peer, conversationId, epoch.id);
        entry = { privateKey: self.privateKey, envelope: wire, peerId: other.userId, peerPublicKey: peer.publicKey, opened };
        cache.set(epoch.id, entry);
      }
      const peer = { ...other, publicKey: entry.peerPublicKey };
      if (current !== undefined) {
        if (peer.userId !== current.peer.userId || peer.keyId !== current.peer.keyId ||
            !sameBytes(peer.publicKey, current.peer.publicKey)) return null;
      } else {
        const bound = await store.getPeer(self.userId, conversationId) ?? (await keysOf(conversationId)).peer.userId;
        if (bound !== peer.userId) return null;
        await checkVerification(self, peer, true);
      }
      return entry.opened;
    } catch {
      return null;
    }
  }

  async function openEpoch(conversationId: string, epoch: EncryptionEpoch): Promise<OpenedEpoch | null> {
    const state = conversation(conversationId);
    if (epoch.version === 2) return openAccountEpoch(conversationId, epoch);
    const opened = state.legacyKeys.get(epoch.id);
    if (opened !== undefined) return opened;
    const context = { conversationId, epochId: epoch.id };
    const attempts: [EpochEnvelope | undefined, CryptoKey | undefined][] = [
      [epoch.envelopes.find((envelope) => envelope.recipientKeyId === device?.id), device?.privateKey],
      [epoch.envelopes.find((envelope) => envelope.recipientKeyId === accountKey?.keyId), accountKey?.privateKey],
      [epoch.envelopes.find((envelope) => envelope.recipientKeyId === archivedKey?.keyId), archivedKey?.privateKey],
    ];
    for (const [envelope, privateKey] of attempts) {
      if (envelope === undefined || privateKey === undefined) continue;
      try {
        const key: OpenedEpoch = await unwrapEpochKey(
          {
            recipientKeyId: envelope.recipientKeyId,
            ephemeralPublicKey: fromBase64(envelope.ephemeralPublicKey),
            iv: fromBase64(envelope.iv),
            ciphertext: fromBase64(envelope.ciphertext),
          },
          privateKey,
          context,
        );
        state.legacyKeys.set(epoch.id, key);
        return key;
      } catch {
        // Not for this key, or changed on the way; try the other.
      }
    }
    return null;
  }

  function sealedReason(epoch: EncryptionEpoch | null): SealedReason {
    if (support !== "supported") return "unsupported";
    const current = account.accountKey?.keyId;
    if (epoch !== null && current !== undefined && accountKey?.keyId !== current) {
      const forAccount =
        epoch.recipients.some((recipient) => recipient.keyId === current) ||
        epoch.envelopes.some((e) => epoch.recipients.some((r) =>
          r.keyId === e.recipientKeyId && r.kind === "account" && r.userId === userId));
      if (forAccount) return "pin";
    }
    return "lost";
  }

  async function createEpoch(conversationId: string, keys: ConversationKeys, pair: AccountPair) {
    const epochId = crypto.randomUUID();
    const seed = newEpochKey();
    const { self, peer } = pair;
    // Construct exactly two envelopes from the account keys used for the safety code.
    const envelopes = await Promise.all([self, peer].map(async (recipient) => {
      const other = recipient === self ? peer : self;
      const envelope = await wrapEpochKey(concatBytes(seed, other.publicKey), recipient.publicKey, {
        conversationId, epochId, recipientKeyId: recipient.keyId,
      });
      return {
        recipientKeyId: envelope.recipientKeyId,
        ephemeralPublicKey: toBase64(envelope.ephemeralPublicKey),
        iv: toBase64(envelope.iv),
        ciphertext: toBase64(envelope.ciphertext),
      };
    }));
    const opened = await deriveAccountEpoch(seed, self, peer, conversationId, epochId);
    const epoch = await api.createConversationEpoch(conversationId, {
      version: 2, epochId, previousEpochId: keys.encryption.currentEpochId, envelopes,
      deviceKeyIds: keys.recipients.filter((r) => r.kind === "device").map((r) => r.keyId),
    });
    if (epoch.id !== epochId || epoch.version !== 2) throw new E2eeError("key-changed");
    remember(conversation(conversationId), epoch);
    keys.currentEpoch = epoch;
    keys.encryption = { ...keys.encryption, currentEpochId: epoch.id };
    return { epochId: epoch.id, sequence: epoch.sequence, opened };
  }

  /**
   * The epoch the next message goes out with, rotating first when the
   * conversation has none since it was switched on, when this browser cannot
   * open the current one, or when the keys it was wrapped for are no longer
   * exactly today's.
   */
  async function decideEpoch(conversationId: string): Promise<SendingEpoch> {
    for (let attempt = 0; attempt < MAX_ATTEMPTS; attempt += 1) {
      const keys = await keysOf(conversationId, attempt > 0);
      if (keys.encryption.state !== "on") {
        // The cached state may predate the switch; the server's word decides.
        if (attempt === 0) continue;
        throw new E2eeError("encryption-off");
      }
      const pair = accountPair(keys);
      await checkVerification(pair.self, pair.peer);
      const current = keys.currentEpoch;
      if (
        current !== null &&
        current.id === keys.encryption.currentEpochId &&
        sameRecipients(current.recipients, keys.recipients)
      ) {
        const opened = await openAccountEpoch(conversationId, current, pair);
        if (opened !== null) return { epochId: current.id, sequence: current.sequence, opened };
      }
      try {
        return await createEpoch(conversationId, keys, pair);
      } catch (error) {
        const code = codeOf(error);
        if (code !== undefined && ROTATE_CODES.has(code)) {
          conversation(conversationId).keysStale = true;
          continue;
        }
        if (code === "encryption-off") throw new E2eeError("encryption-off");
        throw error;
      }
    }
    throw new E2eeError("gave-up");
  }

  function epochForSending(conversationId: string): Promise<SendingEpoch> {
    const state = conversation(conversationId);
    const run = state.queue.then(
      () => decideEpoch(conversationId),
      () => decideEpoch(conversationId),
    );
    state.queue = run.catch(() => undefined);
    return run;
  }

  /* ------------------------------------------------------------ files */

  async function decryptToUrl(uploadId: string, file: LazyFile, shownAs: string): Promise<string | null> {
    const existing = objectUrls.get(uploadId);
    if (existing !== undefined) return existing;
    let running = decrypting.get(uploadId);
    if (running === undefined) {
      running = (async () => {
        try {
          const data = await api.downloadAsset(file.url);
          const plain = await decryptFile(data, file.key, file.iv);
          const url = URL.createObjectURL(new Blob([plain], { type: shownAs }));
          objectUrls.set(uploadId, url);
          return url;
        } catch {
          failedFiles.add(uploadId);
          return null;
        } finally {
          decrypting.delete(uploadId);
        }
      })();
      decrypting.set(uploadId, running);
    }
    return running;
  }

  function decryptSoon(uploadId: string, file: LazyFile, shownAs: string, messageId: string): void {
    if (objectUrls.has(uploadId) || failedFiles.has(uploadId) || decrypting.has(uploadId)) return;
    void decryptToUrl(uploadId, file, shownAs).then(() => emit({ type: "attachments", messageId }));
  }

  /** The object URL of a sealed picture that travels beside a file; decryption starts when it is not ready. */
  function pictureUrl(picture: PayloadAttachment["poster"], messageId: string): string | undefined {
    if (picture === undefined || !shownType(picture.type).startsWith("image/")) return undefined;
    const url = objectUrls.get(picture.id);
    if (url === undefined) {
      decryptSoon(picture.id, { url: `/assets/${picture.id}`, key: picture.key, iv: picture.iv, type: picture.type }, shownType(picture.type), messageId);
    }
    return url;
  }

  function attachmentView(item: PayloadAttachment, stored: Attachment | undefined, messageId: string): Attachment {
    const type = shownType(item.type);
    const base = {
      id: item.id,
      name: item.name,
      size: item.size,
      ...(item.width === undefined ? {} : { width: item.width }),
      ...(item.height === undefined ? {} : { height: item.height }),
      ...(item.duration === undefined ? {} : { durationSeconds: item.duration }),
      ...(item.voice === true ? { isVoiceMessage: true } : {}),
      ...(item.placeholder === undefined ? {} : { placeholder: item.placeholder }),
    };
    const serverUrl = stored?.url ?? `/assets/${item.id}`;
    lazyFiles.set(item.id, { url: serverUrl, key: item.key, iv: item.iv, type });
    // An image that has a preview is shown from the preview alone: its original
    // is fetched and decrypted only when it is opened, saved or copied.
    if (type.startsWith("image/") && item.preview !== undefined && shownType(item.preview.type).startsWith("image/")) {
      const previewUrl = pictureUrl(item.preview, messageId);
      // Until the preview is ready (or if it fails) it is a file card without a link.
      if (previewUrl === undefined) return { ...base, contentType: OPAQUE_TYPE, url: "" };
      return { ...base, contentType: type, url: objectUrls.get(item.id) ?? `${LAZY_ATTACHMENT_PREFIX}${item.id}`, previewUrl };
    }
    const eager = (type.startsWith("image/") || type.startsWith("audio/")) && item.size <= EAGER_LIMIT;
    if (eager) {
      const url = objectUrls.get(item.id);
      if (url !== undefined) return { ...base, contentType: type, url };
      decryptSoon(item.id, { url: serverUrl, key: item.key, iv: item.iv, type }, type, messageId);
      // Until it is ready (or if it fails) it is a file card without a link.
      return { ...base, contentType: OPAQUE_TYPE, url: "" };
    }
    if (type.startsWith("audio/")) {
      return { ...base, contentType: type, url: objectUrls.get(item.id) ?? `${LAZY_ATTACHMENT_PREFIX}${item.id}` };
    }
    if (type.startsWith("video/")) {
      const posterUrl = pictureUrl(item.poster, messageId);
      return {
        ...base,
        contentType: type,
        url: `${LAZY_ATTACHMENT_PREFIX}${item.id}`,
        ...(posterUrl === undefined ? {} : { posterUrl }),
      };
    }
    return { ...base, contentType: OPAQUE_TYPE, url: `${LAZY_ATTACHMENT_PREFIX}${item.id}` };
  }

  function present(message: Message, payload: MessagePayload): Message {
    const stored = new Map(message.attachments.map((attachment) => [attachment.id, attachment]));
    return {
      ...withoutCiphertext(message),
      decrypted: true,
      content: payload.text,
      attachments: payload.attachments.map((item) => attachmentView(item, stored.get(item.id), message.id)),
    };
  }

  const sealed = (message: Message, reason: SealedReason): Message => ({
    ...withoutCiphertext(message),
    content: "",
    attachments: [],
    embeds: [],
    sealed: { reason, count: 1 },
  });

  function encryptionChanged(conversationId: string, encryption: ConversationEncryption): void {
    const state = conversation(conversationId);
    if (state.keys !== null) state.keys = { ...state.keys, encryption };
    state.keysStale = true;
    emit({ type: "switch", conversationId, state: encryption.state });
  }

  async function plaintextAllowed(conversationId: string, peerId: string): Promise<boolean> {
    await ready;
    const self = requireUser();
    const bound = await store.getPeer(self, conversationId);
    if (bound !== null && bound !== peerId) return false;
    const verified = await store.getVerification(self, peerId);
    if (verified === null) return true;
    if (bound === null && !await store.bindPeer(self, conversationId, peerId)) return false;
    return verified.plaintextAllowed === true;
  }

  async function callKeys(conversationId: string): Promise<CallKeys | null> {
    await ready;
    const keys = await keysOf(conversationId, true);
    if (keys.encryption.state !== "on") {
      if (!await plaintextAllowed(conversationId, keys.peer.userId)) throw new E2eeError("encryption-off");
      return null;
    }
    return sendingCallKeys(conversationId);
  }

  async function sendingCallKeys(conversationId: string): Promise<CallKeys> {
    const user = requireUser();
    if (support !== "supported") throw new E2eeError("unsupported");
    if (device?.id === undefined || device.id === null) await sync();
    if (device?.id === undefined || device.id === null) throw new E2eeError("no-device");
    const sending = await epochForSending(conversationId);
    return {
      self: user,
      send: { sequence: sending.sequence, epochId: sending.epochId, base: sending.opened.callBase },
    };
  }

  /* ------------------------------------------------------------ the client */

  return {
    plaintextAllowed,
    getStatus: (): E2eeStatus => status,

    subscribe(listener: (event: E2eeEvent) => void): () => void {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },

    /** After sign-in: probes, reads the account and registers this browser. */
    start(user: string): Promise<void> {
      if (userId === user) return ready;
      userId = user;
      ready = (async () => {
        support = await options.probe();
        publish();
        if (support !== "supported") return;
        try {
          await sync();
        } catch {
          // Offline or refused: the next account event or send tries again.
        }
      })();
      return ready;
    },

    ready: () => ready,

    async sync(): Promise<void> {
      await ready;
      await sync();
    },

    /** Sign-out: this browser's device key and account key copy go. */
    async forget(): Promise<void> {
      const user = userId;
      userId = null;
      archivedKey = null;
      accountKey = null;
      device = null;
      deviceRefreshed = false;
      account = { accountKey: null, backup: null, device: null };
      conversations.clear();
      payloads.clear();
      lazyFiles.clear();
      failedFiles.clear();
      for (const url of objectUrls.values()) URL.revokeObjectURL(url);
      objectUrls.clear();
      publish();
      if (user !== null && support === "supported") await store.forget(user);
    },

    /* ---------------- Account key recovery */

    /** Creates the account key, its recovery-code backup and the first device. */
    async setUp(code: string): Promise<void> {
      await ready;
      const user = requireUser();
      if (support !== "supported" || account.recoveryVersion === undefined) throw new E2eeError("unsupported");
      const pair = await generateKeyPair(true);
      const pkcs8 = await exportPrivateKey(pair.privateKey);
      const publicKey = await exportPublicKey(pair.publicKey);
      const backup = await sealBackup(encodeKeyBackup(pkcs8), recoveryCredential(code), iterations);
      try {
        account = await api.setUpAccountKey({ publicKey: toBase64(publicKey), backup: backupInput(backup) });
      } catch (error) {
        // A different tab registered its own key and recovery code.
        if (codeOf(error) === "account-key-exists") {
          await sync();
          throw new E2eeError("key-changed");
        }
        throw error;
      }
      if (account.accountKey !== null) {
        await adoptAccountKey(user, account.accountKey.keyId, publicKey, pkcs8);
      }
      await ensureDevice(user);
      publish();
    },

    /** Unlocks a current backup or reads a legacy backup for migration. */
    async unlock(input: string): Promise<UnlockResult> {
      await ready;
      const user = requireUser();
      const server = account.accountKey;
      if (server === null) return { kind: "backup-missing" };
      let parameters;
      try {
        parameters = await api.getBackupParameters();
      } catch (error) {
        if (codeOf(error) === "backup-missing") return { kind: "backup-missing" };
        if (error instanceof ApiError && error.kind === "rate-limited") return { kind: "rate-limited" };
        throw error;
      }
      if (parameters.version !== account.recoveryVersion) return { kind: "backup-unusable" };
      const legacy = parameters.version === 1;
      if (legacy ? !isPin(input) : normalizeRecoveryCode(input) === null) return { kind: "invalid-code" };
      const credential = legacy ? input : recoveryCredential(input);
      const { authKey, encKey } = await derivePinKeys(credential, fromBase64(parameters.salt), parameters.iterations);
      let unlocked;
      try {
        unlocked = await api.unlockBackup(toBase64(authKey), parameters.version);
      } catch (error) {
        const code = codeOf(error);
        const details = error instanceof ApiError ? error.details : undefined;
        if (code === "pin-incorrect") {
          const remaining = typeof details?.["remainingAttempts"] === "number" ? details["remainingAttempts"] : 0;
          account = { ...account, backup: account.backup === null ? null : { ...account.backup, remainingAttempts: remaining } };
          publish();
          return { kind: "wrong-code", remainingAttempts: remaining };
        }
        if (code === "backup-deleted") {
          account = { ...account, backup: null };
          publish();
          return { kind: "backup-deleted" };
        }
        if (code === "backup-missing") return { kind: "backup-missing" };
        if (error instanceof ApiError && error.kind === "rate-limited") return { kind: "rate-limited" };
        throw error;
      }
      let pkcs8: Bytes;
      let archived: ArchivedKey | undefined;
      try {
        const payload = await openBackup(encKey, fromBase64(unlocked.iv), fromBase64(unlocked.ciphertext));
        ({ pkcs8, archived } = legacy ? { pkcs8: payload } : await decodeKeyBackup(payload));
      } catch {
        return { kind: "backup-unusable" };
      }
      const publicKey = fromBase64(server.publicKey);
      // Bind the decrypted private key to the active account identity.
      if (unlocked.accountKeyId !== server.keyId || !(await privateKeyMatches(pkcs8, publicKey))) {
        return { kind: "backup-unusable" };
      }
      await adoptAccountKey(user, server.keyId, publicKey, pkcs8, archived);
      if (account.backup !== null) {
        account = { ...account, backup: { ...account.backup, failedAttempts: 0, remainingAttempts: account.backup.failedAttempts + account.backup.remainingAttempts } };
      }
      keysChanged();
      return { kind: "unlocked" };
    },

    /** Seals the backup under the new code. A PIN-era account also gets a fresh identity. */
    async prepareRecoveryCode(code: string): Promise<PreparedRecoveryCode> {
      const user = requireUser();
      const stored = await store.getAccountKey(user);
      if (stored === null || stored.keyId !== account.accountKey?.keyId) throw new E2eeError("locked");
      if (account.recoveryVersion === undefined) throw new E2eeError("unsupported");
      const credential = recoveryCredential(code);
      const current = await openLocally(stored);
      if (account.recoveryVersion !== 2) {
        const pair = await generateKeyPair(true);
        const pkcs8 = await exportPrivateKey(pair.privateKey);
        const archived = { keyId: stored.keyId, publicKey: stored.publicKey, pkcs8: current };
        return {
          accountKeyId: stored.keyId,
          migration: { pkcs8, publicKey: toBase64(await exportPublicKey(pair.publicKey)), archived },
          backup: backupInput(await sealBackup(encodeKeyBackup(pkcs8, archived), credential, iterations)),
        };
      }
      const old = stored.archived;
      const archived = old && { keyId: old.keyId, publicKey: old.publicKey, pkcs8: await openLocally(old) };
      return { accountKeyId: stored.keyId, backup: backupInput(await sealBackup(encodeKeyBackup(current, archived), credential, iterations)) };
    },

    /** Stores the new backup. Throws the server's refusals (password, second step) as they are. */
    async changeRecoveryCode(prepared: PreparedRecoveryCode, currentPassword: string): Promise<void> {
      const user = requireUser();
      const keyId = account.accountKey?.keyId;
      if (keyId === undefined || accountKey?.keyId !== keyId) throw new E2eeError("locked");
      if (prepared.accountKeyId !== keyId) throw new E2eeError("key-changed");
      const migration = prepared.migration;
      if (migration) {
        const answer = await api.resetAccountKey({
          migrateFromKeyId: keyId, currentPassword, publicKey: migration.publicKey, backup: prepared.backup,
        });
        account = { ...account, recoveryVersion: 2, accountKey: answer.accountKey, backup: answer.backup };
        await adoptAccountKey(user, answer.accountKey.keyId, fromBase64(migration.publicKey), migration.pkcs8, migration.archived);
        for (const state of conversations.values()) state.keysStale = true;
        await sync().catch(() => undefined);
        keysChanged();
        return;
      }
      const replace = () => api.replaceBackup({ currentPassword, accountKeyId: keyId, backup: prepared.backup });
      try {
        await replace();
      } catch (error) {
        const code = codeOf(error);
        if (code === "account-key-changed") {
          await sync().catch(() => undefined);
          throw new E2eeError("key-changed");
        }
        if (code !== "device-missing") throw error;
        account = await api.getEncryptionAccount();
        await ensureDevice(user);
        await replace();
      }
      account = await api.getEncryptionAccount();
      publish();
    },

    /** Starts a fresh identity if the owner has lost all access to the old keys. */
    async prepareReset(code: string): Promise<PreparedReset> {
      const keyId = account.accountKey?.keyId;
      if (keyId === undefined) throw new E2eeError("no-account-key");
      if (account.recoveryVersion === undefined) throw new E2eeError("unsupported");
      const pair = await generateKeyPair(true);
      const pkcs8 = await exportPrivateKey(pair.privateKey);
      return {
        accountKeyId: keyId,
        publicKey: toBase64(await exportPublicKey(pair.publicKey)),
        pkcs8,
        backup: backupInput(await sealBackup(encodeKeyBackup(pkcs8), recoveryCredential(code), iterations)),
      };
    },

    async reset(prepared: PreparedReset, currentPassword: string): Promise<void> {
      const user = requireUser();
      const answer = await api.resetAccountKey({
        currentPassword,
        publicKey: prepared.publicKey,
        expectedAccountKeyId: prepared.accountKeyId,
        backup: prepared.backup,
      });
      account = { ...account, recoveryVersion: 2, accountKey: answer.accountKey, backup: answer.backup };
      await adoptAccountKey(user, answer.accountKey.keyId, fromBase64(prepared.publicKey), prepared.pkcs8);
      for (const state of conversations.values()) state.keysStale = true;
      await sync().catch(() => undefined);
      keysChanged();
    },

    listDevices: () => api.listEncryptionDevices(),

    async removeDevice(deviceId: string): Promise<void> {
      await api.removeEncryptionDevice(deviceId);
      for (const state of conversations.values()) state.keysStale = true;
    },

    /* ---------------- conversations */

    /** Moves the switch; a refusal because the state moved first answers with that state. */
    async switchConversation(conversationId: string, action: EncryptionSwitchAction): Promise<ConversationEncryption> {
      let encryption: ConversationEncryption;
      try {
        encryption = await api.switchConversationEncryption(conversationId, action);
      } catch (error) {
        const details = error instanceof ApiError ? error.details : undefined;
        if (codeOf(error) !== "encryption-state" || details === undefined) throw error;
        encryption = parseConversationEncryption(details["encryption"], "encryption-state");
      }
      const self = requireUser();
      const peer = await store.getPeer(self, conversationId);
      if (peer !== null) await store.setPlaintextAllowed(self, peer, action === "disable" && encryption.state === "off");
      encryptionChanged(conversationId, encryption);
      keysChanged(conversationId);
      return encryption;
    },

    encryptionChanged,

    epochCreated(conversationId: string, epoch: EncryptionEpoch): void {
      const state = conversation(conversationId);
      remember(state, epoch);
      if (state.keys !== null && (state.keys.currentEpoch === null || state.keys.currentEpoch.sequence < epoch.sequence)) {
        state.keys = {
          ...state.keys,
          currentEpoch: epoch,
          encryption: { ...state.keys.encryption, currentEpochId: epoch.id },
        };
      }
      emit({ type: "keys", conversationId });
    },

    keysChanged(conversationId: string): void {
      const state = conversation(conversationId);
      state.keysStale = true;
      state.missing.clear();
      keysChanged(conversationId);
    },

    /**
     * Plaintext for an encrypted direct message, a sealed placeholder when this
     * browser cannot read it, and anything else as it came.
     */
    async decode(message: Message): Promise<Message> {
      if (message.line !== undefined || message.encrypted === undefined) return message;
      await ready;
      const known = payloads.get(message.id);
      if (known !== undefined) return present(message, known);
      if (support !== "supported") return sealed(message, "unsupported");
      const { epochId, iv, ciphertext } = message.encrypted;
      const conversationId = message.channelId;
      const epoch = await epochFor(conversationId, epochId);
      const opened = epoch === null ? null : await openEpoch(conversationId, epoch);
      if (opened === null) return sealed(message, sealedReason(epoch));
      let payload: MessagePayload;
      try {
        payload = await decryptPayload(
          opened.key,
          { conversationId, epochId, authorId: message.author.id },
          fromBase64(iv),
          fromBase64(ciphertext),
        );
      } catch {
        return sealed(message, "lost");
      }
      // Every file the plaintext names must be one the message itself lists.
      const listed = new Set(message.attachments.map((attachment) => attachment.id));
      const named = payload.attachments.flatMap((item) => [item.id, ...(item.poster ? [item.poster.id] : []), ...(item.preview ? [item.preview.id] : [])]);
      if (named.some((id) => !listed.has(id))) return sealed(message, "lost");
      payloads.set(message.id, payload);
      return present(message, payload);
    },

    /** Encrypts and uploads one file (and its video poster or image preview) for an encrypted conversation. */
    async uploadFile(file: File, metadata: UploadMediaMetadata): Promise<EncryptedUpload> {
      const sealedFile = await encryptFile(file);
      const uploaded = await api.uploadEncryptedFile(sealedFile.data);
      const type = file.type === "" ? OPAQUE_TYPE : file.type;
      const sealPicture = async (picture: Blob): Promise<NonNullable<PayloadAttachment["poster"]>> => {
        const sealed = await encryptFile(picture);
        const upload = await api.uploadEncryptedFile(sealed.data);
        objectUrls.set(upload.id, URL.createObjectURL(picture));
        return { id: upload.id, key: sealed.key, iv: sealed.iv, type: picture.type || "image/jpeg", size: picture.size };
      };
      const poster = metadata.poster === undefined ? undefined : await sealPicture(metadata.poster);
      const preview = metadata.preview === undefined ? undefined : await sealPicture(metadata.preview);
      const shown = shownType(type);
      if (shown.startsWith("image/") || shown.startsWith("audio/")) {
        // The sender already has the bytes; nothing to download back.
        objectUrls.set(uploaded.id, URL.createObjectURL(new Blob([file], { type: shown })));
      }
      return {
        attachmentIds: [uploaded.id, ...(poster ? [poster.id] : []), ...(preview ? [preview.id] : [])],
        payload: {
          id: uploaded.id,
          key: sealedFile.key,
          iv: sealedFile.iv,
          name: file.name,
          type,
          size: file.size,
          ...(metadata.width === undefined ? {} : { width: metadata.width }),
          ...(metadata.height === undefined ? {} : { height: metadata.height }),
          ...(metadata.durationSeconds === undefined ? {} : { duration: metadata.durationSeconds }),
          ...(metadata.voiceMessage === true ? { voice: true } : {}),
          ...(poster === undefined ? {} : { poster }),
          ...(preview === undefined ? {} : { preview }),
          ...(metadata.placeholder === undefined ? {} : { placeholder: metadata.placeholder }),
        },
      };
    },

    /**
     * Encrypts and sends one message, rotating the epoch when it has to and
     * trying again after the server's epoch refusals, with the same client
     * message id throughout so a retry is never a second message.
     */
    async send(
      conversationId: string,
      input: { peerId?: string; text: string; uploads: readonly EncryptedUpload[]; clientMessageId: string },
    ): Promise<Message> {
      await ready;
      const user = requireUser();
      if (support !== "supported") throw new E2eeError("unsupported");
      if (device?.id === undefined || device.id === null) await sync();
      if (device?.id === undefined || device.id === null) throw new E2eeError("no-device");
      if (input.peerId !== undefined && !await store.bindPeer(user, conversationId, input.peerId)) {
        throw new E2eeError("key-changed");
      }
      const payload: MessagePayload = { v: 1, text: input.text, attachments: input.uploads.map((upload) => upload.payload) };
      const attachmentIds = input.uploads.flatMap((upload) => upload.attachmentIds);
      let networkRetries = 0;
      for (let attempt = 0; attempt < MAX_ATTEMPTS; attempt += 1) {
        const { epochId, opened } = await epochForSending(conversationId);
        const body = await encryptPayload(opened.key, { conversationId, epochId, authorId: user }, payload);
        try {
          const message = await api.sendEncryptedDirectMessage(conversationId, {
            clientMessageId: input.clientMessageId,
            attachmentIds,
            encrypted: { epochId, iv: toBase64(body.iv), ciphertext: toBase64(body.ciphertext) },
          });
          payloads.set(message.id, payload);
          return present(message, payload);
        } catch (error) {
          const code = codeOf(error);
          if (code !== undefined && SEND_CODES.has(code)) {
            conversation(conversationId).keysStale = true;
            continue;
          }
          if (error instanceof ApiError && error.kind === "network" && networkRetries < NETWORK_RETRIES) {
            networkRetries += 1;
            attempt -= 1;
            continue;
          }
          if (code === "encryption-off") {
            conversation(conversationId).keysStale = true;
            throw new E2eeError("encryption-off");
          }
          throw error;
        }
      }
      throw new E2eeError("gave-up");
    },

    /** An object URL for a file shown as a link until opened; null when it cannot be read. */
    async openAttachment(uploadId: string): Promise<{ url: string; type: string } | null> {
      const file = lazyFiles.get(uploadId);
      if (file === undefined) return null;
      const url = await decryptToUrl(uploadId, file, file.type);
      return url === null ? null : { url, type: file.type };
    },

    /* ---------------- calls */

    callKeys,

    /**
     * The keys for joining an encrypted call already under way, whatever the
     * switch says now: the conversation's own sending epoch while it is
     * encrypted, otherwise the newest epoch this browser can open. Throws when
     * there is none.
     */
    async followCall(conversationId: string): Promise<CallKeys> {
      await ready;
      const keys = await keysOf(conversationId, true);
      if (keys.encryption.state === "on") return sendingCallKeys(conversationId);
      if (support !== "supported") throw new E2eeError("unsupported");
      const pair = accountPair(keys);
      await checkVerification(pair.self, pair.peer);
      await loadEpochs(conversationId);
      const newestFirst = [...conversation(conversationId).epochs.values()].sort((a, b) => b.sequence - a.sequence);
      for (const epoch of newestFirst) {
        const opened = await openAccountEpoch(conversationId, epoch, pair);
        if (opened !== null) {
          return { self: requireUser(), send: { sequence: epoch.sequence, epochId: epoch.id, base: opened.callBase } };
        }
      }
      throw new E2eeError("gave-up");
    },

    /**
     * The epoch a call frame names by the low 16 bits of its sequence, loaded
     * when this browser has not seen it yet. Null when it cannot be had.
     */
    async callEpoch(conversationId: string, sequence: number): Promise<CallEpoch | null> {
      const state = conversation(conversationId);
      const find = () =>
        [...state.epochs.values()]
          .filter((epoch) => (epoch.sequence & 0xffff) === (sequence & 0xffff))
          .sort((a, b) => b.sequence - a.sequence)[0];
      let found = find();
      if (found === undefined) {
        try {
          await loadEpochs(conversationId);
        } catch {
          return null;
        }
        found = find();
      }
      if (found === undefined) return null;
      if (accountKey === null || found.version !== 2) return null;
      const pair = accountPair(await keysOf(conversationId));
      await checkVerification(pair.self, pair.peer);
      const opened = await openAccountEpoch(conversationId, found, pair);
      return opened === null ? null : { sequence: found.sequence, epochId: found.id, base: opened.callBase };
    },

    /* ---------------- safety code */

    async safetyCode(conversationId: string): Promise<SafetyCodeView | null> {
      const keys = await keysOf(conversationId, conversation(conversationId).keysStale);
      if (accountKey === null || keys.peer.accountKey === null) return null;
      const { self, peer } = accountPair(keys);
      const groups = await safetyCode(self, peer);
      return { peerId: keys.peer.userId, groups };
    },

    async verification(peerId: string): Promise<StoredVerification | null> {
      if (userId === null || support !== "supported") return null;
      return store.getVerification(userId, peerId);
    },

    async setVerified(peerId: string, code: string | null): Promise<void> {
      const user = requireUser();
      await store.putVerification({ userId: user, peerId, code, at: new Date().toISOString() });
      revision += 1;
      publish();
    },


  };
}

export type E2eeClient = ReturnType<typeof createE2eeClient>;

/** An epoch as a call needs it: what its frames name, and the base their keys come from. */
export interface CallEpoch {
  sequence: number;
  epochId: string;
  /** The epoch key as a non-extractable HKDF key. */
  base: CryptoKey;
}

export interface CallKeys {
  /** The signed-in user, whose id goes into every key this side sends under. */
  self: string;
  /** The epoch to send under. */
  send: CallEpoch;
}
