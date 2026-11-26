import type {
  ConversationEncryption,
  EncryptionSwitchAction,
  OwnDevice,
} from "@/lib/api/types";
import type {
  E2eeStatus,
  PreparedRecoveryCode,
  PreparedReset,
  SafetyCodeView,
  UnlockResult,
} from "@/lib/e2ee/client";

import type { Unsubscribe } from "./chat";

export type { EncryptionSwitchAction, OwnDevice } from "@/lib/api/types";
export type {
  E2eeStatus as EncryptionStatus,
  PreparedRecoveryCode,
  PreparedReset,
  SafetyCodeView,
  UnlockResult,
} from "@/lib/e2ee/client";

/** What the person compared, and when. */
export interface SafetyVerification {
  /** The groups, joined by spaces. */
  code: string | null;
  /** ISO 8601. */
  at: string;
}

/**
 * End-to-end encryption of private conversations, from the account's side:
 * the recovery code, this browser's keys, the devices that hold keys, the switch of one
 * conversation and its safety code. Reading and sending messages goes through
 * the friends port, which decrypts and encrypts by itself.
 */
export interface EncryptionPort {
  getStatus(): E2eeStatus;
  subscribe(listener: () => void): Unsubscribe;

  /** First set-up: a new account key under this recovery code, and this browser's device. */
  setUp(code: string): Promise<void>;
  /** Opens the key backup, so history encrypted for the account opens here. */
  unlock(input: string): Promise<UnlockResult>;
  /** Prepare the encrypted backup once for retries during re-authentication. */
  prepareRecoveryCode(code: string): Promise<PreparedRecoveryCode>;
  changeRecoveryCode(prepared: PreparedRecoveryCode, currentPassword: string): Promise<void>;
  prepareReset(code: string): Promise<PreparedReset>;
  reset(prepared: PreparedReset, currentPassword: string): Promise<void>;

  listDevices(): Promise<OwnDevice[]>;
  /** Removes a browser's keys, which also signs it out. */
  removeDevice(deviceId: string): Promise<void>;

  switchConversation(conversationId: string, action: EncryptionSwitchAction): Promise<ConversationEncryption>;

  safetyCode(conversationId: string): Promise<SafetyCodeView | null>;
  verification(peerId: string): Promise<SafetyVerification | null>;
  setVerified(peerId: string, code: string | null): Promise<void>;
  plaintextAllowed(conversationId: string, peerId: string): Promise<boolean>;

  /** Sign-out: deletes this browser's keys for the account. */
  forget(): Promise<void>;
}
