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
  PublicKeyRecord,
  RecipientKey,
  SealedReason,
} from "@/lib/api/types";
import type { UploadMediaMetadata } from "@/lib/media/probe";

import { decryptFile, encryptFile, OPAQUE_TYPE, shownType } from "./attachment";
import { fromBase64, sameBytes, toBase64, type Bytes } from "./bytes";
import { importEpochKey, newEpochKey, unwrapEpochKey, wrapEpochKey } from "./epoch";
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
import { derivePinKeys, openBackup, PIN_ITERATIONS, sealBackup, type SealedBackup } from "./pin";
import { safetyCode } from "./safetyCode";
import type { KeyStore, StoredVerification } from "./store";
import type { E2eeSupport } from "./support";

/**
 * The browser side of the encryption protocol: keys, PIN backup, devices,
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
  /** The PIN backup on the server; null when there is none. */
  backup: { remainingAttempts: number } | null;
  /** This browser holds the account private key. */
  unlocked: boolean;
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
  | { type: "attachments"; messageId: string };

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
  | { kind: "wrong-pin"; remainingAttempts: number }
  /** That was the last wrong PIN; the backup is gone. */
  | { kind: "backup-deleted" }
  | { kind: "backup-missing" }
  /** Refused before trying: too many attempts lately. No attempt was spent. */
  | { kind: "rate-limited" }
  /** The PIN was right, but what the backup holds is not this account's key. */
  | { kind: "backup-unusable" };

/** One encrypted upload, ready to go into a message. */
export interface EncryptedUpload {
  /** The file's upload id and its poster's or preview's, all of which the message must list. */
  attachmentIds: string[];
  payload: PayloadAttachment;
}

export interface PreparedPin {
  backup: KeyBackupInput;
}

export interface PreparedReset extends PreparedPin {
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

/** The public key a recipient entry stands for, from the keys route's answer. */
export function publicKeyOf(keys: ConversationKeys, recipient: RecipientKey): PublicKeyRecord | null {
  const person = [keys.self, keys.peer].find((candidate) => candidate.userId === recipient.userId);
  if (person === undefined) return null;
  if (recipient.kind === "account") {
    return person.accountKey?.keyId === recipient.keyId ? person.accountKey : null;
  }
  const device = person.devices.find((candidate) => candidate.keyId === recipient.keyId);
  return device === undefined
    ? null
    : { keyId: device.keyId, algorithm: device.algorithm, publicKey: device.publicKey, createdAt: device.createdAt };
}

function backupInput(backup: SealedBackup): KeyBackupInput {
  return {
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
  opened: Map<string, CryptoKey>;
  /** Rotation is decided one send at a time, so two sends never race to create epochs. */
  queue: Promise<unknown>;
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
        opened: new Map(),
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

  async function adoptAccountKey(user: string, keyId: string, publicKey: Bytes, pkcs8: Bytes): Promise<void> {
    const sealed = await sealLocally(pkcs8);
    await store.putAccountKey({ userId: user, keyId, publicKey, ...sealed });
    accountKey = { keyId, publicKey, privateKey: await importPrivateKey(pkcs8, false) };
  }

  async function loadLocalAccountKey(user: string): Promise<void> {
    const stored = await store.getAccountKey(user);
    const server = account.accountKey;
    if (stored === null) {
      accountKey = null;
      return;
    }
    // A reset elsewhere left this copy behind: it opens nothing new.
    if (server === null || stored.keyId !== server.keyId) {
      await store.deleteAccountKey(user);
      accountKey = null;
      return;
    }
    if (accountKey?.keyId === stored.keyId) return;
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
      .then((keys) => {
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

  async function openEpoch(conversationId: string, epoch: EncryptionEpoch): Promise<CryptoKey | null> {
    const state = conversation(conversationId);
    const opened = state.opened.get(epoch.id);
    if (opened !== undefined) return opened;
    const context = { conversationId, epochId: epoch.id };
    const attempts: [EpochEnvelope | undefined, CryptoKey | undefined][] = [
      [epoch.envelopes.find((envelope) => envelope.recipientKeyId === device?.id), device?.privateKey],
      [epoch.envelopes.find((envelope) => envelope.recipientKeyId === accountKey?.keyId), accountKey?.privateKey],
    ];
    for (const [envelope, privateKey] of attempts) {
      if (envelope === undefined || privateKey === undefined) continue;
      try {
        const key = await unwrapEpochKey(
          {
            recipientKeyId: envelope.recipientKeyId,
            ephemeralPublicKey: fromBase64(envelope.ephemeralPublicKey),
            iv: fromBase64(envelope.iv),
            ciphertext: fromBase64(envelope.ciphertext),
          },
          privateKey,
          context,
        );
        state.opened.set(epoch.id, key);
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
        epoch.envelopes.some((envelope) => envelope.recipientKeyId === current);
      if (forAccount) return "pin";
    }
    return "lost";
  }

  async function createEpoch(conversationId: string, keys: ConversationKeys) {
    const epochId = crypto.randomUUID();
    const raw = newEpochKey();
    const envelopes = await Promise.all(
      keys.recipients.map(async (recipient) => {
        const record = publicKeyOf(keys, recipient);
        if (record === null || record.algorithm !== KEY_ALGORITHM) throw new E2eeError("recipient-key-missing");
        const envelope = await wrapEpochKey(raw, fromBase64(record.publicKey), {
          conversationId,
          epochId,
          recipientKeyId: recipient.keyId,
        });
        return {
          recipientKeyId: envelope.recipientKeyId,
          ephemeralPublicKey: toBase64(envelope.ephemeralPublicKey),
          iv: toBase64(envelope.iv),
          ciphertext: toBase64(envelope.ciphertext),
        };
      }),
    );
    const epoch = await api.createConversationEpoch(conversationId, {
      epochId,
      previousEpochId: keys.encryption.currentEpochId,
      envelopes,
    });
    const key = await importEpochKey(raw);
    const state = conversation(conversationId);
    remember(state, epoch);
    state.opened.set(epoch.id, key);
    keys.currentEpoch = epoch;
    keys.encryption = { ...keys.encryption, currentEpochId: epoch.id };
    return { epochId: epoch.id, key };
  }

  /**
   * The epoch the next message goes out with, rotating first when the
   * conversation has none since it was switched on, when this browser cannot
   * open the current one, or when the keys it was wrapped for are no longer
   * exactly today's.
   */
  async function decideEpoch(conversationId: string): Promise<{ epochId: string; key: CryptoKey }> {
    for (let attempt = 0; attempt < MAX_ATTEMPTS; attempt += 1) {
      const keys = await keysOf(conversationId, attempt > 0);
      if (keys.encryption.state !== "on") {
        // The cached state may predate the switch; the server's word decides.
        if (attempt === 0) continue;
        throw new E2eeError("encryption-off");
      }
      const current = keys.currentEpoch;
      if (
        current !== null &&
        current.id === keys.encryption.currentEpochId &&
        sameRecipients(current.recipients, keys.recipients)
      ) {
        const key = await openEpoch(conversationId, conversation(conversationId).epochs.get(current.id) ?? current);
        if (key !== null) return { epochId: current.id, key };
      }
      try {
        return await createEpoch(conversationId, keys);
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

  function epochForSending(conversationId: string) {
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
    const eager = (type.startsWith("image/") || type.startsWith("audio/")) && item.size <= EAGER_LIMIT;
    if (eager) {
      const url = objectUrls.get(item.id);
      const previewUrl = type.startsWith("image/") ? pictureUrl(item.preview, messageId) : undefined;
      if (url !== undefined) {
        return { ...base, contentType: type, url, ...(previewUrl === undefined ? {} : { previewUrl }) };
      }
      decryptSoon(item.id, { url: serverUrl, key: item.key, iv: item.iv, type }, type, messageId);
      // Until it is ready (or if it fails) it is a file card without a link.
      return { ...base, contentType: OPAQUE_TYPE, url: "" };
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
  }

  /* ------------------------------------------------------------ the client */

  return {
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

    /* ---------------- PIN and account key */

    /** First set-up: a new account key, its backup under `pin`, and this browser's device. */
    async setUp(pin: string): Promise<void> {
      await ready;
      const user = requireUser();
      if (support !== "supported") throw new E2eeError("unsupported");
      const pair = await generateKeyPair(true);
      const pkcs8 = await exportPrivateKey(pair.privateKey);
      const publicKey = await exportPublicKey(pair.publicKey);
      const backup = await sealBackup(pkcs8, pin, iterations);
      try {
        account = await api.setUpAccountKey({ publicKey: toBase64(publicKey), backup: backupInput(backup) });
      } catch (error) {
        // Set up in another tab a moment ago: read it rather than fail.
        if (codeOf(error) === "account-key-exists") {
          await sync();
          return;
        }
        throw error;
      }
      if (account.accountKey !== null) {
        await adoptAccountKey(user, account.accountKey.keyId, publicKey, pkcs8);
      }
      await ensureDevice(user);
      publish();
    },

    /** Opens the backup with `pin` and keeps the account key on this browser. */
    async unlock(pin: string): Promise<UnlockResult> {
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
      const { authKey, encKey } = await derivePinKeys(pin, fromBase64(parameters.salt), parameters.iterations);
      let unlocked;
      try {
        unlocked = await api.unlockBackup(toBase64(authKey));
      } catch (error) {
        const code = codeOf(error);
        const details = error instanceof ApiError ? error.details : undefined;
        if (code === "pin-incorrect") {
          const remaining = typeof details?.["remainingAttempts"] === "number" ? details["remainingAttempts"] : 0;
          account = { ...account, backup: account.backup === null ? null : { ...account.backup, remainingAttempts: remaining } };
          publish();
          return { kind: "wrong-pin", remainingAttempts: remaining };
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
      try {
        pkcs8 = await openBackup(encKey, fromBase64(unlocked.iv), fromBase64(unlocked.ciphertext));
      } catch {
        return { kind: "backup-unusable" };
      }
      const publicKey = fromBase64(server.publicKey);
      // The backup must hold this account's current key before it is used or
      // ever written again under another PIN.
      if (unlocked.accountKeyId !== server.keyId || !(await privateKeyMatches(pkcs8, publicKey))) {
        return { kind: "backup-unusable" };
      }
      await adoptAccountKey(user, server.keyId, publicKey, pkcs8);
      if (account.backup !== null) {
        account = { ...account, backup: { ...account.backup, failedAttempts: 0, remainingAttempts: account.backup.failedAttempts + account.backup.remainingAttempts } };
      }
      keysChanged();
      return { kind: "unlocked" };
    },

    /** A backup of the key this browser holds, under a new PIN; slow, so done once per attempt. */
    async prepareNewPin(pin: string): Promise<PreparedPin> {
      const user = requireUser();
      const stored = await store.getAccountKey(user);
      if (stored === null || stored.keyId !== account.accountKey?.keyId) throw new E2eeError("locked");
      return { backup: backupInput(await sealBackup(await openLocally(stored), pin, iterations)) };
    },

    /** Stores the new backup. Throws the server's refusals (password, second step) as they are. */
    async changePin(prepared: PreparedPin, currentPassword: string): Promise<void> {
      const user = requireUser();
      const keyId = account.accountKey?.keyId;
      if (keyId === undefined || accountKey?.keyId !== keyId) throw new E2eeError("locked");
      const replace = () => api.replaceBackup({ currentPassword, accountKeyId: keyId, backup: prepared.backup });
      try {
        await replace();
      } catch (error) {
        const code = codeOf(error);
        if (code === "account-key-changed") {
          await store.deleteAccountKey(user);
          accountKey = null;
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

    /** A new account key and its backup under `pin`, for when no browser holds the old one. */
    async prepareReset(pin: string): Promise<PreparedReset> {
      const pair = await generateKeyPair(true);
      const pkcs8 = await exportPrivateKey(pair.privateKey);
      return {
        publicKey: toBase64(await exportPublicKey(pair.publicKey)),
        pkcs8,
        backup: backupInput(await sealBackup(pkcs8, pin, iterations)),
      };
    },

    async reset(prepared: PreparedReset, currentPassword: string): Promise<void> {
      const user = requireUser();
      const answer = await api.resetAccountKey({
        currentPassword,
        publicKey: prepared.publicKey,
        backup: prepared.backup,
      });
      account = { ...account, accountKey: answer.accountKey, backup: answer.backup };
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
      encryptionChanged(conversationId, encryption);
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
      const key = epoch === null ? null : await openEpoch(conversationId, epoch);
      if (key === null) return sealed(message, sealedReason(epoch));
      let payload: MessagePayload;
      try {
        payload = await decryptPayload(
          key,
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
      input: { text: string; uploads: readonly EncryptedUpload[]; clientMessageId: string },
    ): Promise<Message> {
      await ready;
      const user = requireUser();
      if (support !== "supported") throw new E2eeError("unsupported");
      if (device?.id === undefined || device.id === null) await sync();
      if (device?.id === undefined || device.id === null) throw new E2eeError("no-device");
      const payload: MessagePayload = { v: 1, text: input.text, attachments: input.uploads.map((upload) => upload.payload) };
      const attachmentIds = input.uploads.flatMap((upload) => upload.attachmentIds);
      let networkRetries = 0;
      for (let attempt = 0; attempt < MAX_ATTEMPTS; attempt += 1) {
        const { epochId, key } = await epochForSending(conversationId);
        const body = await encryptPayload(key, { conversationId, epochId, authorId: user }, payload);
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

    /* ---------------- safety code */

    async safetyCode(conversationId: string): Promise<SafetyCodeView | null> {
      const keys = await keysOf(conversationId, conversation(conversationId).keysStale);
      const self = keys.self.accountKey;
      const peer = keys.peer.accountKey;
      if (self === null || peer === null) return null;
      const groups = await safetyCode(
        { userId: keys.self.userId, publicKey: fromBase64(self.publicKey) },
        { userId: keys.peer.userId, publicKey: fromBase64(peer.publicKey) },
      );
      return { peerId: keys.peer.userId, groups };
    },

    async verification(peerId: string): Promise<StoredVerification | null> {
      if (userId === null || support !== "supported") return null;
      return store.getVerification(userId, peerId);
    },

    async setVerified(peerId: string, code: string): Promise<void> {
      const user = requireUser();
      await store.putVerification({ userId: user, peerId, code, at: new Date().toISOString() });
      revision += 1;
      publish();
    },

    async clearVerified(peerId: string): Promise<void> {
      const user = requireUser();
      await store.deleteVerification(user, peerId);
      revision += 1;
      publish();
    },
  };
}

export type E2eeClient = ReturnType<typeof createE2eeClient>;
