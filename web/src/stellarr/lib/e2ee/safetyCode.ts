import { bytesOf, concatBytes, utf8, type Bytes } from "./bytes";

export interface SafetyParty {
  /** Lowercase UUID, as the server writes it. */
  userId: string;
  /** The raw account public key. */
  publicKey: Bytes;
}

const LABEL = "stellarr/e2ee/safety/v1";
const DIGITS = 60;
const GROUP = 5;

/**
 * The code two people compare to know that nobody swapped their keys.
 *
 * SHA-256 over a fixed label, then each person's user id and account public
 * key in user id order, so both browsers compute the same digits whoever opens
 * it. The 256-bit digest is read as one number and its last 60 decimal digits
 * are shown in 12 groups of 5. Device keys are not part of it on purpose: the
 * code stays the same while people sign in on new browsers, and changes only
 * when an account key does.
 */
export async function safetyCode(left: SafetyParty, right: SafetyParty): Promise<string[]> {
  const [first, second] = [left, right].sort((a, b) => (a.userId < b.userId ? -1 : a.userId > b.userId ? 1 : 0));
  if (first === undefined || second === undefined) return [];
  const input = concatBytes(
    utf8(LABEL),
    utf8(first.userId),
    first.publicKey,
    utf8(second.userId),
    second.publicKey,
  );
  const digest = bytesOf(await crypto.subtle.digest("SHA-256", input));
  let value = 0n;
  for (const byte of digest) value = (value << 8n) | BigInt(byte);
  const digits = (value % 10n ** BigInt(DIGITS)).toString().padStart(DIGITS, "0");
  const groups: string[] = [];
  for (let start = 0; start < DIGITS; start += GROUP) groups.push(digits.slice(start, start + GROUP));
  return groups;
}
