import { bytesOf, randomBytes, utf8, type Bytes } from "./bytes";

/** What the client asks of a new backup; the server accepts 600000 to 10000000. */
export const PIN_ITERATIONS = 600_000;
export const PIN_LENGTH = 6;
const SALT_LENGTH = 16;

const AUTH_INFO = "stellarr/e2ee/pin/auth/v1";
const ENC_INFO = "stellarr/e2ee/pin/enc/v1";
/**
 * The HKDF salt for both PIN keys: zero-length, and never to change. The PBKDF2
 * step already has the backup's own salt; changing this one later would make
 * every existing backup unopenable.
 */
const HKDF_SALT = new Uint8Array(0);

export function isPin(text: string): boolean {
  return new RegExp(`^\\d{${PIN_LENGTH}}$`).test(text);
}

export interface PinKeys {
  /** Sent to the server, which keeps only its SHA-256. */
  authKey: Bytes;
  /** Encrypts the backup; never leaves the browser. */
  encKey: CryptoKey;
}

export async function derivePinKeys(pin: string, salt: Bytes, iterations: number): Promise<PinKeys> {
  const base = await crypto.subtle.importKey("raw", utf8(pin), "PBKDF2", false, ["deriveBits"]);
  const master = await crypto.subtle.deriveBits(
    { name: "PBKDF2", hash: "SHA-256", salt, iterations },
    base,
    256,
  );
  const expand = await crypto.subtle.importKey("raw", master, "HKDF", false, ["deriveBits", "deriveKey"]);
  const authKey = bytesOf(
    await crypto.subtle.deriveBits(
      { name: "HKDF", hash: "SHA-256", salt: HKDF_SALT, info: utf8(AUTH_INFO) },
      expand,
      256,
    ),
  );
  const encKey = await crypto.subtle.deriveKey(
    { name: "HKDF", hash: "SHA-256", salt: HKDF_SALT, info: utf8(ENC_INFO) },
    expand,
    { name: "AES-GCM", length: 256 },
    false,
    ["encrypt", "decrypt"],
  );
  return { authKey, encKey };
}

export async function verifierOf(authKey: Bytes): Promise<Bytes> {
  return bytesOf(await crypto.subtle.digest("SHA-256", authKey));
}

/** The PIN-protected copy of the account private key, as the server stores it. */
export interface SealedBackup {
  salt: Bytes;
  iterations: number;
  verifier: Bytes;
  iv: Bytes;
  ciphertext: Bytes;
}

export async function sealBackup(
  accountPkcs8: Bytes,
  pin: string,
  iterations = PIN_ITERATIONS,
): Promise<SealedBackup> {
  const salt = randomBytes(SALT_LENGTH);
  const { authKey, encKey } = await derivePinKeys(pin, salt, iterations);
  const iv = randomBytes(12);
  const ciphertext = bytesOf(await crypto.subtle.encrypt({ name: "AES-GCM", iv }, encKey, accountPkcs8));
  return { salt, iterations, verifier: await verifierOf(authKey), iv, ciphertext };
}

/** The backup's contents; throws when the key is not the one it was sealed with. */
export async function openBackup(encKey: CryptoKey, iv: Bytes, ciphertext: Bytes): Promise<Bytes> {
  return bytesOf(await crypto.subtle.decrypt({ name: "AES-GCM", iv }, encKey, ciphertext));
}
