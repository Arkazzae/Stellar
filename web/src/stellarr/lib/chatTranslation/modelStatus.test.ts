import "fake-indexeddb/auto";

import { createHash } from "node:crypto";

import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  downloadModel,
  getModelStatus,
  getPairSize,
  primaryPair,
} from "./modelStatus";

const model = new Uint8Array([1, 2, 3, 4, 5]);
const lex = new Uint8Array([6, 7, 8]);
const vocab = new Uint8Array([9, 10, 11, 12]);
const payloads: Record<string, Uint8Array> = { "model.bin": model, "lex.bin": lex, "vocab.spm": vocab };

const sha256 = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const listed = (name: string, bytes: Uint8Array) => ({ name, size: bytes.byteLength, sha256: sha256(bytes) });
const files = { model: listed("model.bin", model), lex: listed("lex.bin", lex), vocab: listed("vocab.spm", vocab) };

/** What the stand-in server sends for a file name, when it differs from `payloads`. */
let served: Record<string, Uint8Array> = {};

function removeModelDatabase(): Promise<void> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.deleteDatabase("bergamot-models");
    request.onsuccess = () => resolve();
    request.onerror = () => reject(request.error);
  });
}

function withStore<T>(mode: IDBTransactionMode, ask: (store: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    const open = indexedDB.open("bergamot-models", 1);
    open.onupgradeneeded = () => open.result.createObjectStore("files");
    open.onerror = () => reject(open.error);
    open.onsuccess = () => {
      const database = open.result;
      const request = ask(database.transaction("files", mode).objectStore("files"));
      request.onsuccess = () => {
        database.close();
        resolve(request.result);
      };
      request.onerror = () => reject(request.error);
    };
  });
}

const storeFile = (key: string, bytes: Uint8Array) =>
  withStore("readwrite", (store) => store.put(bytes.slice().buffer, key));
const storedKeys = () => withStore("readonly", (store) => store.getAllKeys());

describe("Bergamot model loading", () => {
  beforeEach(async () => {
    await removeModelDatabase();
    served = {};

    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url === "/bergamot-models/registry.json") {
          return Response.json({ plen: files, enpl: files });
        }

        const name = url.split("/").at(-1) ?? "";
        const payload = served[name] ?? payloads[name];
        return payload === undefined
          ? new Response(null, { status: 404 })
          : new Response(payload.slice().buffer as ArrayBuffer, { status: 200 });
      }),
    );
  });

  it("maps both targets to a real direct model", () => {
    expect(primaryPair("en")).toBe("plen");
    expect(primaryPair("pl")).toBe("enpl");
  });

  it("streams a missing model into IndexedDB and reuses it without network", async () => {
    const progress: number[] = [];

    expect(await getModelStatus("en")).toBe("not-downloaded");
    expect(await getPairSize("en")).toBe(12);

    await downloadModel("en", (value) => progress.push(value));

    expect(progress.at(-1)).toBe(100);
    expect(progress.some((value) => value > 0 && value < 100)).toBe(true);
    expect(await getModelStatus("en")).toBe("ready");

    const fetchMock = vi.mocked(fetch);
    const callsAfterFirstDownload = fetchMock.mock.calls.length;
    await downloadModel("en");

    expect(fetchMock).toHaveBeenCalledWith("/bergamot-models/plen/model.bin");
    expect(fetchMock).toHaveBeenCalledWith("/bergamot-models/plen/lex.bin");
    expect(fetchMock).toHaveBeenCalledWith("/bergamot-models/plen/vocab.spm");
    expect(fetchMock.mock.calls).toHaveLength(callsAfterFirstDownload);
  });

  it("does not use a stored file the registry no longer lists, and removes it", async () => {
    // Files an earlier release left: one under the name it was stored by, one
    // under the hash of a model this registry has replaced.
    await storeFile("plen/model.bin", model);
    await storeFile(sha256(new Uint8Array([0])), new Uint8Array([0]));

    expect(await getModelStatus("en")).toBe("not-downloaded");

    await downloadModel("en");

    expect(vi.mocked(fetch)).toHaveBeenCalledWith("/bergamot-models/plen/model.bin");
    expect((await storedKeys()).sort()).toEqual(Object.values(files).map((file) => file.sha256).sort());
  });

  it("refuses a download whose bytes are not the ones the registry lists", async () => {
    served = { "lex.bin": new Uint8Array([6, 7, 9]) };

    await expect(downloadModel("en")).rejects.toThrow("lex.bin");

    expect(await storedKeys()).not.toContain(files.lex.sha256);
    expect(await getModelStatus("en")).toBe("not-downloaded");
  });
});
