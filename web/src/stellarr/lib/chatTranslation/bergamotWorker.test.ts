import "fake-indexeddb/auto";

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { runInThisContext } from "node:vm";

import { beforeEach, describe, expect, it, vi } from "vitest";

// public/bergamot-worker.js is a classic worker script. It runs here as it is,
// with fake-indexeddb for IndexedDB and a stand-in for the engine.

interface Reply {
  type: "result" | "error";
  id: number;
  translation?: string;
  message?: string;
}

interface WorkerScope {
  onmessage: ((event: { data: unknown }) => Promise<void>) | null;
  postMessage: (reply: Reply) => void;
}

type Engine = Record<string, unknown> & { onRuntimeInitialized: () => void };

const workerSource = readFileSync(join(process.cwd(), "public/bergamot-worker.js"), "utf8");

const model = new Uint8Array([1, 2, 3, 4, 5]);
const lex = new Uint8Array([6, 7, 8]);
const vocab = new Uint8Array([9, 10, 11, 12]);
const payloads: Record<string, Uint8Array> = { "model.bin": model, "lex.bin": lex, "vocab.spm": vocab };

const sha256 = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const listed = (name: string, bytes: Uint8Array) => ({ name, size: bytes.byteLength, sha256: sha256(bytes) });
const files = { model: listed("model.bin", model), lex: listed("lex.bin", lex), vocab: listed("vocab.spm", vocab) };

/** What the stand-in server sends for a file name, when it differs from `payloads`. */
let served: Record<string, Uint8Array> = {};

/**
 * The engine's classes, as far as the worker uses them. A text it has an entry
 * for in `translations` translates to that entry; any other comes back as it is.
 */
function standInEngine(translations: Record<string, string> = {}) {
  const loadedModels: number[][] = [];

  class AlignedMemory {
    readonly bytes: Uint8Array;
    constructor(size: number) {
      this.bytes = new Uint8Array(size);
    }
    getByteArrayView() {
      return this.bytes;
    }
    delete() {}
  }
  class Vector<T> {
    readonly items: T[] = [];
    push_back(item: T) {
      this.items.push(item);
    }
    delete() {}
  }
  class TranslationModel {
    constructor(_from: string, _to: string, _config: string, memory: AlignedMemory) {
      loadedModels.push(Array.from(memory.bytes));
    }
  }
  class BlockingService {
    translate(_model: TranslationModel, input: Vector<string>) {
      const text = input.items[0] ?? "";
      const translated = translations[text] ?? text;
      return { get: () => ({ getTranslatedText: () => translated, delete() {} }), delete() {} };
    }
  }

  const classes = {
    AlignedMemory,
    AlignedMemoryList: Vector,
    VectorString: Vector,
    VectorResponseOptions: Vector,
    TranslationModel,
    BlockingService,
  };
  return { classes, loadedModels };
}

function startWorker(engine: ReturnType<typeof standInEngine>) {
  let deliver: (reply: Reply) => void = () => {};
  const scope: WorkerScope = { onmessage: null, postMessage: (reply) => deliver(reply) };
  const loadBergamot = (module: Engine) => {
    Object.assign(module, engine.classes);
    queueMicrotask(() => module.onRuntimeInitialized());
  };
  const run = runInThisContext(`(function (self, importScripts, loadBergamot) {\n${workerSource}\n})`) as (
    scope: WorkerScope,
    importScripts: () => void,
    loadBergamot: (module: Engine) => void,
  ) => void;
  run(scope, () => {}, loadBergamot);

  return (text: string, from = "pl", to = "en") =>
    new Promise<Reply>((resolve) => {
      deliver = resolve;
      void scope.onmessage?.({ data: { type: "translate", id: 1, from, to, text } });
    });
}

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

describe("the translation worker", () => {
  beforeEach(async () => {
    await removeModelDatabase();
    served = {};

    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url === "/bergamot-models/registry.json") return Response.json({ plen: files, enpl: files });

        const name = url.split("/").at(-1) ?? "";
        const payload = served[name] ?? payloads[name];
        return payload === undefined
          ? new Response(null, { status: 404 })
          : new Response(payload.slice().buffer as ArrayBuffer, { status: 200 });
      }),
    );
  });

  it("uses only the files stored under the hashes the registry lists, and removes the others", async () => {
    // Left by an earlier release: a model under the name it was once stored
    // by, and a file the registry no longer lists.
    await storeFile("plen/model.bin", new Uint8Array([0, 0, 0, 0, 0]));
    await storeFile(sha256(new Uint8Array([7])), new Uint8Array([7]));
    const engine = standInEngine();

    expect(await startWorker(engine)("Nie wiem")).toMatchObject({ type: "result" });

    expect(engine.loadedModels).toEqual([Array.from(model)]);
    expect((await storedKeys()).sort()).toEqual(Object.values(files).map((file) => file.sha256).sort());
  });

  it("refuses a downloaded file whose bytes are not the ones the registry lists", async () => {
    served = { "lex.bin": new Uint8Array([6, 7, 9]) };

    expect(await startWorker(standInEngine())("Nie wiem")).toMatchObject({ type: "error" });
    expect(await storedKeys()).not.toContain(files.lex.sha256);
  });
});
