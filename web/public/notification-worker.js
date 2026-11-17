/*
 * Stellarr's Web Push service worker.
 *
 * This file is served as-is from the site root and is registered only when a
 * person explicitly enrolls this device. It handles push and clicks, and does
 * not intercept fetches or cache anything.
 *
 * The push payload is { id, title, body, url, tag }. The server sends `tag`
 * with the same value the live page uses for that event when it has one, so a
 * system notification and a page notification replace each other instead of
 * stacking. A payload without a tag gets a stable per-id tag of its own.
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
    const url = new URL(value, self.location.origin);
    if (url.origin !== self.location.origin) return HOME_URL;
    if (url.protocol !== "http:" && url.protocol !== "https:") return HOME_URL;
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

async function visibleFocusedClient() {
  try {
    const windows = await self.clients.matchAll({
      type: "window",
      includeUncontrolled: true,
    });
    return windows.some(
      (client) => client.visibilityState === "visible" && client.focused === true,
    );
  } catch {
    // If the client list cannot be read, delivering is better than dropping.
    return false;
  }
}

async function showPushNotification(payload) {
  // A visible, focused app already showed this in the page. A hidden app does
  // not suppress it: the system notification is how a phone reaches someone
  // whose browser is in the background or closed.
  if (await visibleFocusedClient()) return false;

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
  // The home page is a focus target of its own: bring the app forward without
  // inventing a route or moving the person away from where they were.
  if (client === undefined && target === HOME_URL) {
    client = windows.find((candidate) => clientKey(candidate.url) !== null);
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

self.addEventListener("install", (event) => {
  event.waitUntil(self.skipWaiting());
});

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

self.addEventListener("push", (event) => {
  const payload = parsePayload(event.data);
  if (payload === null) return;
  event.waitUntil(showPushNotification(payload).catch(() => undefined));
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const data = event.notification.data;
  const url = data && typeof data.url === "string" ? data.url : HOME_URL;
  event.waitUntil(
    openNotification(url).catch(() => self.clients.openWindow(HOME_URL).catch(() => undefined)),
  );
});
