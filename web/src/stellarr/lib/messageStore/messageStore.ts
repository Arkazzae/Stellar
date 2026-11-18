import { openDatabase, requestResult, transactionDone } from "@/lib/chatTranslation/idb";
import type { Message } from "@/ports";

/**
 * The newest messages of the channels read most recently, kept on the device so
 * that a channel opens with its transcript already on screen while the server
 * brings it up to date.
 *
 * Only messages of server channels go in here. A direct message, an encrypted
 * one above all, never does: the one place that writes (`write`) refuses any
 * message that carries encryption traces and any channel that is not a text
 * channel of the open server, and the code that reads direct messages does not
 * import this module at all.
 */

/** How many of a channel's newest messages are kept. */
export const WINDOW_LIMIT = 100;
/** How many channels are kept; the one written longest ago goes first. */
export const CHANNEL_LIMIT = 50;

const DB_NAME = "stellarr-message-store";
/**
 * The stored format. There is no migration: a different number deletes every
 * store the old one made and starts empty (see `createStores`).
 */
const DB_VERSION = 1;
const WINDOW_STORE = "windows";
const META = "meta";
const BY_WRITTEN = "writtenAt";
const BY_SERVER = "serverId";
const OWNER = "owner";

/**
 * A channel's newest messages together with the journal cursor they are
 * current as of. They live in one record, so one transaction writes both: a
 * cursor newer than its messages would skip the changes in between for good.
 */
export interface StoredWindow {
  messages: Message[];
  cursor: string;
}

interface WindowRow extends StoredWindow {
  key: string;
  serverId: string;
  channelId: string;
  /** Time of the write; the trimming to `CHANNEL_LIMIT` goes by it. */
  writtenAt: number;
}

/** Whose store this is and which server it is read for. */
export interface StoreScope {
  userId: string;
  serverId: string;
}

export interface MessageStore {
  /**
   * Declares the text channels of the open server and deletes the stored
   * windows of every other channel of it: one that no longer exists, or that
   * this account can no longer read.
   */
  keepOnly: (channelIds: readonly string[]) => Promise<void>;
  read: (channelId: string) => Promise<StoredWindow | undefined>;
  /**
   * Replaces the channel's stored window with the newest `WINDOW_LIMIT` of
   * `messages`, leaving out those the server has not confirmed (`pendingIds`).
   * When any of them is one that must not reach the disk, nothing is kept for
   * the channel.
   */
  write: (
    channelId: string,
    messages: readonly Message[],
    cursor: string,
    pendingIds: ReadonlySet<string>,
  ) => Promise<void>;
}

/** Bumped by `wipeAll`, so a store created before it can no longer write. */
let generation = 0;

const rowKey = (serverId: string, channelId: string) => `${serverId}/${channelId}`;

const available = () => typeof indexedDB !== "undefined";

/**
 * Whether a message may be written to the disk. The plaintext of an end-to-end
 * encrypted message, the entry standing for one this browser cannot read and
 * the lines about a conversation's encryption all belong to direct messages.
 */
export function isStorable(message: Message): boolean {
  return (
    message.encrypted === undefined &&
    message.decrypted !== true &&
    message.sealed === undefined &&
    message.line === undefined
  );
}

/**
 * The part of a bucket that is known to follow one another with no gap, oldest
 * first: the messages from `floor` upward. A search hit or an opened thread can
 * put a single old message into a bucket, and that one must not be stored as if
 * the history below the newest page reached it. `floor` is null when the run
 * reaches the beginning of the channel.
 */
export function contiguousRun(
  bucket: readonly Message[],
  floor: Pick<Message, "id" | "createdAt"> | null,
): Message[] {
  if (floor === null) return [...bucket];
  const floorTime = Date.parse(floor.createdAt);
  return bucket.filter((message) => {
    const time = Date.parse(message.createdAt);
    return time > floorTime || (time === floorTime && message.id >= floor.id);
  });
}

/** The stores of the format this code writes, and none of any other. */
function createStores(database: IDBDatabase): void {
  for (const name of Array.from(database.objectStoreNames)) database.deleteObjectStore(name);
  const windows = database.createObjectStore(WINDOW_STORE, { keyPath: "key" });
  windows.createIndex(BY_WRITTEN, BY_WRITTEN);
  windows.createIndex(BY_SERVER, BY_SERVER);
  database.createObjectStore(META, { keyPath: "key" });
}

async function withDatabase<T>(work: (database: IDBDatabase) => Promise<T>): Promise<T> {
  const database = await openDatabase(DB_NAME, DB_VERSION, createStores);
  try {
    return await work(database);
  } finally {
    database.close();
  }
}

/** Deletes, in the caller's transaction, the rows of one server that `keep` refuses. */
function deleteUnless(
  windows: IDBObjectStore,
  serverId: string,
  keep: (row: WindowRow) => boolean,
): void {
  const request = windows.index(BY_SERVER).openCursor(IDBKeyRange.only(serverId));
  request.onsuccess = () => {
    const cursor = request.result;
    if (cursor === null) return;
    if (!keep(cursor.value as WindowRow)) cursor.delete();
    cursor.continue();
  };
}

/** Deletes the longest-ago written rows beyond `CHANNEL_LIMIT`, in the caller's transaction. */
function trim(windows: IDBObjectStore): void {
  const counted = windows.count();
  counted.onsuccess = () => {
    let excess = counted.result - CHANNEL_LIMIT;
    if (excess <= 0) return;
    const request = windows.index(BY_WRITTEN).openCursor();
    request.onsuccess = () => {
      const cursor = request.result;
      if (cursor === null || excess === 0) return;
      cursor.delete();
      excess -= 1;
      cursor.continue();
    };
  };
}

export function createMessageStore({ userId, serverId }: StoreScope): MessageStore {
  const born = generation;
  let channels = new Set<string>();

  // The first thing every operation waits for: a store that belongs to another
  // account is emptied before it can be read or written.
  let claimed: Promise<void> | undefined;
  const claim = (): Promise<void> => {
    claimed ??= withDatabase(async (database) => {
      const transaction = database.transaction([WINDOW_STORE, META], "readwrite");
      const meta = transaction.objectStore(META);
      const request = meta.get(OWNER);
      request.onsuccess = () => {
        const owner = request.result as { userId: string } | undefined;
        if (owner?.userId === userId) return;
        transaction.objectStore(WINDOW_STORE).clear();
        meta.put({ key: OWNER, userId });
      };
      await transactionDone(transaction);
    });
    claimed.catch(() => {
      claimed = undefined;
    });
    return claimed;
  };

  // Every failure is the same answer: the device cannot keep messages, and the
  // app goes on as if there were no store.
  const guarded = async <T>(work: () => Promise<T>, otherwise: T): Promise<T> => {
    if (!available()) return otherwise;
    try {
      await claim();
      return await work();
    } catch {
      return otherwise;
    }
  };

  return {
    keepOnly: (channelIds) => {
      channels = new Set(channelIds);
      return guarded(
        () =>
          withDatabase(async (database) => {
            const transaction = database.transaction(WINDOW_STORE, "readwrite");
            deleteUnless(transaction.objectStore(WINDOW_STORE), serverId, (row) =>
              channels.has(row.channelId));
            await transactionDone(transaction);
          }),
        undefined,
      );
    },

    read: (channelId) =>
      guarded(
        () =>
          withDatabase(async (database) => {
            const request = database
              .transaction(WINDOW_STORE, "readonly")
              .objectStore(WINDOW_STORE)
              .get(rowKey(serverId, channelId));
            const row = (await requestResult(request)) as WindowRow | undefined;
            if (
              row === undefined ||
              typeof row.cursor !== "string" ||
              !Array.isArray(row.messages) ||
              !row.messages.every(isStorable)
            ) {
              return undefined;
            }
            return { messages: row.messages, cursor: row.cursor };
          }),
        undefined,
      ),

    write: (channelId, messages, cursor, pendingIds) => {
      if (generation !== born || !channels.has(channelId)) return Promise.resolve();
      const confirmed = messages.filter((message) => !pendingIds.has(message.id));
      return guarded(
        () =>
          withDatabase(async (database) => {
            const transaction = database.transaction(WINDOW_STORE, "readwrite");
            const windows = transaction.objectStore(WINDOW_STORE);
            const key = rowKey(serverId, channelId);
            if (confirmed.every(isStorable)) {
              const row: WindowRow = {
                key,
                serverId,
                channelId,
                messages: confirmed.slice(-WINDOW_LIMIT),
                cursor,
                writtenAt: Date.now(),
              };
              windows.put(row);
              trim(windows);
            } else {
              windows.delete(key);
            }
            await transactionDone(transaction);
          }),
        undefined,
      );
    },
  };
}

/** Forgets the stored windows of one server, such as one this account was shut out of. */
export async function wipeServer(serverId: string): Promise<void> {
  if (!available()) return;
  try {
    await withDatabase(async (database) => {
      const transaction = database.transaction(WINDOW_STORE, "readwrite");
      deleteUnless(transaction.objectStore(WINDOW_STORE), serverId, () => false);
      await transactionDone(transaction);
    });
  } catch {
    // Nothing more to do; the windows are replaced the next time they are written.
  }
}

/** Forgets everything stored, for good: signing out calls it. */
export async function wipeAll(): Promise<void> {
  generation += 1;
  if (!available()) return;
  await new Promise<void>((resolve) => {
    const request = indexedDB.deleteDatabase(DB_NAME);
    // Blocked means a connection is still open; the deletion finishes once it closes.
    request.onsuccess = () => resolve();
    request.onerror = () => resolve();
    request.onblocked = () => resolve();
  });
}
