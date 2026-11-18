import type { E2eeSupport } from "@/lib/e2ee/support";
import { callEncryptionPath } from "@/lib/voice/e2ee/callCrypto";
import type { DmConversation } from "@/ports";
import { useEncryption } from "@/state/hooks";

/** Why this browser cannot make or answer a conversation's call. */
export type CallBlock = "unsupported" | "insecure" | "device-limit";

/**
 * An encrypted conversation's call is encrypted or does not happen, so a
 * browser that cannot encrypt one is told before it tries: without encoded
 * transforms or the encryption the messages need, or without keys of its
 * own because the account has reached its limit of browsers. While the
 * encryption support is still being checked nothing is blocked; the voice
 * client refuses anyway if it comes to that.
 */
export function callBlockOf(
  conversation: Pick<DmConversation, "encryption"> | null | undefined,
  support: E2eeSupport | "checking",
  deviceLimit: boolean,
  transforms: boolean,
): CallBlock | null {
  if (conversation?.encryption?.state !== "on") return null;
  if (support === "insecure") return "insecure";
  if (support === "unsupported" || !transforms) return "unsupported";
  if (deviceLimit) return "device-limit";
  return null;
}

export function useCallBlock(conversation: Pick<DmConversation, "encryption"> | null | undefined): CallBlock | null {
  const { status } = useEncryption();
  return callBlockOf(conversation, status.support, status.deviceLimit, callEncryptionPath() !== null);
}

export const CALL_BLOCKED_KEY = {
  unsupported: "dm.callBlocked.unsupported",
  insecure: "dm.callBlocked.insecure",
  "device-limit": "dm.callBlocked.deviceLimit",
} as const;

export const INCOMING_BLOCKED_KEY = {
  unsupported: "dm.incomingBlocked.unsupported",
  insecure: "dm.incomingBlocked.insecure",
  "device-limit": "dm.incomingBlocked.deviceLimit",
} as const;
