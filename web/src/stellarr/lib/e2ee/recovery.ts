import { fromBase64, fromUtf8, randomBytes, toBase64, utf8, type Bytes } from "./bytes";
import { privateKeyMatches } from "./keys";

export function generateRecoveryCode(): string {
  const hex = Array.from(randomBytes(32), (byte) => byte.toString(16).padStart(2, "0")).join("").toUpperCase();
  return hex.match(/.{8}/g)!.join("-");
}

export function normalizeRecoveryCode(value: string): string | null {
  const hex = value.replace(/[\s-]/g, "").toUpperCase();
  return /^[0-9A-F]{64}$/.test(hex) ? hex : null;
}

/** The existing password-based envelope uses a distinct input domain for generated codes. */
export function recoveryCredential(code: string): string {
  const hex = normalizeRecoveryCode(code);
  if (hex === null) throw new Error("Invalid recovery code");
  return "stellarr/e2ee/recovery-code/v2/" + hex;
}

export interface ArchivedKey {
  keyId: string;
  publicKey: Bytes;
  pkcs8: Bytes;
}

export function encodeKeyBackup(pkcs8: Bytes, archived?: ArchivedKey): Bytes {
  return utf8(JSON.stringify({
    version: 2,
    privateKey: toBase64(pkcs8),
    ...(archived && { archived: {
      keyId: archived.keyId, publicKey: toBase64(archived.publicKey), privateKey: toBase64(archived.pkcs8),
    } }),
  }));
}

export async function decodeKeyBackup(bytes: Bytes): Promise<{ pkcs8: Bytes; archived?: ArchivedKey }> {
  const value = JSON.parse(fromUtf8(bytes));
  if (value?.version !== 2 || typeof value.privateKey !== "string") throw new Error("Invalid key backup");
  const pkcs8 = fromBase64(value.privateKey);
  const old = value.archived;
  if (old === undefined) return { pkcs8 };
  if (typeof old.keyId !== "string" || typeof old.publicKey !== "string" || typeof old.privateKey !== "string") {
    throw new Error("Invalid archived key");
  }
  const archived = { keyId: old.keyId, publicKey: fromBase64(old.publicKey), pkcs8: fromBase64(old.privateKey) };
  if (!await privateKeyMatches(archived.pkcs8, archived.publicKey)) throw new Error("Invalid archived key pair");
  return { pkcs8, archived };
}
