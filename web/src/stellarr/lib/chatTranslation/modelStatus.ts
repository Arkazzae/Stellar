import type { TranslationTargetLanguage } from "@/i18n";

import { openDatabase, requestResult, transactionDone } from "./idb";

const BUNDLED_MODELS = "/bergamot-models";

// public/bergamot-worker.js reads and writes the same database. Both store a
// file under its SHA-256 from the registry, and only after checking the bytes
// against it, so a stored file is reused only while the registry names exactly
// those bytes, whatever the file is called.
const DB_NAME = "bergamot-models";
const DB_VERSION = 1;
const STORE = "files";

interface ModelFile {
  name: string;
  size: number;
  sha256: string;
}

interface PairEntry {
  model: ModelFile;
  lex: ModelFile;
  vocab: ModelFile;
}

export type ModelStatus = "ready" | "not-downloaded" | "unavailable";

let registryRequest: Promise<Record<string, PairEntry>> | null = null;

async function fetchRegistry(): Promise<Record<string, PairEntry>> {
  const response = await fetch(`${BUNDLED_MODELS}/registry.json`);
  if (!response.ok) throw new Error(`Could not fetch the model registry (${response.status}).`);
  return response.json();
}

function loadRegistry(): Promise<Record<string, PairEntry>> {
  registryRequest ??= fetchRegistry().catch((error) => {
    // A failed request must not be remembered, or "retry" could never succeed.
    registryRequest = null;
    throw error;
  });
  return registryRequest;
}

/**
 * The model for the most likely source language, which is Polish when reading
 * into English and English when reading into Polish. Other pairs are fetched
 * by the worker the first time a message needs them.
 */
export function primaryPair(targetLanguage: TranslationTargetLanguage): string {
  return targetLanguage === "en" ? "plen" : "enpl";
}

async function pairFiles(pair: string): Promise<ModelFile[] | null> {
  const entry = (await loadRegistry())[pair];
  return entry === undefined ? null : [entry.model, entry.lex, entry.vocab];
}

function totalSize(files: readonly ModelFile[]): number {
  return files.reduce((sum, file) => sum + file.size, 0);
}

const openModels = () =>
  openDatabase(DB_NAME, DB_VERSION, (database) => {
    if (!database.objectStoreNames.contains(STORE)) database.createObjectStore(STORE);
  });

async function isStored(database: IDBDatabase, key: string): Promise<boolean> {
  try {
    const request = database.transaction(STORE, "readonly").objectStore(STORE).count(key);
    return (await requestResult(request)) > 0;
  } catch {
    return false;
  }
}

export async function getModelStatus(
  targetLanguage: TranslationTargetLanguage,
): Promise<ModelStatus> {
  const pair = primaryPair(targetLanguage);
  const files = await pairFiles(pair);
  if (files === null) return "unavailable";
  if (typeof indexedDB === "undefined") return "not-downloaded";

  const database = await openModels();
  try {
    const stored = await Promise.all(files.map((file) => isStored(database, file.sha256)));
    return stored.every(Boolean) ? "ready" : "not-downloaded";
  } finally {
    database.close();
  }
}

export async function getPairSize(targetLanguage: TranslationTargetLanguage): Promise<number> {
  const files = await pairFiles(primaryPair(targetLanguage));
  return files === null ? 0 : totalSize(files);
}

/** Streams the copy the instance serves so the caller can show progress. */
async function downloadFile(
  pair: string,
  file: ModelFile,
  onBytes: (count: number) => void,
): Promise<ArrayBuffer> {
  const response = await fetch(`${BUNDLED_MODELS}/${pair}/${file.name}`);
  if (!response.ok) throw new Error(`Could not fetch the model file ${file.name}.`);

  if (response.body === null) {
    const buffer = await response.arrayBuffer();
    onBytes(buffer.byteLength);
    return buffer;
  }

  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let received = 0;
  for (let part = await reader.read(); !part.done; part = await reader.read()) {
    chunks.push(part.value);
    received += part.value.byteLength;
    onBytes(part.value.byteLength);
  }

  const content = new Uint8Array(received);
  let offset = 0;
  for (const chunk of chunks) {
    content.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return content.buffer;
}

async function sha256(content: ArrayBuffer): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", content));
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

async function store(database: IDBDatabase, key: string, content: ArrayBuffer): Promise<void> {
  const transaction = database.transaction(STORE, "readwrite");
  transaction.objectStore(STORE).put(content, key);
  await transactionDone(transaction);
}

/**
 * Removes the files no pair of the registry names, those of an earlier
 * release, which would otherwise stay on the device for good.
 */
async function keepOnly(database: IDBDatabase, registry: Record<string, PairEntry>): Promise<void> {
  const named = new Set(
    Object.values(registry).flatMap((entry) => [entry.model, entry.lex, entry.vocab].map((file) => file.sha256)),
  );
  const transaction = database.transaction(STORE, "readwrite");
  const files = transaction.objectStore(STORE);
  for (const key of await requestResult(files.getAllKeys())) {
    if (typeof key !== "string" || !named.has(key)) files.delete(key);
  }
  await transactionDone(transaction);
}

/** Progress is a whole percentage; 100 is reported only once everything is stored. */
export async function downloadModel(
  targetLanguage: TranslationTargetLanguage,
  onProgress?: (percent: number) => void,
): Promise<void> {
  const pair = primaryPair(targetLanguage);
  const files = await pairFiles(pair);
  if (files === null) throw new Error(`No model for the language pair ${pair}.`);

  const total = totalSize(files);
  let finished = 0;
  const report = () => onProgress?.(Math.min(99, Math.round((finished / total) * 100)));

  const database = await openModels();
  try {
    await keepOnly(database, await loadRegistry());
    for (const file of files) {
      if (await isStored(database, file.sha256)) {
        finished += file.size;
        report();
        continue;
      }

      const content = await downloadFile(pair, file, (count) => {
        finished += count;
        report();
      });
      // A listed size of 0 means the size is unknown, so there is nothing to compare.
      if (file.size > 0 && content.byteLength !== file.size) {
        throw new Error(`The downloaded model file ${file.name} is incomplete.`);
      }
      if ((await sha256(content)) !== file.sha256) {
        throw new Error(`The downloaded model file ${file.name} is not the one the registry lists.`);
      }
      await store(database, file.sha256, content);
    }
    onProgress?.(100);
  } finally {
    database.close();
  }
}
