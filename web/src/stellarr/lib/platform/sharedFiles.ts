import { openDatabase, requestResult, transactionDone } from "@/lib/chatTranslation/idb";

// Shared with receiveShare() in the root service worker.
interface Incoming {
  owner: string | null;
  expires: number;
  files: File[];
}

async function transaction<T>(action: (store: IDBObjectStore) => Promise<T>): Promise<T> {
  const db = await openDatabase("stellarr-incoming-shares", 1, db => db.createObjectStore("shares"));
  try {
    const tx = db.transaction("shares", "readwrite");
    const [result] = await Promise.all([action(tx.objectStore("shares")), transactionDone(tx)]);
    return result;
  } finally {
    db.close();
  }
}

/** Preserve a signed-out handoff through login, drop files belonging to an ended session. */
export function setShareOwner(owner: string | null) {
  return transaction(async store => {
    const ids = await requestResult(store.getAllKeys());
    for (const id of ids) {
      if (id === "owner") continue;
      const batch: Incoming = await requestResult(store.get(id));
      if (batch.expires <= Date.now() || (batch.owner !== null && batch.owner !== owner)) store.delete(id);
    }
    store.put(owner, "owner");
  });
}

/** A single read/write transaction lets only one window take the files. */
export function sharedFiles(id: string, user: string, take = false): Promise<File[]> {
  if (!/^[a-f\d-]{36}$/i.test(id)) return Promise.resolve([]);
  return transaction(async store => {
    const batch: Incoming | undefined = await requestResult(store.get(id));
    if (!batch || (batch.owner !== null && batch.owner !== user)) return [];
    if (batch.expires <= Date.now()) {
      store.delete(id);
      return [];
    }
    if (take) store.delete(id);
    else if (batch.owner === null) store.put({ ...batch, owner: user }, id);
    return batch.files;
  });
}
