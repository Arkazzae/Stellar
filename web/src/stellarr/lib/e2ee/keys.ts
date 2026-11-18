import { bytesOf, fromBase64Url, randomBytes, sameBytes, type Bytes } from "./bytes";

/** The one key algorithm today; the server stores this tag with every key. */
export const KEY_ALGORITHM = "x25519";

const X25519 = { name: "X25519" } as const;

export async function generateKeyPair(extractable: boolean): Promise<CryptoKeyPair> {
  return (await crypto.subtle.generateKey(X25519, extractable, ["deriveBits"])) as CryptoKeyPair;
}

/** The raw 32 bytes the server and the other browser see. */
export async function exportPublicKey(key: CryptoKey): Promise<Bytes> {
  return bytesOf(await crypto.subtle.exportKey("raw", key));
}

export function importPublicKey(raw: Bytes): Promise<CryptoKey> {
  return crypto.subtle.importKey("raw", raw, X25519, true, []);
}

export function importPrivateKey(pkcs8: Bytes, extractable: boolean): Promise<CryptoKey> {
  return crypto.subtle.importKey("pkcs8", pkcs8, X25519, extractable, ["deriveBits"]);
}

export async function exportPrivateKey(key: CryptoKey): Promise<Bytes> {
  return bytesOf(await crypto.subtle.exportKey("pkcs8", key));
}

/** 32 shared bytes between a private key and someone's public key. */
export async function agree(privateKey: CryptoKey, publicKey: CryptoKey): Promise<ArrayBuffer> {
  return crypto.subtle.deriveBits({ name: "X25519", public: publicKey }, privateKey, 256);
}

/**
 * Whether a private key (PKCS#8) belongs to this public key.
 *
 * A private JWK carries its public half as `x` (base64url); the server's copy
 * is raw bytes, so the comparison is on bytes. A backup that decrypts but
 * holds another key is refused by the caller, not used.
 */
export async function privateKeyMatches(pkcs8: Bytes, publicKey: Bytes): Promise<boolean> {
  let jwk: JsonWebKey;
  try {
    jwk = await crypto.subtle.exportKey("jwk", await importPrivateKey(pkcs8, true));
  } catch {
    return false;
  }
  return typeof jwk.x === "string" && sameBytes(fromBase64Url(jwk.x), publicKey);
}

/**
 * The account private key as this browser keeps it: encrypted by a key of the
 * browser's own that can never leave it, so a copy of the database alone does
 * not give the account key away, while the browser can still read it back to
 * write a new backup without the old PIN.
 */
export interface LocallySealedKey {
  wrappingKey: CryptoKey;
  iv: Bytes;
  sealed: Bytes;
}

export async function sealLocally(pkcs8: Bytes): Promise<LocallySealedKey> {
  const wrappingKey = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, [
    "encrypt",
    "decrypt",
  ]);
  const iv = randomBytes(12);
  const sealed = bytesOf(await crypto.subtle.encrypt({ name: "AES-GCM", iv }, wrappingKey, pkcs8));
  return { wrappingKey, iv, sealed };
}

export async function openLocally(key: LocallySealedKey): Promise<Bytes> {
  return bytesOf(await crypto.subtle.decrypt({ name: "AES-GCM", iv: key.iv }, key.wrappingKey, key.sealed));
}
