/*
 * Stellarr's service worker.
 *
 * This file is served as-is from the site root and registered by every page
 * of the web client at start. It is the one worker of the origin's scope,
 * which has room for one, so it does two jobs: it handles push and clicks,
 * and it keeps the web client's shell for the release in use and answers
 * from it (see "The shell" below).
 *
 * The push payload is { id, title, body, url, tag }. Retries of the same
 * delivery reuse its tag, so they replace its previous system notification.
 */
"use strict";

// The build hashes `APP_ICON_URL`; this stable copy is the same icon, so a
// worker that is not bundled can still name it.
const NOTIFICATION_ICON = "/notification-icon.svg";
const HOME_URL = "/";

const MAX_ID = 128;
const MAX_TITLE = 200;
const MAX_BODY = 500;
const MAX_URL = 2048;
const MAX_TAG = 200;
const FALLBACK_TAG_PREFIX = "stellarr-push:";

function text(value, limit) {
  return typeof value === "string" ? value.trim().slice(0, limit) : "";
}

/** A same-origin path, or the home page when the payload names anything else. */
function safeUrl(value) {
  if (typeof value !== "string" || value.length === 0 || value.length > MAX_URL) {
    return HOME_URL;
  }
  try {
    const decoded = decodeURI(value);
    if ([value, decoded].some((text) => /[\\\u0000-\u001f\u007f]/.test(text))) return HOME_URL;
    const url = new URL(value, self.location.origin);
    if (url.origin !== self.location.origin) return HOME_URL;
    if (url.protocol !== "http:" && url.protocol !== "https:") return HOME_URL;
    if (url.pathname.startsWith("//")) return HOME_URL;
    return `${url.pathname}${url.search}${url.hash}` || HOME_URL;
  } catch {
    return HOME_URL;
  }
}

function parsePayload(data) {
  if (data === null || data === undefined) return null;
  let raw;
  try {
    raw = data.json();
  } catch {
    try {
      raw = JSON.parse(data.text());
    } catch {
      return null;
    }
  }
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return null;

  const id = text(raw.id, MAX_ID);
  const title = text(raw.title, MAX_TITLE);
  if (id.length === 0 || title.length === 0) return null;

  const body = text(raw.body, MAX_BODY) || title;
  const tag = text(raw.tag, MAX_TAG) || `${FALLBACK_TAG_PREFIX}${id}`;
  return { id, title, body, tag, url: safeUrl(raw.url) };
}

async function showPushNotification(payload) {
  // Every received Web Push must produce a visible system notification.
  await self.registration.showNotification(payload.title, {
    body: payload.body,
    tag: payload.tag,
    icon: NOTIFICATION_ICON,
    badge: NOTIFICATION_ICON,
    data: { url: payload.url, notificationId: payload.id },
  });
  return true;
}

function clientKey(value) {
  try {
    const url = new URL(value, self.location.origin);
    if (url.origin !== self.location.origin) return null;
    return `${url.pathname}${url.search}${url.hash}` || HOME_URL;
  } catch {
    return null;
  }
}

// A signed-in client can open the conversation in place, keeping its call alive.
// An old or suspended client may never acknowledge, so keep the cold-start path.
function handOffNotice(client, url) {
  return new Promise((resolve) => {
    const channel = new MessageChannel();
    const settle = (ok) => {
      clearTimeout(timer);
      channel.port1.close();
      resolve(ok);
    };
    const timer = setTimeout(() => settle(false), 800);
    channel.port1.onmessage = (event) => settle(event.data === true);
    try {
      client.postMessage({ type: "stellarr-open-notice", url }, [channel.port2]);
    } catch {
      settle(false);
    }
  });
}

async function openNotification(value) {
  const target = safeUrl(value);
  let windows = [];
  try {
    windows = await self.clients.matchAll({
      type: "window",
      includeUncontrolled: true,
    });
  } catch {
    windows = [];
  }

  let client = windows.find((candidate) => clientKey(candidate.url) === target);
  if (!client && target.startsWith("/?notice=")) {
    const home = windows.find((candidate) => clientKey(candidate.url) === HOME_URL);
    if (home && typeof home.postMessage === "function") {
      await home.focus();
      if (await handOffNotice(home, target)) return;
    }
  }
  // The home page is a focus target of its own: bring the app forward without
  // inventing a route or moving the person away from where they were.
  if (client === undefined && target === HOME_URL) {
    client = windows.find((candidate) => ["/", "/login", "/register"].includes(clientKey(candidate.url)));
  }

  if (client !== undefined) {
    await client.focus();
    if (
      target !== HOME_URL &&
      clientKey(client.url) !== target &&
      typeof client.navigate === "function"
    ) {
      await client.navigate(target);
    }
    return;
  }

  await self.clients.openWindow(target);
}

/*
 * The shell.
 *
 * The worker keeps a copy of the web client's shell for the release the page
 * last switched to: the web document and the files under /static/ and /fonts/
 * that it and the signed-in app need, as /release.json lists them. A
 * navigation to the web client is answered from that copy without waiting for
 * the network, and so is a file of the shell. Everything else goes to the
 * network as if the worker were not there: Cockpit, the API and the hub, the
 * worklets, codecs, models and sounds.
 *
 * A release is taken in this order, which keeps parts of two releases apart:
 * every file of the new shell is fetched and stored in a cache of its own, and
 * only then is the pointer to the current shell moved to it. The page reloads
 * after that. Nothing is deleted before the switch, and a store that fails
 * leaves the pointer where it was.
 */

const RELEASE_URL = "/release.json";
const DOCUMENT_PATH = "/index.html";
const INSTANCE_PATH = "/stellarr-instance.js";

// One cache per release, named by its id, holding the files of its shell.
const SHELL_CACHE_PREFIX = "stellarr-shell:";
// The cache that outlives releases: the pointer to the current shell and the
// last answer for the instance file.
const KEPT_CACHE = "stellarr-kept";
const CURRENT_KEY = "/current";

const ADOPT_MESSAGE = "stellarr-adopt";

// What nginx hands to the API; never the web client's address, whatever the
// path rules say. Repeats infra/nginx.conf.
const API_PATH = /^\/(api|assets|health|hubs|_activity|_stellarr|\.well-known)(\/|$)/;
const BUILD_FILE_PATH = /^\/(static|fonts)\//;

function mergeSlashes(pathname) {
  return pathname.replace(/\/{2,}/g, "/");
}

/*
 * Whether the web client's document answers `pathname`. This is the rule of
 * documentFor in web/documents.ts, with the folders that hold only files as
 * the release file gives them; documents.test.ts holds the two to the same
 * answers.
 */
function isWebAddress(pathname, directories) {
  const segments = mergeSlashes(pathname).split("/");
  const first = segments[1] || "";
  const last = segments[segments.length - 1] || "";
  if (first.startsWith("@") || directories.includes(first) || last.includes(".")) return false;
  return first !== "cockpit";
}

/*
 * How a request is answered: "document" and "file" from the shell's copy,
 * "instance" from its last copy, and "network" for
 * everything the worker leaves alone. `directories` is null while no shell is
 * kept.
 */
function routeOf(method, mode, url, origin, directories) {
  if (method !== "GET" || url.origin !== origin) return "network";
  if (url.pathname === INSTANCE_PATH) return "instance";
  if (directories === null || API_PATH.test(mergeSlashes(url.pathname))) return "network";
  if (mode === "navigate") return isWebAddress(url.pathname, directories) ? "document" : "network";
  return BUILD_FILE_PATH.test(url.pathname) ? "file" : "network";
}

// The shell this worker answers from: undefined until the pointer has been
// read, null when no shell is kept, else { id, directories }.
let current;

async function readCurrent() {
  const kept = await caches.open(KEPT_CACHE);
  const stored = await kept.match(CURRENT_KEY);
  if (stored === undefined) return null;
  const { id, directories } = await stored.json();
  return typeof id === "string" && Array.isArray(directories) ? { id, directories } : null;
}

async function loadCurrent() {
  if (current === undefined) {
    try {
      current = await readCurrent();
    } catch {
      current = null;
    }
  }
  return current;
}

/** The shell's copy of a file, or the network's when the copy has none. */
async function fromShell(request, url, route) {
  try {
    const cache = await caches.open(SHELL_CACHE_PREFIX + current.id);
    const kept = await cache.match(route === "document" ? DOCUMENT_PATH : url.pathname);
    if (kept !== undefined) return kept;
  } catch {
    // A cache that cannot be read is no cache.
  }
  return fetch(request);
}

/*
 * The instance file, from the last copy at once when there is one, and
 * refreshed from the network behind it with an ordinary fetch, so the hour
 * of freshness nginx gives it still holds and a load never waits on it. A change of
 * the instance language reaches the page one load later. With no copy yet
 * the fetch is waited for and its answer kept.
 */
async function fromKeptCopy(request, event) {
  const cache = await caches.open(KEPT_CACHE).catch(() => null);
  const kept = cache === null ? undefined : await cache.match(INSTANCE_PATH).catch(() => undefined);
  const refresh = fetch(request).then(async (response) => {
    if (response.ok && cache !== null) await cache.put(INSTANCE_PATH, response.clone()).catch(() => undefined);
    return response;
  });
  if (kept === undefined) return refresh;
  event.waitUntil(refresh.catch(() => undefined));
  return kept;
}

async function answer(request, url, event) {
  await loadCurrent();
  const directories = current === null ? null : current.directories;
  const route = routeOf(request.method, request.mode, url, self.location.origin, directories);
  if (route === "instance") return fromKeptCopy(request, event);
  if (route === "network") return fetch(request);
  return fromShell(request, url, route);
}

// The share POST is a local handoff. Keep each batch under a random handle;
// the redirect contains neither a filename nor the bytes. The page claims it
// in the same IndexedDB transaction that checks the account and expiry.
async function receiveShare(request) {
  let result = "error";
  let db;
  try {
    const origin = request.headers.get("Origin");
    if (origin && origin !== "null" && origin !== self.location.origin) throw Error();
    if (!request.headers.get("Content-Type")?.startsWith("multipart/form-data")) throw Error();
    const limit = 256 * 1024 * 1024;
    if (Number(request.headers.get("Content-Length")) > limit + 65536) throw Error();
    const files = (await request.formData()).getAll("files");
    if (!files.length || files.length > 10 || files.some(file => !(file instanceof File) || !file.name || !file.size)
        || files.reduce((size, file) => size + file.size, 0) > limit) throw Error();
    const id = crypto.randomUUID();
    db = await new Promise((resolve, reject) => {
      const open = indexedDB.open("stellarr-incoming-shares", 1);
      open.onupgradeneeded = () => open.result.createObjectStore("shares");
      open.onsuccess = () => resolve(open.result);
      open.onerror = () => reject(open.error);
    });
    await new Promise((resolve, reject) => {
      const tx = db.transaction("shares", "readwrite");
      const store = tx.objectStore("shares");
      const now = Date.now();
      let pending = 0;
      let owner = null;
      const cursor = store.openCursor();
      cursor.onsuccess = () => {
        const row = cursor.result;
        if (row) {
          if (row.key === "owner") owner = row.value;
          else if (row.value.expires <= now) row.delete();
          else pending++;
          row.continue();
        } else if (pending < 4) {
          store.add({ owner, files, expires: now + 60 * 60 * 1000 }, id);
        } else tx.abort();
      };
      tx.oncomplete = resolve;
      tx.onabort = tx.onerror = () => reject(tx.error);
    });
    result = id;
  } catch {
    // The landing dialog offers the ordinary file picker on failure.
  } finally {
    db?.close();
  }
  return Response.redirect(new URL("/?share=" + result, self.location.origin), 303);
}

self.addEventListener("fetch", (event) => {
  const request = event.request;
  const url = new URL(request.url);
  if (url.origin === self.location.origin && url.pathname === "/pwa/share" && request.method === "POST" && request.mode === "navigate") {
    event.respondWith(receiveShare(request));
    return;
  }
  // Until the pointer is read, the folders are not known and every folder
  // counts as a page, which only lets more requests through to `answer`.
  const directories = current === undefined ? [] : current === null ? null : current.directories;
  if (routeOf(request.method, request.mode, url, self.location.origin, directories) === "network") return;
  event.respondWith(answer(request, url, event));
});

function isAddress(value) {
  return typeof value === "string" && value.startsWith("/") && !value.startsWith("//");
}

/** The release the server serves now, with the files of its shell. */
async function fetchManifest() {
  const response = await fetch(RELEASE_URL, { cache: "no-store" });
  if (!response.ok) throw new Error("The release file is not there.");
  const manifest = await response.json();
  const valid =
    typeof manifest.id === "string" &&
    manifest.id !== "" &&
    Array.isArray(manifest.shell) &&
    manifest.shell.every(isAddress) &&
    manifest.shell.includes(DOCUMENT_PATH) &&
    Array.isArray(manifest.directories) &&
    manifest.directories.every((directory) => typeof directory === "string");
  if (!valid) throw new Error("The release file is not a release.");
  return manifest;
}

/** Whether the cache of a shell holds its document: a pointer can name a cache that is gone. */
async function shellIsHeld(id) {
  try {
    const cache = await caches.open(SHELL_CACHE_PREFIX + id);
    return (await cache.match(DOCUMENT_PATH)) !== undefined;
  } catch {
    return false;
  }
}

/** Fetches every file of the shell into a cache of its own, past the HTTP cache. */
async function storeShell(manifest) {
  const name = SHELL_CACHE_PREFIX + manifest.id;
  // What an earlier attempt that failed half way left behind.
  await caches.delete(name);
  try {
    const cache = await caches.open(name);
    await Promise.all(
      manifest.shell.map(async (path) => {
        const response = await fetch(path, { cache: "reload" });
        if (!response.ok) throw new Error(path + " answered " + response.status + ".");
        await cache.put(path, response);
      }),
    );
    // A server that moved on while the files were fetched would hand over
    // the document of one release and the files of another.
    const document = await cache.match(DOCUMENT_PATH);
    if (document === undefined || !(await document.text()).includes(manifest.id)) {
      throw new Error("The document is not the one of release " + manifest.id + ".");
    }
  } catch (error) {
    await caches.delete(name).catch(() => false);
    throw error;
  }
  return name;
}

/** Makes the stored shell the one the worker answers from. */
async function switchTo(manifest) {
  const kept = await caches.open(KEPT_CACHE);
  const pointer = JSON.stringify({ id: manifest.id, directories: manifest.directories });
  await kept.put(CURRENT_KEY, new Response(pointer, { headers: { "Content-Type": "application/json" } }));
  current = { id: manifest.id, directories: manifest.directories };
}

/** Deletes every shell but `keep`, the copies of releases that are over. */
async function forgetShellsBut(keep) {
  for (const name of await caches.keys()) {
    if (name.startsWith(SHELL_CACHE_PREFIX) && name !== keep) await caches.delete(name);
  }
}

/*
 * Takes the release the server serves now: stores its whole shell, moves the
 * pointer to it, and only then deletes the copies of older releases. A
 * failure before the move leaves the pointer and the old shell as they were,
 * and the partial copy is deleted. Resolves to the id of the release taken.
 */
async function takeRelease() {
  const manifest = await fetchManifest();
  const held = await loadCurrent();
  if (held !== null && held.id === manifest.id && (await shellIsHeld(held.id))) return manifest.id;
  const name = await storeShell(manifest);
  await switchTo(manifest);
  await forgetShellsBut(name).catch(() => undefined);
  return manifest.id;
}

// Messages that arrive while a release is being taken wait for that run and
// then take whatever the server serves by then, since a run that began before
// a release changed would otherwise answer for the old one.
let taking = null;

function takeOnce() {
  const run = taking === null ? takeRelease() : taking.then(takeRelease, takeRelease);
  taking = run;
  run.then(
    () => undefined,
    () => undefined,
  ).then(() => {
    if (taking === run) taking = null;
  });
  return run;
}

self.addEventListener("install", (event) => {
  event.waitUntil(self.skipWaiting());
});

// What a crash in the middle of taking a release left behind.
async function forgetStaleShells() {
  const held = await loadCurrent();
  await forgetShellsBut(held === null ? null : SHELL_CACHE_PREFIX + held.id);
}

self.addEventListener("activate", (event) => {
  event.waitUntil(Promise.all([self.clients.claim(), forgetStaleShells().catch(() => undefined)]));
});

// A page asks for the release it has found and gets an answer on the port it
// sends: { ok: true, id } once the shell is stored and switched to, or
// { ok: false } when it could not be, with the old shell still in place.
self.addEventListener("message", (event) => {
  const data = event.data;
  if (typeof data !== "object" || data === null || data.type !== ADOPT_MESSAGE) return;
  const port = event.ports[0];
  const reply = takeOnce().then(
    (id) => ({ ok: true, id }),
    () => ({ ok: false }),
  );
  event.waitUntil(reply.then((result) => (port === undefined ? undefined : port.postMessage(result))));
});

// Read the absolute count so repeated or out-of-order deliveries converge.
let badgeRevision = 0;
async function refreshBadge() {
  const revision = ++badgeRevision;
  const badge = self.navigator;
  if (typeof badge?.setAppBadge !== "function") return;
  const response = await fetch("/api/me/app-badge", { credentials: "same-origin", cache: "no-store", redirect: "error" });
  if (!response.ok && response.status !== 401) return;
  const unread = response.ok ? (await response.json()).unread : 0;
  if (revision !== badgeRevision || !Number.isSafeInteger(unread) || unread < 0) return;
  if (unread) await badge.setAppBadge(unread);
  else await badge.clearAppBadge();
}

self.addEventListener("push", (event) => {
  const payload = parsePayload(event.data) ?? {
    title: "Stellarr", body: "", tag: "stellarr-activity", url: HOME_URL,
  };
  event.waitUntil(Promise.all([showPushNotification(payload), refreshBadge()].map(task => task.catch(() => undefined))));
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const data = event.notification.data;
  const url = data && typeof data.url === "string" ? data.url : HOME_URL;
  event.waitUntil(
    openNotification(url).catch(() => self.clients.openWindow(safeUrl(url)).catch(() => undefined)),
  );
});
