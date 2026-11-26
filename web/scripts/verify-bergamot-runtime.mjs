// Smoke test for the translation runtime: runs the real worker and WASM in
// headless Chromium against the files assembled from artifacts/ into public/,
// and translates Polish into English and English into Polish. The model files
// are served from that same directory, so the test needs no network and no
// running Stellarr. It is not part of CI.
//
//   npm run test:translation-runtime        (STELLARR_CHROMIUM overrides the binary)
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { extname, join, resolve, sep } from "node:path";

import { assemble } from "../../artifacts/assemble.mjs";

const appPublic = resolve(import.meta.dirname, "..", "public");
const publicRoot = process.env.STELLARR_TRANSLATION_ROOT ? resolve(process.env.STELLARR_TRANSLATION_ROOT) : appPublic;
const chromium = process.env.STELLARR_CHROMIUM ?? "/usr/bin/chromium";
const startupTimeoutMs = 15_000;
const runTimeoutMs = 180_000;

// The first two hold two sentences each, so the text goes through the engine's
// sentence splitting. The expected words are a floor for a translation, not its
// wording.
const cases = [
  {
    from: "pl",
    to: "en",
    text: "Wczoraj wieczorem padał deszcz, więc zostaliśmy w domu. Było bardzo miło.",
    expect: [/\brain/i, /\bhome\b/i, /\bnice\b/i],
  },
  {
    from: "en",
    to: "pl",
    text: "It rained last night, so we stayed at home. It was very nice.",
    expect: [/pada/i, /\bdomu\b/i, /\bmił/i],
  },
  // A writer's apostrophes, quotes, & and < come back as they were; the 1.0
  // models made "5 > 3 & & 2" of the first.
  {
    from: "pl",
    to: "en",
    text: 'Nie wiem, czy 5 > 3 && 2 < 4 to "prawda".',
    expect: [/\bdon't\b/, /5 > 3 && 2 < 4/, /"/],
  },
  {
    from: "en",
    to: "pl",
    text: `I don't know if 5 > 3 && 2 < 4 is "true".`,
    expect: [/\bnie wiem\b/i, /5 > 3 && 2 < 4/, /"/],
  },
  // Replies the models say twice, which the worker says once.
  { from: "pl", to: "en", text: "Dzięki!", expect: [/^Thanks!$/] },
  { from: "en", to: "pl", text: "No.", expect: [/^Nie\.$/] },
];

const mimeTypes = {
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".wasm": "application/wasm",
};

// Runs the real worker after watching what it cannot report itself: which
// matrix multiplication the engine chose, which comes as a line it prints,
// and whether it split the text with Intl.Segmenter.
const probeWorker = `
const report = { printed: [], segmenters: [], mozIntGemm: typeof WebAssembly.mozIntGemm };
const NativeSegmenter = Intl.Segmenter;
Intl.Segmenter = class extends NativeSegmenter {
  constructor(locale, options) {
    super(locale, options);
    report.segmenters.push(locale + "/" + options?.granularity);
  }
};
const debug = console.debug.bind(console);
console.debug = (...parts) => {
  report.printed.push(parts.join(" "));
  debug(...parts);
};
self.addEventListener("message", ({ data }) => {
  if (data.type === "probe") self.postMessage({ type: "probe", report });
});
importScripts("/bergamot-worker.js");
`;

// Files an earlier release could have left in the model database: one under
// the name the worker used to store it by, one under a hash no registry
// lists. Neither may be used, and the worker must remove both.
const staleKeys = ["plen/model.plen.intgemm.alphas.bin", "0".repeat(64)];

// The page reports through its title, which the DevTools target list exposes
// without a protocol session.
const fixturePage = `<!doctype html>
<meta charset="utf-8">
<title>BERGAMOT_PENDING</title>
<script>
  const cases = ${JSON.stringify(cases.map(({ from, to, text }) => ({ from, to, text })))};
  const staleKeys = ${JSON.stringify(staleKeys)};

  function withStore(mode, use) {
    return new Promise((resolve, reject) => {
      const open = indexedDB.open("bergamot-models", 1);
      open.onupgradeneeded = () => open.result.createObjectStore("files");
      open.onerror = () => reject(open.error);
      open.onsuccess = () => {
        const database = open.result;
        const request = use(database.transaction("files", mode).objectStore("files"));
        request.onsuccess = () => {
          database.close();
          resolve(request.result);
        };
        request.onerror = () => reject(request.error);
      };
    });
  }

  async function run() {
    for (const key of staleKeys) {
      await withStore("readwrite", (store) => store.put(new ArrayBuffer(8), key));
    }

    const worker = new Worker("/probe-worker.js");
    const replies = new Map();
    worker.onmessage = ({ data }) => replies.get(data.type === "probe" ? "probe" : data.id)?.(data);
    worker.onerror = (event) => give("ERROR", event.message || "worker error");
    const ask = (key, message) =>
      new Promise((resolve) => {
        replies.set(key, resolve);
        worker.postMessage(message);
      });

    const translations = [];
    for (const [index, item] of cases.entries()) {
      const reply = await ask(index, { type: "translate", id: index, ...item });
      if (reply.type !== "result") return give("ERROR", reply.message);
      translations.push(reply.translation);
    }
    const { report } = await ask("probe", { type: "probe" });
    const keys = await withStore("readonly", (store) => store.getAllKeys());
    worker.terminate();
    // The target list escapes the title as HTML. Percent-encoding, apostrophes
    // included, which encodeURIComponent leaves as they are, gives it nothing
    // to escape.
    give("OK", encodeURIComponent(JSON.stringify({ translations, report, keys })).replaceAll("'", "%27"));
  }

  function give(verdict, detail) {
    document.title = "BERGAMOT_" + verdict + " " + detail;
  }

  run().catch((error) => give("ERROR", error.message || String(error)));
</script>`;

let downloadedModelFiles = 0;

async function serveStatic(pathname, response) {
  const root = pathname === "/bergamot-worker.js" ? appPublic : publicRoot;
  const file = resolve(root, `.${pathname}`);
  if (!file.startsWith(root + sep)) {
    response.writeHead(404).end();
    return;
  }
  let body;
  try {
    body = await readFile(file);
  } catch (error) {
    // Chromium asks for /favicon.ico, which public/ does not have.
    if (error?.code !== "ENOENT") throw error;
    response.writeHead(404).end();
    return;
  }
  if (pathname.startsWith("/bergamot-models/") && !pathname.endsWith("/registry.json")) {
    downloadedModelFiles += 1;
  }
  response.writeHead(200, {
    "content-type": mimeTypes[extname(file)] ?? "application/octet-stream",
    "content-length": body.byteLength,
  });
  response.end(body);
}

const server = createServer(async (request, response) => {
  try {
    const { pathname } = new URL(request.url ?? "/", "http://localhost");
    if (pathname === "/") {
      response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      response.end(fixturePage);
    } else if (pathname === "/probe-worker.js") {
      response.writeHead(200, { "content-type": mimeTypes[".js"] });
      response.end(probeWorker);
    } else {
      await serveStatic(pathname, response);
    }
  } catch (error) {
    // The detail goes to this terminal; the page only learns that it failed.
    console.error("[bergamot] stand-in server:", error);
    response.writeHead(500, { "content-type": "text/plain; charset=utf-8" });
    response.end("Internal error");
  }
});

function listen() {
  return new Promise((resolveListen, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => resolveListen(server.address().port));
  });
}

function startChromium(profile, url) {
  const browser = spawn(
    chromium,
    [
      "--headless=new",
      "--no-sandbox",
      "--disable-gpu",
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      url,
    ],
    { stdio: ["ignore", "ignore", "pipe"] },
  );

  const debuggingPort = new Promise((resolvePort, reject) => {
    const startupTimer = setTimeout(
      () => reject(new Error("Chromium did not open a debugging port.")),
      startupTimeoutMs,
    );
    let log = "";
    browser.stderr.setEncoding("utf8");
    browser.stderr.on("data", (chunk) => {
      log += chunk;
      const port = /DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)/.exec(log)?.[1];
      if (port === undefined) return;
      clearTimeout(startupTimer);
      resolvePort(port);
    });
    browser.once("exit", (code) => {
      clearTimeout(startupTimer);
      reject(new Error(`Chromium exited (${code}).\n${log}`));
    });
  });

  return { browser, debuggingPort };
}

async function waitForVerdict(port, url) {
  const deadline = Date.now() + runTimeoutMs;
  while (Date.now() < deadline) {
    const targets = await fetch(`http://127.0.0.1:${port}/json/list`).then((r) => r.json());
    const title = targets.find((target) => target.type === "page" && target.url === url)?.title;
    const verdict = /^BERGAMOT_(OK|ERROR) ([\s\S]*)$/.exec(title ?? "");
    if (verdict !== null) {
      if (verdict[1] === "OK") return JSON.parse(decodeURIComponent(verdict[2]));
      throw new Error(`Bergamot runtime failed: ${verdict[2] || "no detail"}`);
    }
    await new Promise((resolveWait) => setTimeout(resolveWait, 250));
  }
  throw new Error("Bergamot runtime did not answer in time.");
}

/** One browser session; the profile directory keeps IndexedDB between sessions. */
async function runSession(profile, url) {
  const { browser, debuggingPort } = startChromium(profile, url);
  const exited = new Promise((resolveExit) => browser.once("exit", resolveExit));
  try {
    return await waitForVerdict(await debuggingPort, url);
  } finally {
    browser.kill("SIGTERM");
    await exited;
  }
}

/** Throws unless the session translated both ways on the engine's own paths. */
function check(session, { translations, report, keys }, listedHashes) {
  for (const [index, { from, to, expect }] of cases.entries()) {
    const translation = translations[index];
    if (!expect.every((pattern) => pattern.test(translation))) {
      throw new Error(`${session}: ${from}→${to} gave "${translation}", which lacks ${expect.join(" or ")}.`);
    }
  }
  // Chromium has no built-in matrix multiplication for the engine to import.
  if (report.mozIntGemm !== "undefined" || !report.printed.some((line) => line.includes("Using fallback gemm"))) {
    throw new Error(`${session}: the engine did not take its own matrix multiplication: ${report.printed}`);
  }
  for (const language of new Set(cases.map(({ from }) => from))) {
    if (!report.segmenters.includes(`${language}/sentence`)) {
      throw new Error(`${session}: no Intl.Segmenter split ${language} into sentences: ${report.segmenters}`);
    }
  }
  if (JSON.stringify([...keys].sort()) !== JSON.stringify([...listedHashes].sort())) {
    throw new Error(`${session}: IndexedDB holds ${keys}, not exactly the files the registry lists.`);
  }
}

const problems = publicRoot === appPublic ? await assemble({ log: console.log }) : [];
if (problems.length > 0) {
  for (const problem of problems) console.error(`  - ${problem}`);
  throw new Error("The artifacts in public/ do not match artifacts/manifest.json.");
}

const registry = JSON.parse(await readFile(join(publicRoot, "bergamot-models", "registry.json"), "utf8"));
const pairs = cases.map(({ from, to }) => registry[`${from}${to}`]);
// A file both pairs share, the vocabulary, is stored once and fetched once.
const listedHashes = new Set(pairs.flatMap((files) => Object.values(files).map((file) => file.sha256)));

const profile = await mkdtemp(join(tmpdir(), "stellarr-bergamot-"));
try {
  const url = `http://127.0.0.1:${await listen()}/`;

  const first = await runSession(profile, url);
  check("first session", first, listedHashes);
  if (downloadedModelFiles !== listedHashes.size) {
    throw new Error(`Expected ${listedHashes.size} model downloads, saw ${downloadedModelFiles}.`);
  }

  const second = await runSession(profile, url);
  check("second session", second, listedHashes);
  if (downloadedModelFiles !== listedHashes.size) {
    throw new Error("The second session downloaded models that IndexedDB should have kept.");
  }

  for (const [index, { from, to, text }] of cases.entries()) {
    console.log(`[bergamot] ${from}→${to}: ${text}`);
    console.log(`[bergamot]   first session:  ${first.translations[index]}`);
    console.log(`[bergamot]   second session: ${second.translations[index]}`);
  }
  console.log(
    "[bergamot] WASM with its fallback matrix multiplication, Intl.Segmenter, " +
      "model loading, IndexedDB reuse and removal of stale files verified",
  );
} finally {
  await new Promise((resolveClose) => server.close(resolveClose));
  await rm(profile, { recursive: true, force: true });
}
