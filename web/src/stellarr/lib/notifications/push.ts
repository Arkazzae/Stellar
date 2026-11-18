import { WORKER_PATH, WORKER_SCOPE } from "@/lib/platform/worker";
import type { PushSubscriptionInput } from "@/ports/notificationDelivery";

/**
 * Web Push, kept apart from the Notification API the live page uses.
 *
 * The browser subscription is device-wide, not account-wide: it must be
 * inspected without enrolling, registered only when the person asks for it,
 * and removed before the session that bound it goes away. Nothing here logs a
 * subscription: its endpoint and keys are delivery credentials.
 */


/** How long a freshly registered worker may take to become active. */
const WORKER_TIMEOUT_MS = 8_000;
/** Reading a registration must never leave the settings pane waiting. */
const INSPECTION_TIMEOUT_MS = 2_000;
/** A subscription request can reach the push service; bound it as well. */
const SUBSCRIBE_TIMEOUT_MS = 15_000;
/** Cleanup must not hold a sign-out open when the network is gone. */
const CLEANUP_TIMEOUT_MS = 3_000;

const REMEMBERED_SUBSCRIPTION_KEY = "stellarr.push-subscription";

export type PushFailureCode =
  | "unsupported"
  | "invalid-key"
  | "worker"
  | "subscribe"
  | "server"
  | "cancelled";

export class PushEnrollmentError extends Error {
  readonly code: PushFailureCode;

  constructor(code: PushFailureCode, cause?: unknown) {
    super(`Web Push enrollment failed: ${code}`);
    this.name = "PushEnrollmentError";
    this.code = code;
    if (cause !== undefined) this.cause = cause;
  }
}

export interface RememberedSubscription {
  userId: string;
  endpoint: string;
}

export type DevicePushInspection =
  | { state: "unsupported" }
  | { state: "blocked" }
  | { state: "idle" }
  | { state: "enrolled"; endpoint: string }
  | { state: "stale"; endpoint: string };

export interface EnrollPushOptions {
  userId: string;
  applicationServerKey: Uint8Array<ArrayBuffer>;
  /** Binds the endpoint on the server; resolves to the row id. */
  save: (input: PushSubscriptionInput) => Promise<string>;
  /** Forgets an older endpoint this browser owned before replacing it. */
  deleteEndpoint: (endpoint: string) => Promise<void>;
  /** Checked after the server accepted the endpoint; false rolls it back. */
  isStillWanted?: () => boolean;
}

export interface EnrolledPush {
  id: string;
  endpoint: string;
}

function runnable(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof navigator !== "undefined" &&
    "serviceWorker" in navigator &&
    typeof window.PushManager === "function" &&
    typeof window.Notification === "function"
  );
}

/** Whether this browser has the three pieces Web Push needs. */
export function pushManagerSupported(): boolean {
  return runnable();
}

export function deviceNotificationPermission(): NotificationPermission | "unsupported" {
  return runnable() ? window.Notification.permission : "unsupported";
}

/**
 * Starts the permission prompt now. Callers invoke this directly from the
 * click that asked for enrollment, before awaiting anything else: browsers
 * only treat the synchronous call as a gesture-backed request.
 */
export function requestDeviceNotificationPermission(): Promise<NotificationPermission | "unsupported"> {
  if (!runnable()) return Promise.resolve("unsupported");
  return window.Notification.requestPermission();
}

/** The worker URL as an absolute same-origin address. */
function workerUrl(): string {
  return new URL(WORKER_PATH, window.location.href).href;
}

function isOurWorker(registration: ServiceWorkerRegistration): boolean {
  const expected = workerUrl();
  return [registration.active, registration.waiting, registration.installing].some(
    (worker) => worker?.scriptURL === expected,
  );
}

function timeout<T>(promise: Promise<T>, ms: number, code: PushFailureCode): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = window.setTimeout(
      () => reject(new PushEnrollmentError(code)),
      ms,
    );
    promise.then(
      (value) => {
        window.clearTimeout(timer);
        resolve(value);
      },
      (cause: unknown) => {
        window.clearTimeout(timer);
        reject(cause);
      },
    );
  });
}

/** An existing registration for our worker, without registering anything. */
export async function getNotificationWorkerRegistration(): Promise<ServiceWorkerRegistration | null> {
  if (!runnable()) return null;
  try {
    const registration = await timeout(
      navigator.serviceWorker.getRegistration(WORKER_PATH),
      INSPECTION_TIMEOUT_MS,
      "worker",
    );
    return registration !== undefined && isOurWorker(registration) ? registration : null;
  } catch {
    return null;
  }
}

/** Waits for the registration returned by `register`, never for `ready`. */
async function waitForActiveWorker(
  registration: ServiceWorkerRegistration,
): Promise<ServiceWorkerRegistration> {
  if (registration.active !== null) return registration;

  await new Promise<void>((resolve, reject) => {
    const timer = window.setTimeout(
      () => reject(new PushEnrollmentError("worker")),
      WORKER_TIMEOUT_MS,
    );
    const workers = [registration.installing, registration.waiting].filter(
      (worker): worker is ServiceWorker => worker !== null,
    );
    const settle = () => {
      if (registration.active === null) return;
      window.clearTimeout(timer);
      for (const worker of workers) worker.removeEventListener("statechange", settle);
      resolve();
    };
    for (const worker of workers) worker.addEventListener("statechange", settle);
    settle();
  });

  if (!isOurWorker(registration)) throw new PushEnrollmentError("worker");
  return registration;
}

async function registerWorker(): Promise<ServiceWorkerRegistration> {
  if (!runnable()) throw new PushEnrollmentError("unsupported");
  let registration: ServiceWorkerRegistration;
  try {
    registration = await timeout(
      navigator.serviceWorker.register(WORKER_PATH, {
        scope: WORKER_SCOPE,
      }),
      WORKER_TIMEOUT_MS,
      "worker",
    );
  } catch (cause) {
    throw cause instanceof PushEnrollmentError ? cause : new PushEnrollmentError("worker", cause);
  }
  return waitForActiveWorker(registration);
}

/** Decodes the VAPID key before asking for permission, so a bad key never prompts. */
export function decodeApplicationServerKey(value: string): Uint8Array<ArrayBuffer> {
  const normalized = value.replaceAll("-", "+").replaceAll("_", "/");
  const padding = normalized.length % 4;
  const padded = padding === 0 ? normalized : normalized + "=".repeat(4 - padding);
  try {
    const binary = window.atob(padded);
    const bytes = new Uint8Array(new ArrayBuffer(binary.length));
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
    if (bytes.length === 0) throw new Error("empty key");
    return bytes;
  } catch (cause) {
    throw new PushEnrollmentError("invalid-key", cause);
  }
}

function remembered(): RememberedSubscription | null {
  try {
    const raw = window.localStorage.getItem(REMEMBERED_SUBSCRIPTION_KEY);
    if (raw === null) return null;
    const stored: unknown = JSON.parse(raw);
    if (typeof stored !== "object" || stored === null) return null;
    const { userId, endpoint } = stored as Record<string, unknown>;
    return typeof userId === "string" && typeof endpoint === "string" && endpoint.length > 0
      ? { userId, endpoint }
      : null;
  } catch {
    return null;
  }
}

function remember(userId: string, endpoint: string): void {
  try {
    window.localStorage.setItem(
      REMEMBERED_SUBSCRIPTION_KEY,
      JSON.stringify({ userId, endpoint }),
    );
  } catch {
    // Storage may be unavailable; the browser subscription still works.
  }
}

export function forgetRememberedSubscription(): void {
  try {
    window.localStorage.removeItem(REMEMBERED_SUBSCRIPTION_KEY);
  } catch {
    // Nothing to forget when storage is unavailable.
  }
}

function payloadOf(subscription: PushSubscription): PushSubscriptionInput {
  const json = subscription.toJSON();
  const endpoint = json.endpoint ?? subscription.endpoint;
  const p256dh = json.keys?.p256dh;
  const auth = json.keys?.auth;
  if (
    typeof endpoint !== "string" ||
    endpoint.length === 0 ||
    typeof p256dh !== "string" ||
    p256dh.length === 0 ||
    typeof auth !== "string" ||
    auth.length === 0
  ) {
    throw new PushEnrollmentError("subscribe");
  }
  return { endpoint, keys: { p256dh, auth } };
}

async function unsubscribeQuietly(subscription: PushSubscription): Promise<void> {
  try {
    await subscription.unsubscribe();
  } catch {
    // A subscription that cannot be removed is reported by the enrollment
    // step that needed it; cleanup never hides the original failure.
  }
}

/**
 * What this browser has now. Inspection is read-only: it never registers the
 * worker and never subscribes, so opening settings cannot prompt the person.
 */
export async function inspectDevicePush(userId: string): Promise<DevicePushInspection> {
  if (!runnable()) return { state: "unsupported" };
  if (deviceNotificationPermission() === "denied") return { state: "blocked" };

  const registration = await getNotificationWorkerRegistration();
  if (registration === null) return { state: "idle" };

  let subscription: PushSubscription | null;
  try {
    subscription = await timeout(
      registration.pushManager.getSubscription(),
      INSPECTION_TIMEOUT_MS,
      "worker",
    );
  } catch {
    return { state: "idle" };
  }
  if (subscription === null) return { state: "idle" };

  const known = remembered();
  return known !== null &&
    known.userId === userId &&
    known.endpoint === subscription.endpoint
    ? { state: "enrolled", endpoint: subscription.endpoint }
    : { state: "stale", endpoint: subscription.endpoint };
}

/**
 * Registers the worker, replaces any subscription this browser already held,
 * and subscribes under the current account. A rejected server save removes
 * the newly created local subscription before the error reaches the pane.
 */
export async function enrollDevicePush(options: EnrollPushOptions): Promise<EnrolledPush> {
  if (!runnable()) throw new PushEnrollmentError("unsupported");

  const registration = await registerWorker();
  let existing: PushSubscription | null = null;
  try {
    existing = await timeout(
      registration.pushManager.getSubscription(),
      INSPECTION_TIMEOUT_MS,
      "worker",
    );
  } catch {
    existing = null;
  }

  if (existing !== null) {
    // The endpoint is about to be replaced. The server scopes the deletion to
    // this account, so another owner's row is untouched.
    try {
      await timeout(
        options.deleteEndpoint(existing.endpoint),
        CLEANUP_TIMEOUT_MS,
        "server",
      );
    } catch {
      // Local replacement still has to happen; the server's session binding
      // remains authoritative for the old row.
    }
    await unsubscribeQuietly(existing);
    forgetRememberedSubscription();
  }

  let subscription: PushSubscription;
  try {
    subscription = await timeout(
      registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: options.applicationServerKey,
      }),
      SUBSCRIBE_TIMEOUT_MS,
      "subscribe",
    );
  } catch (cause) {
    throw cause instanceof PushEnrollmentError ? cause : new PushEnrollmentError("subscribe", cause);
  }

  let payload: PushSubscriptionInput;
  try {
    payload = payloadOf(subscription);
  } catch (cause) {
    await unsubscribeQuietly(subscription);
    throw cause instanceof PushEnrollmentError
      ? cause
      : new PushEnrollmentError("subscribe", cause);
  }

  if (options.isStillWanted !== undefined && !options.isStillWanted()) {
    await unsubscribeQuietly(subscription);
    throw new PushEnrollmentError("cancelled");
  }

  try {
    const id = await timeout(options.save(payload), SUBSCRIBE_TIMEOUT_MS, "server");
    if (options.isStillWanted !== undefined && !options.isStillWanted()) {
      await timeout(
        options.deleteEndpoint(payload.endpoint),
        CLEANUP_TIMEOUT_MS,
        "server",
      ).catch(() => undefined);
      await unsubscribeQuietly(subscription);
      throw new PushEnrollmentError("cancelled");
    }
    remember(options.userId, payload.endpoint);
    return { id, endpoint: payload.endpoint };
  } catch (cause) {
    if (cause instanceof PushEnrollmentError && cause.code === "cancelled") throw cause;
    // The server refused the row: take the new browser subscription back so
    // the device does not keep a subscription nobody is bound to.
    await unsubscribeQuietly(subscription);
    forgetRememberedSubscription();
    throw cause instanceof PushEnrollmentError
      ? cause
      : new PushEnrollmentError("server", cause);
  }
}

export interface DisablePushResult {
  /** Whether a server row was removed; false means local removal still happened. */
  serverRemoved: boolean;
  removedLocally: boolean;
}

/** Turns this device off, server row first, local subscription in every case. */
export async function disableDevicePush(options: {
  userId: string;
  deleteEndpoint: (endpoint: string) => Promise<void>;
}): Promise<DisablePushResult> {
  const registration = await getNotificationWorkerRegistration();
  if (registration === null) {
    forgetRememberedSubscription();
    return { serverRemoved: true, removedLocally: false };
  }

  let subscription: PushSubscription | null = null;
  try {
    subscription = await registration.pushManager.getSubscription();
  } catch {
    subscription = null;
  }
  if (subscription === null) {
    forgetRememberedSubscription();
    return { serverRemoved: true, removedLocally: false };
  }

  const known = remembered();
  let serverRemoved = true;
  // A row for another owner is not ours to remove; deleting would be refused
  // or a no-op, and the server decides. Attempt it only for our own memory.
  if (known !== null && known.userId === options.userId && known.endpoint === subscription.endpoint) {
    try {
      await timeout(
        options.deleteEndpoint(subscription.endpoint),
        CLEANUP_TIMEOUT_MS,
        "server",
      );
    } catch {
      serverRemoved = false;
    }
  }

  try {
    await subscription.unsubscribe();
  } finally {
    forgetRememberedSubscription();
  }
  return { serverRemoved, removedLocally: true };
}

/**
 * Removes whatever this browser holds, with no user action. Used before the
 * session ends and after it was ended by the server or a revocation.
 */
export async function releaseDevicePush(options: {
  deleteEndpoint: (endpoint: string) => Promise<void>;
}): Promise<void> {
  let subscription: PushSubscription | null = null;
  if (runnable()) {
    try {
      const registration = await getNotificationWorkerRegistration();
      subscription = registration === null
        ? null
        : await registration.pushManager.getSubscription();
    } catch {
      subscription = null;
    }
  }

  if (subscription === null) {
    forgetRememberedSubscription();
    return;
  }

  try {
    try {
      await timeout(
        options.deleteEndpoint(subscription.endpoint),
        CLEANUP_TIMEOUT_MS,
        "server",
      );
    } catch {
      // The session may already be invalid; the local removal below is still
      // required so a signed-out browser cannot keep receiving for it.
    }
  } finally {
    try {
      await subscription.unsubscribe();
    } finally {
      forgetRememberedSubscription();
    }
  }
}
