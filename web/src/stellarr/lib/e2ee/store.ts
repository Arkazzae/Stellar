import type { Bytes } from "./bytes";
import type { LocallySealedKey } from "./keys";

/**
 * What this browser keeps for encryption, per signed-in account.
 *
 * One IndexedDB database, `stellarr-e2ee`, with three object stores, each
 * keyed by the account's user id so two accounts on one browser profile never
 * share a key:
 *
 * - `device`: this browser's device key pair. The private key is a
 *   non-extractable `CryptoKey`; IndexedDB stores the handle, and its bytes
 *   cannot be read back out by any script.
 * - `account`: the account private key, sealed by a non-extractable AES-GCM
 *   key of this browser's own (`LocallySealedKey`), with the key id and public
 *   key it belongs to.
 * - `verified`: per peer, the safety code the person confirmed and when.
 *
 * Signing out deletes the first two; the third holds no secret.
 */

export interface StoredDevice {
  userId: string;
  privateKey: CryptoKey;
  publicKey: Bytes;
}

export interface StoredAccountKey extends LocallySealedKey {
  userId: string;
  keyId: string;
  publicKey: Bytes;
}

export interface StoredVerification {
  userId: string;
  peerId: string;
  /** The code that was compared, groups joined by spaces. */
  code: string;
  /** ISO 8601. */
  at: string;
}

export interface KeyStore {
  getDevice(userId: string): Promise<StoredDevice | null>;
  putDevice(device: StoredDevice): Promise<void>;
  getAccountKey(userId: string): Promise<StoredAccountKey | null>;
  putAccountKey(key: StoredAccountKey): Promise<void>;
  deleteAccountKey(userId: string): Promise<void>;
  getVerification(userId: string, peerId: string): Promise<StoredVerification | null>;
  putVerification(verification: StoredVerification): Promise<void>;
  deleteVerification(userId: string, peerId: string): Promise<void>;
  /** Deletes the device key and the account key copy of this account. */
  forget(userId: string): Promise<void>;
}

const DATABASE = "stellarr-e2ee";
const VERSION = 1;
type StoreName = "device" | "account" | "verified";

function promised<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("IndexedDB request failed"));
  });
}

function done(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => reject(transaction.error ?? new Error("IndexedDB transaction failed"));
    transaction.onabort = () => reject(transaction.error ?? new Error("IndexedDB transaction aborted"));
  });
}

const verificationKey = (userId: string, peerId: string) => `${userId}:${peerId}`;

export function createIndexedDbKeyStore(name = DATABASE): KeyStore {
  let opening: Promise<IDBDatabase> | null = null;

  const open = (): Promise<IDBDatabase> => {
    opening ??= new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open(name, VERSION);
      request.onupgradeneeded = () => {
        const db = request.result;
        if (!db.objectStoreNames.contains("device")) db.createObjectStore("device", { keyPath: "userId" });
        if (!db.objectStoreNames.contains("account")) db.createObjectStore("account", { keyPath: "userId" });
        if (!db.objectStoreNames.contains("verified")) db.createObjectStore("verified");
      };
      request.onsuccess = () => {
        const db = request.result;
        // Another tab upgrading later closes this handle; the next call reopens.
        db.onversionchange = () => {
          db.close();
          opening = null;
        };
        resolve(db);
      };
      request.onerror = () => {
        opening = null;
        reject(request.error ?? new Error("IndexedDB could not be opened"));
      };
    });
    return opening;
  };

  const read = async <T>(store: StoreName, key: IDBValidKey): Promise<T | null> => {
    const db = await open();
    const value = (await promised(db.transaction(store).objectStore(store).get(key))) as T | undefined;
    return value ?? null;
  };

  const write = async (store: StoreName, change: (target: IDBObjectStore) => void): Promise<void> => {
    const db = await open();
    const transaction = db.transaction(store, "readwrite");
    change(transaction.objectStore(store));
    await done(transaction);
  };

  return {
    getDevice: (userId) => read<StoredDevice>("device", userId),
    putDevice: (device) => write("device", (store) => store.put(device)),
    getAccountKey: (userId) => read<StoredAccountKey>("account", userId),
    putAccountKey: (key) => write("account", (store) => store.put(key)),
    deleteAccountKey: (userId) => write("account", (store) => store.delete(userId)),
    getVerification: (userId, peerId) => read<StoredVerification>("verified", verificationKey(userId, peerId)),
    putVerification: (verification) =>
      write("verified", (store) => store.put(verification, verificationKey(verification.userId, verification.peerId))),
    deleteVerification: (userId, peerId) =>
      write("verified", (store) => store.delete(verificationKey(userId, peerId))),
    async forget(userId) {
      const db = await open();
      const transaction = db.transaction(["device", "account"], "readwrite");
      transaction.objectStore("device").delete(userId);
      transaction.objectStore("account").delete(userId);
      await done(transaction);
    },
  };
}

/** The same contract in memory, for tests and for a browser without IndexedDB. */
export function createMemoryKeyStore(): KeyStore {
  const devices = new Map<string, StoredDevice>();
  const accounts = new Map<string, StoredAccountKey>();
  const verified = new Map<string, StoredVerification>();
  return {
    getDevice: async (userId) => devices.get(userId) ?? null,
    putDevice: async (device) => void devices.set(device.userId, device),
    getAccountKey: async (userId) => accounts.get(userId) ?? null,
    putAccountKey: async (key) => void accounts.set(key.userId, key),
    deleteAccountKey: async (userId) => void accounts.delete(userId),
    getVerification: async (userId, peerId) => verified.get(verificationKey(userId, peerId)) ?? null,
    putVerification: async (verification) =>
      void verified.set(verificationKey(verification.userId, verification.peerId), verification),
    deleteVerification: async (userId, peerId) => void verified.delete(verificationKey(userId, peerId)),
    async forget(userId) {
      devices.delete(userId);
      accounts.delete(userId);
    },
  };
}
