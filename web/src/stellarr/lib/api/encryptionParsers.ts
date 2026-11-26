import { ApiError } from "./errors";
import { expectArray, expectBoolean, expectNumber, expectRecord, expectString, isRecord } from "./expect";
import {
  ENCRYPTION_OFF,
  type BackupParameters,
  type ConversationEncryption,
  type ConversationKeys,
  type DeviceKeyRecord,
  type EncryptionAccount,
  type EncryptionEpoch,
  type EpochEnvelope,
  type EpochPage,
  type KeyBackupStatus,
  type OwnDevice,
  type PersonKeys,
  type PublicKeyRecord,
  type RecipientKey,
  type UnlockedBackup,
} from "./types";

/** The wire shapes of the encryption routes (contract §2). */

function nullableString(record: Record<string, unknown>, key: string, path: string): string | null {
  const value = record[key];
  if (value === null || value === undefined) return null;
  if (typeof value !== "string") throw new ApiError("malformed", `${path}: "${key}" must be a string or null`);
  return value;
}

export function parseConversationEncryption(value: unknown, path: string): ConversationEncryption {
  // A server that predates encryption sends none: such a conversation is off.
  if (value === null || value === undefined) return ENCRYPTION_OFF;
  const record = expectRecord(value, path);
  const state = record["state"];
  return {
    state: state === "requested" || state === "on" ? state : "off",
    requestedById: nullableString(record, "requestedById", path),
    requestedAt: nullableString(record, "requestedAt", path),
    enabledAt: nullableString(record, "enabledAt", path),
    currentEpochId: nullableString(record, "currentEpochId", path),
  };
}

export function parsePublicKey(value: unknown, path: string): PublicKeyRecord {
  const record = expectRecord(value, path);
  return {
    keyId: expectString(record, "keyId", path),
    algorithm: expectString(record, "algorithm", path),
    publicKey: expectString(record, "publicKey", path),
    createdAt: expectString(record, "createdAt", path),
  };
}

function nullablePublicKey(value: unknown, path: string): PublicKeyRecord | null {
  return value === null || value === undefined ? null : parsePublicKey(value, path);
}

export function parseBackupStatus(value: unknown, path: string): KeyBackupStatus {
  const record = expectRecord(value, path);
  return {
    iterations: expectNumber(record, "iterations", path),
    failedAttempts: expectNumber(record, "failedAttempts", path),
    remainingAttempts: expectNumber(record, "remainingAttempts", path),
    updatedAt: expectString(record, "updatedAt", path),
  };
}

export function parseOwnDevice(value: unknown, path: string): OwnDevice {
  const record = expectRecord(value, path);
  return {
    id: expectString(record, "id", path),
    algorithm: expectString(record, "algorithm", path),
    publicKey: expectString(record, "publicKey", path),
    label: expectString(record, "label", path),
    createdAt: expectString(record, "createdAt", path),
    lastSeenAt: expectString(record, "lastSeenAt", path),
    current: record["current"] === true,
  };
}

function recoveryVersion(value: unknown): 1 | 2 {
  if (value === 1) return 1;
  if (value === 2) return 2;
  throw new ApiError("malformed", "Unknown account recovery version");
}

export function parseEncryptionAccount(value: unknown, path: string): EncryptionAccount {
  const record = expectRecord(value, path);
  return {
    ...(record["recoveryVersion"] === undefined ? {} : { recoveryVersion: recoveryVersion(record["recoveryVersion"]) }),
    accountKey: nullablePublicKey(record["accountKey"], `${path}.accountKey`),
    backup:
      record["backup"] === null || record["backup"] === undefined
        ? null
        : parseBackupStatus(record["backup"], `${path}.backup`),
    device:
      record["device"] === null || record["device"] === undefined
        ? null
        : parseOwnDevice(record["device"], `${path}.device`),
  };
}

export function parseOwnDevices(value: unknown, path: string): OwnDevice[] {
  return expectArray(expectRecord(value, path)["devices"], `${path}.devices`).map((item) =>
    parseOwnDevice(item, `${path}.devices[]`),
  );
}

export function parseBackupParameters(value: unknown, path: string): BackupParameters {
  const record = expectRecord(value, path);
  return {
    version: recoveryVersion(record["version"]),
    salt: expectString(record, "salt", path),
    iterations: expectNumber(record, "iterations", path),
    remainingAttempts: expectNumber(record, "remainingAttempts", path),
  };
}

export function parseUnlockedBackup(value: unknown, path: string): UnlockedBackup {
  const record = expectRecord(value, path);
  return {
    accountKeyId: expectString(record, "accountKeyId", path),
    iv: expectString(record, "iv", path),
    ciphertext: expectString(record, "ciphertext", path),
  };
}

function parseDeviceKey(value: unknown, path: string): DeviceKeyRecord {
  const record = expectRecord(value, path);
  return {
    keyId: expectString(record, "keyId", path),
    algorithm: expectString(record, "algorithm", path),
    publicKey: expectString(record, "publicKey", path),
    createdAt: expectString(record, "createdAt", path),
  };
}

function parseRecipient(value: unknown, path: string): RecipientKey {
  const record = expectRecord(value, path);
  return {
    keyId: expectString(record, "keyId", path),
    kind: record["kind"] === "account" ? "account" : "device",
    userId: expectString(record, "userId", path),
  };
}

function parseEnvelope(value: unknown, path: string): EpochEnvelope {
  const record = expectRecord(value, path);
  return {
    recipientKeyId: expectString(record, "recipientKeyId", path),
    ephemeralPublicKey: expectString(record, "ephemeralPublicKey", path),
    iv: expectString(record, "iv", path),
    ciphertext: expectString(record, "ciphertext", path),
  };
}

export function parseEpoch(value: unknown, path: string): EncryptionEpoch {
  const record = expectRecord(value, path);
  const version = record["version"];
  if (version !== undefined && version !== 1 && version !== 2) throw new ApiError("malformed", "Unknown epoch version");
  return {
    ...(version === undefined ? {} : { version }),
    id: expectString(record, "id", path),
    sequence: expectNumber(record, "sequence", path),
    previousEpochId: nullableString(record, "previousEpochId", path),
    createdById: expectString(record, "createdById", path),
    createdAt: expectString(record, "createdAt", path),
    recipients: expectArray(record["recipients"], `${path}.recipients`).map((item) =>
      parseRecipient(item, `${path}.recipients[]`),
    ),
    envelopes: expectArray(record["envelopes"], `${path}.envelopes`).map((item) =>
      parseEnvelope(item, `${path}.envelopes[]`),
    ),
  };
}

function parsePersonKeys(value: unknown, path: string): PersonKeys {
  const record = expectRecord(value, path);
  return {
    userId: expectString(record, "userId", path),
    accountKey: nullablePublicKey(record["accountKey"], `${path}.accountKey`),
    devices: expectArray(record["devices"], `${path}.devices`).map((item) =>
      parseDeviceKey(item, `${path}.devices[]`),
    ),
  };
}

export function parseConversationKeys(value: unknown, path: string): ConversationKeys {
  const record = expectRecord(value, path);
  const current = record["currentEpoch"];
  return {
    conversationId: expectString(record, "conversationId", path),
    encryption: parseConversationEncryption(record["encryption"], `${path}.encryption`),
    self: parsePersonKeys(record["self"], `${path}.self`),
    peer: parsePersonKeys(record["peer"], `${path}.peer`),
    currentDeviceKeyId: nullableString(record, "currentDeviceKeyId", path),
    recipients: expectArray(record["recipients"], `${path}.recipients`).map((item) =>
      parseRecipient(item, `${path}.recipients[]`),
    ),
    currentEpoch: current === null || current === undefined ? null : parseEpoch(current, `${path}.currentEpoch`),
  };
}

export function parseEpochPage(value: unknown, path: string): EpochPage {
  const record = expectRecord(value, path);
  return {
    currentEpochId: nullableString(record, "currentEpochId", path),
    epochs: expectArray(record["epochs"], `${path}.epochs`).map((item) => parseEpoch(item, `${path}.epochs[]`)),
    hasMore: expectBoolean(record, "hasMore", path),
  };
}

/** `{ conversationId, encryption }`, as `DirectEncryptionChanged` carries it. */
export function parseEncryptionChanged(value: unknown, path: string) {
  const record = expectRecord(value, path);
  return {
    conversationId: expectString(record, "conversationId", path),
    encryption: parseConversationEncryption(record["encryption"], `${path}.encryption`),
  };
}

export function parseEpochCreated(value: unknown, path: string) {
  const record = expectRecord(value, path);
  return {
    conversationId: expectString(record, "conversationId", path),
    epoch: parseEpoch(record["epoch"], `${path}.epoch`),
  };
}

export function parseKeysChanged(value: unknown, path: string) {
  const record = expectRecord(value, path);
  return {
    conversationId: expectString(record, "conversationId", path),
    userId: expectString(record, "userId", path),
  };
}

/** The problem document's extra fields, when the body is one. */
export function problemFields(value: unknown): Record<string, unknown> | undefined {
  return isRecord(value) ? value : undefined;
}
