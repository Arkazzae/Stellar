import type { EncryptionApiClient } from "@/lib/api/calls/encryption";
import { ApiError } from "@/lib/api/errors";
import type {
  Attachment,
  ConversationEncryption,
  EncryptionEpoch,
  EpochEnvelope,
  KeyBackupInput,
  Message,
  OwnDevice,
  PublicKeyRecord,
  RecipientKey,
} from "@/lib/api/types";
import { fromBase64, sameBytes, toBase64 } from "@/lib/e2ee/bytes";

/**
 * The encryption routes of the contract, kept in memory for one conversation
 * between two people, with the checks the server makes: coverage of every
 * active key, the current epoch, the switch, the PIN attempts. Protocol tests
 * run real clients against it; `failNext` makes one call refuse once.
 */

const WHEN = "2026-11-18T10:00:00.000Z";
let counter = 0;
const id = () => {
  counter += 1;
  return `00000000-0000-4000-8000-${String(counter).padStart(12, "0")}`;
};

function problem(status: number, code: string, details: Record<string, unknown> = {}): ApiError {
  return new ApiError(status === 403 ? "forbidden" : status === 404 ? "not-found" : "server", code, status, {
    code,
    details: { code, ...details },
  });
}

interface Person {
  userId: string;
  name: string;
  accountKey: PublicKeyRecord | null;
  backup: (KeyBackupInput & { failed: number }) | null;
  devices: { id: string; session: string; publicKey: string; revoked: boolean; label: string }[];
}

interface StoredEpoch {
  epoch: EncryptionEpoch;
  envelopes: EpochEnvelope[];
}

type Method = keyof EncryptionApiClient;

export class FakeEncryptionServer {
  readonly conversationId = id();
  readonly people = new Map<string, Person>();
  encryption: ConversationEncryption = {
    state: "off",
    requestedById: null,
    requestedAt: null,
    enabledAt: null,
    currentEpochId: null,
  };
  readonly epochs: StoredEpoch[] = [];
  readonly messages: Message[] = [];
  readonly uploads = new Map<string, ArrayBuffer>();
  readonly calls: { method: Method; userId: string; input?: unknown }[] = [];
  private readonly failures = new Map<Method, ApiError[]>();

  constructor(first: string, second: string) {
    for (const [name, userId] of [[first, id()], [second, id()]] as const) {
      this.people.set(userId, { userId, name, accountKey: null, backup: null, devices: [] });
    }
  }

  userIds(): [string, string] {
    const [a, b] = [...this.people.keys()];
    return [a ?? "", b ?? ""];
  }

  failNext(method: Method, error: ApiError): void {
    this.failures.set(method, [...(this.failures.get(method) ?? []), error]);
  }

  turnOn(): void {
    this.encryption = { ...this.encryption, state: "on", enabledAt: WHEN, currentEpochId: null, requestedById: null, requestedAt: null };
  }

  person(userId: string): Person {
    const person = this.people.get(userId);
    if (person === undefined) throw new Error("no such person");
    return person;
  }

  peerOf(userId: string): Person {
    return [...this.people.values()].find((person) => person.userId !== userId) as Person;
  }

  activeDevices(userId: string) {
    return this.person(userId).devices.filter((device) => !device.revoked);
  }

  revokeDevice(deviceId: string): void {
    for (const person of this.people.values()) {
      for (const device of person.devices) if (device.id === deviceId) device.revoked = true;
    }
  }

  recipients(): RecipientKey[] {
    return [...this.people.values()].flatMap((person) => [
      ...(person.accountKey === null ? [] : [{ keyId: person.accountKey.keyId, kind: "account" as const, userId: person.userId }]),
      ...this.activeDevices(person.userId).map((device) => ({ keyId: device.id, kind: "device" as const, userId: person.userId })),
    ]);
  }

  private coverageOf(keyIds: readonly string[]) {
    const wanted = new Set(this.recipients().map((key) => key.keyId));
    const given = new Set(keyIds);
    return {
      missing: [...wanted].filter((key) => !given.has(key)),
      extra: [...given].filter((key) => !wanted.has(key)),
    };
  }

  private forReader(stored: StoredEpoch, userId: string, session: string): EncryptionEpoch {
    const person = this.person(userId);
    const device = person.devices.find((candidate) => candidate.session === session && !candidate.revoked);
    const mine = new Set([person.accountKey?.keyId, device?.id].filter((key) => key !== undefined));
    return { ...stored.epoch, envelopes: stored.envelopes.filter((envelope) => mine.has(envelope.recipientKeyId)) };
  }

  /** The API as one signed-in session of `userId` sees it. */
  api(userId: string, session: string): EncryptionApiClient {
    const call = async <T>(method: Method, input: unknown, run: () => T | Promise<T>): Promise<T> => {
      this.calls.push({ method, userId, input });
      const queued = this.failures.get(method);
      const failure = queued?.shift();
      if (failure !== undefined) throw failure;
      return run();
    };
    const person = () => this.person(userId);
    const ownDevice = (device: Person["devices"][number]): OwnDevice => ({
      id: device.id,
      algorithm: "x25519",
      publicKey: device.publicKey,
      label: device.label,
      createdAt: WHEN,
      lastSeenAt: WHEN,
      current: device.session === session,
    });
    const account = () => {
      const current = person().devices.find((device) => device.session === session && !device.revoked);
      const backup = person().backup;
      return {
        accountKey: person().accountKey,
        backup:
          backup === null
            ? null
            : { iterations: backup.iterations, failedAttempts: backup.failed, remainingAttempts: 10 - backup.failed, updatedAt: WHEN },
        device: current === undefined ? null : ownDevice(current),
      };
    };

    return {
      getEncryptionAccount: () => call("getEncryptionAccount", undefined, account),
      setUpAccountKey: (input) =>
        call("setUpAccountKey", input, () => {
          if (person().accountKey !== null) throw problem(409, "account-key-exists");
          person().accountKey = { keyId: id(), algorithm: "x25519", publicKey: input.publicKey, createdAt: WHEN };
          person().backup = { ...input.backup, failed: 0 };
          return { ...account(), device: null };
        }),
      getBackupParameters: () =>
        call("getBackupParameters", undefined, () => {
          const backup = person().backup;
          if (backup === null) throw problem(404, "backup-missing");
          return { salt: backup.salt, iterations: backup.iterations, remainingAttempts: 10 - backup.failed };
        }),
      unlockBackup: (authKey) =>
        call("unlockBackup", authKey, async () => {
          const backup = person().backup;
          if (backup === null) throw problem(404, "backup-missing");
          const verifier = new Uint8Array(await crypto.subtle.digest("SHA-256", fromBase64(authKey)));
          if (!sameBytes(verifier, fromBase64(backup.verifier))) {
            backup.failed += 1;
            if (backup.failed >= 10) {
              person().backup = null;
              throw problem(410, "backup-deleted", { remainingAttempts: 0 });
            }
            throw problem(403, "pin-incorrect", { remainingAttempts: 10 - backup.failed });
          }
          backup.failed = 0;
          return { accountKeyId: person().accountKey?.keyId ?? "", iv: backup.iv, ciphertext: backup.ciphertext };
        }),
      replaceBackup: (input) =>
        call("replaceBackup", input, () => {
          if (input.accountKeyId !== person().accountKey?.keyId) {
            throw problem(409, "account-key-changed", { accountKeyId: person().accountKey?.keyId });
          }
          if (!person().devices.some((device) => device.session === session && !device.revoked)) {
            throw problem(409, "device-missing");
          }
          person().backup = { ...input.backup, failed: 0 };
          return { iterations: input.backup.iterations, failedAttempts: 0, remainingAttempts: 10, updatedAt: WHEN };
        }),
      resetAccountKey: (input) =>
        call("resetAccountKey", input, () => {
          const accountKey = { keyId: id(), algorithm: "x25519", publicKey: input.publicKey, createdAt: WHEN };
          person().accountKey = accountKey;
          person().backup = { ...input.backup, failed: 0 };
          let revokedDevices = 0;
          for (const device of person().devices) {
            if (device.session !== session && !device.revoked) {
              device.revoked = true;
              revokedDevices += 1;
            }
          }
          return {
            accountKey,
            backup: { iterations: input.backup.iterations, failedAttempts: 0, remainingAttempts: 10, updatedAt: WHEN },
            revokedDevices,
          };
        }),
      registerDevice: (input) =>
        call("registerDevice", input, () => {
          if (person().accountKey === null) throw problem(409, "account-key-missing");
          const existing = person().devices.find((device) => device.session === session && !device.revoked);
          if (existing !== undefined && existing.publicKey === input.publicKey) return ownDevice(existing);
          if (existing !== undefined) existing.revoked = true;
          const device = { id: id(), session, publicKey: input.publicKey, revoked: false, label: input.label };
          person().devices.push(device);
          return ownDevice(device);
        }),
      listEncryptionDevices: () =>
        call("listEncryptionDevices", undefined, () => this.activeDevices(userId).map(ownDevice)),
      removeEncryptionDevice: (deviceId) =>
        call("removeEncryptionDevice", deviceId, () => {
          this.revokeDevice(deviceId);
        }),
      switchConversationEncryption: (_conversationId, action) =>
        call("switchConversationEncryption", action, () => {
          if (action === "request") {
            if (this.encryption.state !== "off") throw problem(409, "encryption-state", { encryption: this.encryption });
            this.encryption = { ...this.encryption, state: "requested", requestedById: userId, requestedAt: WHEN };
          } else if (action === "confirm") {
            if (this.encryption.state !== "requested") throw problem(409, "encryption-state", { encryption: this.encryption });
            this.turnOn();
          } else {
            this.encryption = { state: "off", requestedById: null, requestedAt: null, enabledAt: null, currentEpochId: null };
          }
          return this.encryption;
        }),
      getConversationKeys: () =>
        call("getConversationKeys", undefined, () => {
          const keysOf = (subject: Person) => ({
            userId: subject.userId,
            accountKey: subject.accountKey,
            devices: this.activeDevices(subject.userId).map((device) => ({
              keyId: device.id,
              algorithm: "x25519",
              publicKey: device.publicKey,
              createdAt: WHEN,
            })),
          });
          const current = this.epochs.find((stored) => stored.epoch.id === this.encryption.currentEpochId);
          return {
            conversationId: this.conversationId,
            encryption: this.encryption,
            self: keysOf(person()),
            peer: keysOf(this.peerOf(userId)),
            currentDeviceKeyId: person().devices.find((device) => device.session === session && !device.revoked)?.id ?? null,
            recipients: this.recipients(),
            currentEpoch: current === undefined ? null : this.forReader(current, userId, session),
          };
        }),
      listConversationEpochs: (_conversationId, after) =>
        call("listConversationEpochs", after, () => ({
          currentEpochId: this.encryption.currentEpochId,
          epochs: this.epochs
            .filter((stored) => stored.epoch.sequence > after)
            .map((stored) => this.forReader(stored, userId, session)),
          hasMore: false,
        })),
      createConversationEpoch: (_conversationId, input) =>
        call("createConversationEpoch", input, () => {
          const existing = this.epochs.find((stored) => stored.epoch.id === input.epochId);
          if (existing !== undefined) {
            if (existing.epoch.createdById === userId) return existing.epoch;
            throw problem(409, "epoch-exists");
          }
          if (this.encryption.state !== "on") throw problem(409, "encryption-off", { encryption: this.encryption });
          if (input.previousEpochId !== this.encryption.currentEpochId) {
            throw problem(409, "epoch-stale", { currentEpochId: this.encryption.currentEpochId });
          }
          const coverage = this.coverageOf(input.envelopes.map((envelope) => envelope.recipientKeyId));
          if (coverage.missing.length > 0 || coverage.extra.length > 0) throw problem(409, "epoch-coverage", coverage);
          const epoch: EncryptionEpoch = {
            id: input.epochId,
            sequence: this.epochs.length + 1,
            previousEpochId: input.previousEpochId,
            createdById: userId,
            createdAt: WHEN,
            recipients: this.recipients(),
            envelopes: [],
          };
          const stored = { epoch, envelopes: input.envelopes };
          this.epochs.push(stored);
          this.encryption = { ...this.encryption, currentEpochId: epoch.id };
          return this.forReader(stored, userId, session);
        }),
      sendEncryptedDirectMessage: (_conversationId, input) =>
        call("sendEncryptedDirectMessage", input, () => {
          if (this.encryption.state !== "on") throw problem(409, "encryption-off", { encryption: this.encryption });
          if (input.encrypted.epochId !== this.encryption.currentEpochId) {
            throw problem(409, "epoch-not-current", { currentEpochId: this.encryption.currentEpochId });
          }
          const current = this.epochs.find((stored) => stored.epoch.id === this.encryption.currentEpochId);
          const coverage = this.coverageOf(current?.epoch.recipients.map((key) => key.keyId) ?? []);
          if (coverage.missing.length > 0 || coverage.extra.length > 0) throw problem(409, "epoch-outdated", coverage);
          const message: Message = {
            id: id(),
            channelId: this.conversationId,
            author: { id: userId, displayName: person().name },
            content: "",
            attachments: input.attachmentIds.map((upload) => ({
              id: upload,
              url: `/assets/${upload}`,
              name: "encrypted.bin",
              contentType: "application/octet-stream",
              size: this.uploads.get(upload)?.byteLength ?? 0,
            })),
            embeds: [],
            createdAt: WHEN,
            encrypted: input.encrypted,
          };
          this.messages.push(message);
          return message;
        }),
      uploadEncryptedFile: (data) =>
        call("uploadEncryptedFile", undefined, async () => {
          const upload = id();
          this.uploads.set(upload, await data.arrayBuffer());
          const attachment: Attachment = {
            id: upload,
            url: `/assets/${upload}`,
            name: "encrypted.bin",
            contentType: "application/octet-stream",
            size: data.size,
          };
          return attachment;
        }),
      downloadAsset: (url) =>
        call("downloadAsset", url, () => {
          const data = this.uploads.get(url.split("/").pop() ?? "");
          if (data === undefined) throw problem(404, "missing");
          return data;
        }),
    };
  }

  /** The stored ciphertext of a message, for tampering. */
  replaceCiphertext(messageId: string, ciphertext: Uint8Array): Message {
    const message = this.messages.find((item) => item.id === messageId);
    if (message?.encrypted === undefined) throw new Error("no such encrypted message");
    const changed = { ...message, encrypted: { ...message.encrypted, ciphertext: toBase64(ciphertext) } };
    this.messages.splice(this.messages.indexOf(message), 1, changed);
    return changed;
  }
}
