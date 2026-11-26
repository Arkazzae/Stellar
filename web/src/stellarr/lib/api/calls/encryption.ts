import { parseAttachment, parseMessage } from "../chatParsers";
import {
  parseBackupParameters,
  parseBackupStatus,
  parseConversationEncryption,
  parseConversationKeys,
  parseEncryptionAccount,
  parseEpoch,
  parseEpochPage,
  parseOwnDevice,
  parseOwnDevices,
  parsePublicKey,
  parseUnlockedBackup,
} from "../encryptionParsers";
import { ApiError, statusToKind } from "../errors";
import { expectNumber, expectRecord, isRecord } from "../expect";
import { JSON_HEADERS, readJson, request } from "../transport";
import type {
  Attachment,
  BackupParameters,
  ConversationEncryption,
  ConversationKeys,
  EncryptedBody,
  EncryptionAccount,
  EncryptionEpoch,
  EncryptionSwitchAction,
  EpochEnvelope,
  EpochPage,
  KeyBackupInput,
  KeyBackupStatus,
  Message,
  OwnDevice,
  PublicKeyRecord,
  UnlockedBackup,
} from "../types";
import { ACCEPT_JSON, route, withQuery, type Parse } from "./http";

/** The encryption routes of the contract; every byte field is base64. */
export interface EncryptionApiClient {
  getEncryptionAccount(signal?: AbortSignal): Promise<EncryptionAccount>;
  setUpAccountKey(input: { publicKey: string; backup: KeyBackupInput }): Promise<EncryptionAccount>;
  getBackupParameters(): Promise<BackupParameters>;
  unlockBackup(authKey: string, version: 1 | 2): Promise<UnlockedBackup>;
  replaceBackup(input: { currentPassword: string; accountKeyId: string; backup: KeyBackupInput }): Promise<KeyBackupStatus>;
  resetAccountKey(input: {
    migrateFromKeyId?: string;
    expectedAccountKeyId?: string;
    currentPassword: string;
    publicKey: string;
    backup: KeyBackupInput;
  }): Promise<{ accountKey: PublicKeyRecord; backup: KeyBackupStatus; revokedDevices: number }>;
  registerDevice(input: { publicKey: string; label: string }): Promise<OwnDevice>;
  listEncryptionDevices(): Promise<OwnDevice[]>;
  removeEncryptionDevice(deviceId: string): Promise<void>;
  switchConversationEncryption(
    conversationId: string,
    action: EncryptionSwitchAction,
  ): Promise<ConversationEncryption>;
  getConversationKeys(conversationId: string): Promise<ConversationKeys>;
  listConversationEpochs(conversationId: string, after: number): Promise<EpochPage>;
  createConversationEpoch(
    conversationId: string,
    input: { version: 2; epochId: string; previousEpochId: string | null; deviceKeyIds: string[]; envelopes: EpochEnvelope[] },
  ): Promise<EncryptionEpoch>;
  sendEncryptedDirectMessage(
    conversationId: string,
    input: { clientMessageId: string; attachmentIds: string[]; encrypted: EncryptedBody },
  ): Promise<Message>;
  uploadEncryptedFile(data: Blob): Promise<Attachment>;
  /** The stored bytes of an upload, as they are (still encrypted). */
  downloadAsset(url: string): Promise<ArrayBuffer>;
}

const ALGORITHM = "x25519";

/**
 * Throws the refusal with its whole problem document.
 *
 * The protocol branches on the code of a 409, 410 or 404 and needs the
 * fields beside it (`currentEpochId`, `missing`, `encryption`,
 * `remainingAttempts`), so unlike the shared helper this reads the body of
 * every refusal, not only of a 403.
 */
async function refuse(response: Response, path: string): Promise<never> {
  let body: unknown = null;
  try {
    body = await response.json();
  } catch {
    // No body: a 429, or a proxy's page.
  }
  const details = isRecord(body) ? body : undefined;
  const code = details !== undefined && typeof details["code"] === "string" ? details["code"] : undefined;
  throw new ApiError(statusToKind(response.status), `${path} responded ${response.status}`, response.status, {
    ...(code === undefined ? {} : { code }),
    ...(details === undefined ? {} : { details }),
  });
}

async function call<T>(
  method: "GET" | "POST" | "PUT" | "DELETE",
  path: string,
  body: unknown,
  parse: Parse<T> | null,
  signal?: AbortSignal,
): Promise<T> {
  const init: RequestInit =
    body === undefined
      ? { method, headers: ACCEPT_JSON }
      : { method, headers: JSON_HEADERS, body: JSON.stringify(body) };
  const response = await request(path, init, signal);
  if (!response.ok) await refuse(response, path);
  if (parse === null) return undefined as T;
  return parse(await readJson(response, path), path);
}

const parseReset: Parse<{ accountKey: PublicKeyRecord; backup: KeyBackupStatus; revokedDevices: number }> = (
  value,
  path,
) => {
  const record = expectRecord(value, path);
  return {
    accountKey: parsePublicKey(record["accountKey"], `${path}.accountKey`),
    backup: parseBackupStatus(record["backup"], `${path}.backup`),
    revokedDevices: expectNumber(record, "revokedDevices", path),
  };
};

export function encryptionCalls(): EncryptionApiClient {
  return {
    getEncryptionAccount: (signal) =>
      call("GET", "/api/me/encryption", undefined, parseEncryptionAccount, signal),

    setUpAccountKey: ({ publicKey, backup }) =>
      call("POST", "/api/me/encryption/account-key", { algorithm: ALGORITHM, publicKey, backup }, parseEncryptionAccount),

    getBackupParameters: () => call("GET", "/api/me/encryption/backup", undefined, parseBackupParameters),

    unlockBackup: (authKey, version) => call("POST", "/api/me/encryption/backup/unlock", { version, authKey }, parseUnlockedBackup),

    replaceBackup: (input) => call("PUT", "/api/me/encryption/backup", input, parseBackupStatus),

    resetAccountKey: ({ currentPassword, publicKey, backup, migrateFromKeyId, expectedAccountKeyId }) =>
      call(
        "POST",
        "/api/me/encryption/account-key/reset",
        { currentPassword, algorithm: ALGORITHM, publicKey, backup, migrateFromKeyId, expectedAccountKeyId },
        parseReset,
      ),

    registerDevice: ({ publicKey, label }) =>
      call("POST", "/api/me/encryption/devices", { algorithm: ALGORITHM, publicKey, label }, parseOwnDevice),

    listEncryptionDevices: () => call("GET", "/api/me/encryption/devices", undefined, parseOwnDevices),

    removeEncryptionDevice: (deviceId) =>
      call("DELETE", route`/api/me/encryption/devices/${deviceId}`, undefined, null),

    switchConversationEncryption: (conversationId, action) =>
      call(
        "POST",
        `${route`/api/social/conversations/${conversationId}`}/encryption/${action}`,
        undefined,
        parseConversationEncryption,
      ),

    getConversationKeys: (conversationId) =>
      call("GET", route`/api/social/conversations/${conversationId}/encryption/keys`, undefined, parseConversationKeys),

    listConversationEpochs: (conversationId, after) =>
      call(
        "GET",
        withQuery(route`/api/social/conversations/${conversationId}/encryption/epochs`, { after }),
        undefined,
        parseEpochPage,
      ),

    createConversationEpoch: (conversationId, input) =>
      call("POST", route`/api/social/conversations/${conversationId}/encryption/epochs`, input, parseEpoch),

    sendEncryptedDirectMessage: (conversationId, input) =>
      call("POST", route`/api/social/conversations/${conversationId}/messages`, input, parseMessage),

    async uploadEncryptedFile(data) {
      const path = "/api/uploads/encrypted";
      const form = new FormData();
      form.append("file", data, "encrypted.bin");
      const response = await request(path, { method: "POST", body: form, headers: ACCEPT_JSON });
      if (!response.ok) await refuse(response, path);
      return parseAttachment(await readJson(response, path), path);
    },

    async downloadAsset(url) {
      const response = await request(url, { method: "GET" });
      if (!response.ok) await refuse(response, url);
      return response.arrayBuffer();
    },
  };
}

/** The problem code of a refusal, when it carried one. */
export function problemCodeOf(error: unknown): string | undefined {
  return error instanceof ApiError ? error.code : undefined;
}
