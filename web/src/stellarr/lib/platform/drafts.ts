import { openDatabase, requestResult, transactionDone } from "@/lib/chatTranslation/idb";
import { fromUtf8, randomBytes, utf8 } from "@/lib/e2ee/bytes";
import type { CodeLanguageId } from "@/lib/messages/codeLanguages";

export interface Draft {
  text: string;
  manualCode: boolean;
  codeLanguage: CodeLanguageId | "auto";
  /**
   * When the message is to go out, ISO 8601, if a time was chosen. Kept with
   * the words, so leaving the channel and coming back does not turn a
   * scheduled message into one that sends at once.
   */
  sendAt?: string;
}
const empty: Draft = { text: "", manualCode: false, codeLanguage: "auto" };
export function draftScope(account: string, ...destination: string[]) {
  return JSON.stringify([account, ...destination]);
}
let opening: Promise<IDBDatabase> | undefined;
function database() {
  opening ??= openDatabase("stellarr-composer-drafts", 2, db => {
    for (const store of ["keys", "drafts", "files", "fileOrder"]) {
      if (!db.objectStoreNames.contains(store)) db.createObjectStore(store);
    }
  }).then(db => {
    db.onversionchange = () => { db.close(); opening = undefined; };
    return db;
  }).catch(error => { opening = undefined; throw error; });
  return opening;
}

// Serialize edits, restores and sign-out. A queued older edit can never land
// after the delete for a delivered message.
let pending: Promise<unknown> = Promise.resolve();
function ordered<T>(work: () => Promise<T>): Promise<T> {
  const result = pending.then(work, work);
  pending = result.catch(() => undefined);
  return result;
}

async function keyFor(db: IDBDatabase, user: string): Promise<CryptoKey> {
  const held = await requestResult<CryptoKey | undefined>(db.transaction("keys").objectStore("keys").get(user));
  if (held) return held;
  const candidate = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]);
  // Two first edits in separate tabs must agree on a single non-extractable key.
  const tx = db.transaction("keys", "readwrite");
  const store = tx.objectStore("keys");
  const current = await requestResult<CryptoKey | undefined>(store.get(user));
  if (!current) store.put(candidate, user);
  await transactionDone(tx);
  return current ?? candidate;
}

async function read(scope: string): Promise<Draft> {
  const db = await database();
  const stored = await requestResult(db.transaction("drafts").objectStore("drafts").get(scope));
  if (!stored) return empty;
  const key = await keyFor(db, JSON.parse(scope)[0]);
  const clear = await crypto.subtle.decrypt({ name: "AES-GCM", iv: stored.iv, additionalData: utf8(scope) }, key, stored.body);
  return JSON.parse(fromUtf8(clear)) as Draft;
}

async function write(scope: string, draft: Draft) {
  const db = await database();
  let record;
  if (draft.text) {
    const key = await keyFor(db, JSON.parse(scope)[0]);
    const iv = randomBytes(12);
    const body = await crypto.subtle.encrypt({ name: "AES-GCM", iv, additionalData: utf8(scope) }, key, utf8(JSON.stringify(draft)));
    record = { iv, body };
  }
  const tx = db.transaction("drafts", "readwrite");
  const store = tx.objectStore("drafts");
  if (record) store.put(record, scope);
  else store.delete(scope);
  await transactionDone(tx);
}

export interface DraftFile { id: string; file: File }
interface SealedFile { iv: Uint8Array<ArrayBuffer>; body: ArrayBuffer }

export function readDraftFiles(scope: string): Promise<DraftFile[]> {
  return ordered(async () => {
    const db = await database();
    const tx = db.transaction(["fileOrder", "files"]);
    const order: string[] = await requestResult(tx.objectStore("fileOrder").get(scope)) ?? [];
    const records: SealedFile[] = await Promise.all(order.map(id => requestResult(tx.objectStore("files").get([scope, id]))));
    if (!order.length) return [];
    const key = await keyFor(db, JSON.parse(scope)[0]);
    const files: DraftFile[] = [];
    for (const [index, record] of records.entries()) {
      const id = order[index]!;
      const bytes = new Uint8Array(await crypto.subtle.decrypt(
        { name: "AES-GCM", iv: record.iv, additionalData: utf8(scope + id) }, key, record.body));
      const line = bytes.indexOf(10);
      const [name, type, lastModified] = JSON.parse(fromUtf8(bytes.subarray(0, line)));
      files.push({ id, file: new File([bytes.subarray(line + 1)], name, { type, lastModified }) });
    }
    return files;
  });
}

/** Encrypt new originals once. Upload progress does not write or copy file bytes. */
export function writeDraftFiles(scope: string, before: DraftFile[], after: DraftFile[]) {
  const oldIds = new Set(before.map(item => item.id));
  const ids = after.map(item => item.id);
  const added = after.filter(item => !oldIds.has(item.id));
  return ordered(async () => {
    const db = await database();
    const sealed: [string, SealedFile][] = [];
    if (added.length) {
      const key = await keyFor(db, JSON.parse(scope)[0]);
      for (const { id, file } of added) {
        const iv = randomBytes(12);
        const header = JSON.stringify([file.name, file.type, file.lastModified]) + "\n";
        const bytes = await new Blob([header, file]).arrayBuffer();
        const body = await crypto.subtle.encrypt({ name: "AES-GCM", iv, additionalData: utf8(scope + id) }, key, bytes);
        sealed.push([id, { iv, body }]);
      }
    }
    const tx = db.transaction(["fileOrder", "files"], "readwrite");
    const files = tx.objectStore("files");
    const orders = tx.objectStore("fileOrder");
    const existing: string[] = await requestResult(orders.get(scope)) ?? [];
    for (const id of oldIds) if (!ids.includes(id)) files.delete([scope, id]);
    for (const [id, record] of sealed) files.put(record, [scope, id]);
    // Keep another window's additions and never resurrect its deletions.
    const order = [...existing.filter(id => !oldIds.has(id) && !ids.includes(id)),
      ...ids.filter(id => !oldIds.has(id) || existing.includes(id))];
    if (order.length) orders.put(order, scope);
    else orders.delete(scope);
    await transactionDone(tx);
  });
}

const entries = new Map<string, ReturnType<typeof createEntry>>();
function createEntry(scope?: string) {
  let snapshot = { draft: empty, loading: !!scope, failed: false };
  const listeners = new Set<() => void>();
  let started = false;
  let version = 0;
  const announce = () => listeners.forEach(listener => listener());
  const live = () => !scope || entries.get(scope) === entry;
  const entry = {
    get: () => snapshot,
    storageFailed() { snapshot = { ...snapshot, failed: true }; announce(); },
    subscribe(listener: () => void) {
      listeners.add(listener);
      if (scope && !started) {
        started = true;
        void ordered(() => read(scope)).then(
          draft => { if (version === 0) snapshot = { draft, loading: false, failed: false }; },
          () => { snapshot = { ...snapshot, loading: false, failed: true }; },
        ).then(announce);
      }
      return () => { listeners.delete(listener); };
    },
    change(update: (draft: Draft) => Draft) {
      if (!live()) return;
      const draft = update(snapshot.draft);
      const revision = ++version;
      snapshot = { draft, loading: false, failed: false };
      announce();
      if (scope) {
        void ordered(async () => {
          if (revision === version && live()) await write(scope, draft);
        }).catch(() => {
          if (revision !== version) return;
          snapshot = { ...snapshot, failed: true };
          announce();
        });
      }
    },
  };
  return entry;
}

export function composerDraft(scope?: string) {
  if (!scope) return createEntry();
  let entry = entries.get(scope);
  if (!entry) {
    entry = createEntry(scope);
    entries.set(scope, entry);
  }
  return entry;
}

export function forgetServerDrafts(server: string) {
  const belongs = (scope: unknown) => typeof scope === "string" && JSON.parse(scope)[1] === server;
  for (const scope of entries.keys()) if (belongs(scope)) entries.delete(scope);
  return ordered(async () => {
    const db = await database();
    const tx = db.transaction(["drafts", "fileOrder", "files"], "readwrite");
    const done = transactionDone(tx);
    await Promise.all(["drafts", "fileOrder", "files"].map(async name => {
      const store = tx.objectStore(name);
      for (const key of await requestResult(store.getAllKeys())) {
        if (belongs(Array.isArray(key) ? key[0] : key)) store.delete(key);
      }
    }));
    await done;
  });
}

export function clearDrafts() {
  entries.clear();
  return ordered(async () => {
    const db = await database();
    const tx = db.transaction(["keys", "drafts", "files", "fileOrder"], "readwrite");
    tx.objectStore("drafts").clear();
    tx.objectStore("keys").clear();
    tx.objectStore("files").clear();
    tx.objectStore("fileOrder").clear();
    await transactionDone(tx);
  });
}
