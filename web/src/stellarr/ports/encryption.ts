import type {
  ConversationEncryption,
  EncryptionSwitchAction,
  OwnDevice,
} from "@/lib/api/types";
import type {
  E2eeStatus,
  PreparedPin,
  PreparedReset,
  SafetyCodeView,
  UnlockResult,
} from "@/lib/e2ee/client";

import type { Unsubscribe } from "./chat";

export type { EncryptionSwitchAction, OwnDevice } from "@/lib/api/types";
export type {
  E2eeStatus as EncryptionStatus,
  PreparedPin,
  PreparedReset,
  SafetyCodeView,
  UnlockResult,
} from "@/lib/e2ee/client";

/** What the person compared, and when. */
export interface SafetyVerification {
  /** The groups, joined by spaces. */
  code: string;
  /** ISO 8601. */
  at: string;
}

/**
 * End-to-end encryption of private conversations, from the account's side:
 * the PIN, this browser's keys, the devices that hold keys, the switch of one
 * conversation and its safety code. Reading and sending messages goes through
 * the friends port, which decrypts and encrypts by itself.
 */
export interface EncryptionPort {
  getStatus(): E2eeStatus;
  subscribe(listener: () => void): Unsubscribe;

  /** First set-up: a new account key under this PIN, and this browser's device. */
  setUp(pin: string): Promise<void>;
  /** Opens the PIN backup, so history encrypted for the account opens here. */
  unlock(pin: string): Promise<UnlockResult>;
  /** The PIN derivation is slow, so a change is prepared once and sent as often as re-authentication needs. */
  prepareNewPin(pin: string): Promise<PreparedPin>;
  changePin(prepared: PreparedPin, currentPassword: string): Promise<void>;
  prepareReset(pin: string): Promise<PreparedReset>;
  reset(prepared: PreparedReset, currentPassword: string): Promise<void>;

  listDevices(): Promise<OwnDevice[]>;
  /** Removes a browser's keys, which also signs it out. */
  removeDevice(deviceId: string): Promise<void>;

  switchConversation(conversationId: string, action: EncryptionSwitchAction): Promise<ConversationEncryption>;

  safetyCode(conversationId: string): Promise<SafetyCodeView | null>;
  verification(peerId: string): Promise<SafetyVerification | null>;
  setVerified(peerId: string, code: string): Promise<void>;
  clearVerified(peerId: string): Promise<void>;

  /** Sign-out: deletes this browser's keys for the account. */
  forget(): Promise<void>;
}
