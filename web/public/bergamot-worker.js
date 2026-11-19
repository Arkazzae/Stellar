/**
 * Translation worker: runs the Bergamot engine, Mozilla's build of the Marian
 * translator for the web, on the models this instance serves, and keeps the
 * model files in IndexedDB so a pair is downloaded once.
 *
 * Protocol:
 *   Main → Worker: { type: "translate", id, text, from, to }
 *   Worker → Main: { type: "result", id, translation }
 *   Worker → Main: { type: "error", id, message }
 */

const ENGINE_GLUE = "/codecs/bergamot/bergamot-translator.js";
const ENGINE_WASM = "/codecs/bergamot/bergamot-translator.wasm";
const MODELS = "/bergamot-models";

// Marian's decoding options, as Firefox passes them. The model file carries
// its own architecture; these only shape the search. The precision is the one
// for `*.intgemm.alphas.bin` models, the only kind the registry lists.
const MODEL_CONFIG = `beam-size: 1
normalize: 1.0
word-penalty: 0
max-length-break: 128
mini-batch-words: 1024
workspace: 128
max-length-factor: 2.0
skip-cost: true
cpu-threads: 0
quiet: true
quiet-translation: true
gemm-precision: int8shiftAlphaAll
alignment: soft
`;

// The alignment, in bytes, the engine needs for each file in its memory.
const ALIGNMENT = { model: 256, lex: 64, vocab: 64 };

// ── IndexedDB ──
//
// src/stellarr/lib/chatTranslation/modelStatus.ts reads and writes the same
// database. A file is stored under its SHA-256, which registry.json lists, so
// a file is only reused while the registry names exactly those bytes, and is
// only stored once its bytes have been checked against it.

const DB_NAME = "bergamot-models";
const DB_STORE = "files";
const DB_VERSION = 1;

function openDB() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => req.result.createObjectStore(DB_STORE);
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function idbGet(key) {
  const db = await openDB();
  try {
    return await new Promise((resolve, reject) => {
      const req = db.transaction(DB_STORE, "readonly").objectStore(DB_STORE).get(key);
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
  } finally {
    db.close();
  }
}

async function idbSet(key, value) {
  const db = await openDB();
  try {
    await new Promise((resolve, reject) => {
      const tx = db.transaction(DB_STORE, "readwrite");
      tx.objectStore(DB_STORE).put(value, key);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error);
    });
  } finally {
    db.close();
  }
}

/** Deletes every stored file whose hash is not in `keep`. */
async function idbKeepOnly(keep) {
  const db = await openDB();
  try {
    await new Promise((resolve, reject) => {
      const tx = db.transaction(DB_STORE, "readwrite");
      const store = tx.objectStore(DB_STORE);
      const req = store.getAllKeys();
      req.onsuccess = () => {
        for (const key of req.result) if (!keep.has(key)) store.delete(key);
      };
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error);
    });
  } finally {
    db.close();
  }
}

// ── Model registry ──

let registryRequest = null;

/** registry.json: `{ <pair>: { model, lex, vocab } }`, each `{ name, size, sha256 }`. */
function loadRegistry() {
  registryRequest ??= fetch(`${MODELS}/registry.json`)
    .then((response) => {
      if (!response.ok) throw new Error(`Failed to fetch model registry: ${response.status}`);
      return response.json();
    })
    .catch((error) => {
      // A failed request must not be remembered, or no later message could succeed.
      registryRequest = null;
      throw error;
    });
  return registryRequest;
}

let pruning = null;

/**
 * Once per worker, removes the files the registry no longer names: those of
 * an earlier release, which would otherwise stay on the device for good.
 */
function pruneStoredFiles(registry) {
  pruning ??= idbKeepOnly(
    new Set(Object.values(registry).flatMap((pair) => Object.values(pair).map((file) => file.sha256))),
  ).catch((error) => {
    // A failed cleanup costs disk space, not a translation.
    pruning = null;
    console.warn("[bergamot] could not remove stale model files:", error);
  });
  return pruning;
}

async function sha256(buffer) {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", buffer));
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

async function modelFile(pair, file) {
  const stored = await idbGet(file.sha256);
  if (stored !== undefined) return stored;

  const response = await fetch(`${MODELS}/${pair}/${file.name}`);
  if (!response.ok) throw new Error(`Failed to download model ${pair}/${file.name}`);
  const buffer = await response.arrayBuffer();
  if ((await sha256(buffer)) !== file.sha256) {
    throw new Error(`The model file ${pair}/${file.name} is not the one the registry lists`);
  }
  await idbSet(file.sha256, buffer);
  return buffer;
}

// ── Engine ──

let engineRequest = null;

/**
 * The glue defines `loadBergamot(Module)`, which compiles the WASM into the
 * object it is given and adds the engine's classes to it.
 */
function loadEngine() {
  engineRequest ??= new Promise((resolve, reject) => {
    importScripts(ENGINE_GLUE);
    const engine = {
      // The glue looks for its WASM beside the URL of the worker, not beside
      // its own.
      locateFile: () => ENGINE_WASM,
      // Firefox starts with 40 MiB: growing from a small start uses less memory
      // than overshooting it, and the default 16 MiB only means more growing.
      INITIAL_MEMORY: 41_943_040,
      print: (line) => console.debug("[bergamot]", line),
      printErr: (line) => console.warn("[bergamot]", line),
      onAbort: (reason) => reject(new Error(`The translation engine stopped: ${reason}`)),
      onRuntimeInitialized: () => resolve(engine),
    };
    loadBergamot(engine);
  }).catch((error) => {
    engineRequest = null;
    throw error;
  });
  return engineRequest;
}

function alignedCopy(engine, buffer, alignment) {
  const memory = new engine.AlignedMemory(buffer.byteLength, alignment);
  memory.getByteArrayView().set(new Uint8Array(buffer));
  return memory;
}

const models = new Map();
let service = null;

/** Forgets the engine and what lives in its memory, so the next message starts over. */
function discardEngine() {
  engineRequest = null;
  models.clear();
  service = null;
}

async function createModel(engine, pair, from, to) {
  const registry = await loadRegistry();
  const files = registry[pair];
  const [model, lex, vocab] = await Promise.all([
    modelFile(pair, files.model),
    modelFile(pair, files.lex),
    modelFile(pair, files.vocab),
  ]);

  const modelMemory = alignedCopy(engine, model, ALIGNMENT.model);
  const lexMemory = alignedCopy(engine, lex, ALIGNMENT.lex);
  const vocabMemory = alignedCopy(engine, vocab, ALIGNMENT.vocab);
  // One vocabulary serves both sides; the engine shares it.
  const vocabs = new engine.AlignedMemoryList();
  vocabs.push_back(vocabMemory);
  try {
    // The languages name the Intl.Segmenter that splits the source text into
    // sentences.
    return new engine.TranslationModel(from, to, MODEL_CONFIG, modelMemory, lexMemory, vocabs, null);
  } finally {
    // The model has moved the bytes out of these; only their handles are left.
    for (const handle of [modelMemory, lexMemory, vocabMemory, vocabs]) handle.delete();
  }
}

function getModel(engine, from, to) {
  const pair = `${from}${to}`;
  let model = models.get(pair);
  if (model === undefined) {
    model = createModel(engine, pair, from, to);
    models.set(pair, model);
    model.catch(() => models.delete(pair));
  }
  return model;
}

function translateWith(engine, chain, text) {
  // The client keeps finished translations, so the engine's own cache would
  // only hold a second copy; Firefox turns it off too.
  service ??= new engine.BlockingService({ cacheSize: 0 });
  const input = new engine.VectorString();
  const options = new engine.VectorResponseOptions();
  let output = null;
  try {
    input.push_back(text);
    options.push_back({ qualityScores: false, alignment: false, html: false });
    output =
      chain.length === 1
        ? service.translate(chain[0], input, options)
        : service.translateViaPivoting(chain[0], chain[1], input, options);
    const response = output.get(0);
    try {
      return response.getTranslatedText();
    } finally {
      response.delete();
    }
  } finally {
    input.delete();
    options.delete();
    output?.delete();
  }
}

// ── A reply said twice ──
//
// The models say a short reply twice when it is one capitalised word and one
// full stop or exclamation mark: "Dzięki!" comes back as "Thanks! Thanks!",
// "No." as "Nie. Nie.". Firefox, with the same engine and options, does the
// same. When the text is one sentence, split as the engine splits it, and the
// translation is one sentence twice over, the second is dropped. A text that
// repeats itself is two sentences and keeps its repeat.

function sentences(text, language) {
  const segmenter = new Intl.Segmenter(language, { granularity: "sentence" });
  return Array.from(segmenter.segment(text), ({ segment }) => segment);
}

function withoutRepeat(text, translation, from, to) {
  if (sentences(text, from).length !== 1) return translation;
  const parts = sentences(translation, to);
  if (parts.length !== 2) return translation;
  const [first, second] = parts;
  const sentence = second.trim();
  if (sentence === "" || first.trim() !== sentence) return translation;
  // Whatever whitespace the translation starts and ends with stays.
  return first.slice(0, first.indexOf(sentence) + sentence.length) + second.slice(second.trimEnd().length);
}

async function handleTranslate(from, to, text) {
  if (text.trim() === "") return text;

  const registry = await loadRegistry();
  await pruneStoredFiles(registry);

  let languages;
  if (registry[`${from}${to}`]) languages = [[from, to]];
  else if (registry[`${from}en`] && registry[`en${to}`]) languages = [[from, "en"], ["en", to]];
  else throw new Error(`No translation model available for ${from} → ${to}`);

  const engine = await loadEngine();
  try {
    const chain = await Promise.all(languages.map(([source, target]) => getModel(engine, source, target)));
    return withoutRepeat(text, translateWith(engine, chain, text), from, to);
  } catch (error) {
    // An abort inside the WASM, running out of memory among them, leaves the
    // engine unusable, and keeping it would fail every later message the same
    // way.
    if (error instanceof WebAssembly.RuntimeError) discardEngine();
    throw error;
  }
}

// ── Message handler ──

self.onmessage = async function (e) {
  const { type, id, text, from, to } = e.data;
  if (type !== "translate") return;

  try {
    const translation = await handleTranslate(from || "en", to || "en", text);
    self.postMessage({ type: "result", id, translation });
  } catch (err) {
    self.postMessage({ type: "error", id, message: err.message || String(err) });
  }
};
