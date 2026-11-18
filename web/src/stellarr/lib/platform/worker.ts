/**
 * The web client's one service worker. An origin's scope has room for one,
 * and a second script registered for it would replace the first, so a single
 * file, /stellarr-worker.js, serves Web Push and keeps the shell's local copy.
 * Everyone registers it at start; push enrolment registers it again, which
 * changes nothing.
 */
export const WORKER_PATH = "/stellarr-worker.js";
export const WORKER_SCOPE = "/";

/** What a page sends the worker to have it take the release the server serves. */
export const ADOPT_MESSAGE = "stellarr-adopt";

/** How long a page waits for a worker to be active before it goes on without one. */
const READY_TIMEOUT_MS = 3_000;

/** A shell is a few dozen files; this is a stalled network, not a slow one. */
const ADOPT_TIMEOUT_MS = 30_000;

function supported(): boolean {
  return typeof navigator !== "undefined" && "serviceWorker" in navigator;
}

/** Registers the worker for this page's scope. A browser that refuses has no worker, and nothing else changes. */
export async function registerWorker(): Promise<void> {
  if (!supported()) return;
  try {
    await navigator.serviceWorker.register(WORKER_PATH, { scope: WORKER_SCOPE });
  } catch {
    // Private windows and the like: the page works as it does without a worker.
  }
}

/** What `promise` gives, or null when it fails or is not done within `ms`. */
function within<T>(promise: Promise<T>, ms: number): Promise<T | null> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve(null), ms);
    const settle = (value: T | null) => {
      clearTimeout(timer);
      resolve(value);
    };
    promise.then(settle, () => settle(null));
  });
}

/** Asks the worker to take the release; true when it says the shell is stored and switched to. */
function askWorker(worker: ServiceWorker): Promise<boolean> {
  return new Promise((resolve) => {
    const channel = new MessageChannel();
    channel.port1.onmessage = (event: MessageEvent<{ ok?: unknown }>) => {
      channel.port1.close();
      resolve(event.data.ok === true);
    };
    worker.postMessage({ type: ADOPT_MESSAGE }, [channel.port2]);
  });
}

/**
 * Has the worker keep the shell of the release the server serves now: it
 * stores every file of it and switches to it before it answers. True means
 * the page may go on to reload, because the worker has switched or there is
 * no worker to ask and a plain reload is all there is. False means a worker
 * is there and could not, so a reload would only bring its old shell back.
 */
export async function keepShell(): Promise<boolean> {
  if (!supported()) return true;
  const registration = await within(navigator.serviceWorker.ready, READY_TIMEOUT_MS);
  const worker = registration?.active ?? null;
  if (worker === null) return true;
  return (await within(askWorker(worker), ADOPT_TIMEOUT_MS)) === true;
}
