import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { runInNewContext } from "node:vm";

import { describe, expect, it, vi } from "vitest";

const SOURCE = readFileSync(
  resolve(process.cwd(), "public/notification-worker.js"),
  "utf8",
);

interface TestClient {
  url: string;
  visibilityState: "visible" | "hidden";
  focused: boolean;
  focus: ReturnType<typeof vi.fn>;
  navigate: ReturnType<typeof vi.fn>;
}

function client(url: string, visible = false): TestClient {
  return {
    url,
    visibilityState: visible ? "visible" : "hidden",
    focused: visible,
    focus: vi.fn(async () => undefined),
    navigate: vi.fn(async () => undefined),
  };
}

function loadWorker(windows: TestClient[] = []) {
  const listeners = new Map<string, (event: Record<string, unknown>) => void>();
  const shown: { title: string; options: NotificationOptions }[] = [];
  const openWindow = vi.fn(async (url: string) => ({ url }));
  const matchAll = vi.fn(async () => windows);
  const self = {
    location: { origin: "https://chat.example" },
    addEventListener: (type: string, listener: (event: Record<string, unknown>) => void) => {
      listeners.set(type, listener);
    },
    skipWaiting: vi.fn(async () => undefined),
    registration: {
      showNotification: vi.fn(async (title: string, options: NotificationOptions) => {
        shown.push({ title, options });
      }),
    },
    clients: {
      matchAll,
      openWindow,
      claim: vi.fn(async () => undefined),
    },
  };

  runInNewContext(SOURCE, { self, URL, JSON, Array, Object, String, Error, console });

  const listener = (name: string) => {
    const found = listeners.get(name);
    if (found === undefined) throw new Error(`No ${name} listener`);
    return found;
  };

  return { self, shown, openWindow, matchAll, listener };
}

function pushEvent(payload: unknown) {
  const event = {
    data: {
      json: () => payload,
      text: () => JSON.stringify(payload),
    },
    waitUntil: vi.fn(),
  };
  return event;
}

async function push(worker: ReturnType<typeof loadWorker>, payload: unknown) {
  const event = pushEvent(payload);
  worker.listener("push")(event);
  await event.waitUntil.mock.calls[0]?.[0];
  return event;
}

function clickEvent(url: string) {
  const close = vi.fn();
  const event = {
    notification: { close, data: { url } },
    waitUntil: vi.fn(),
  };
  return { event, close };
}

describe("notification worker push handling", () => {
  it("shows a valid payload with the app icon, safe data and its dedupe tag", async () => {
    const worker = loadWorker();

    await push(worker, {
      id: "m-1",
      title: "Nils mentions you",
      body: "Look at this",
      url: "/channel/general",
      tag: "stellarr-message:m-1",
    });

    expect(worker.shown).toHaveLength(1);
    expect(worker.shown[0]).toMatchObject({
      title: "Nils mentions you",
      options: {
        body: "Look at this",
        tag: "stellarr-message:m-1",
        icon: "/notification-icon.svg",
        data: { url: "/channel/general", notificationId: "m-1" },
      },
    });
  });

  it("falls back to a stable tag when the payload supplies none", async () => {
    const worker = loadWorker();

    await push(worker, { id: "n-1", title: "A direct message", body: "Hello" });

    expect(worker.shown[0]?.options.tag).toBe("stellarr-push:n-1");
    expect(worker.shown[0]?.options.data).toMatchObject({ url: "/" });
  });

  it("ignores malformed JSON, a non-object, and a payload without a title", async () => {
    const worker = loadWorker();
    const broken = {
      data: {
        json: () => {
          throw new Error("not json");
        },
        text: () => {
          throw new Error("not text");
        },
      },
      waitUntil: vi.fn(),
    };

    worker.listener("push")(broken);
    await push(worker, ["not", "an", "object"]);
    await push(worker, { id: "n-1", body: "no title" });

    expect(broken.waitUntil).not.toHaveBeenCalled();
    expect(worker.shown).toHaveLength(0);
  });

  it("truncates overlong titles and bodies instead of showing unbounded text", async () => {
    const worker = loadWorker();

    await push(worker, { id: "n-1", title: "x".repeat(400), body: "y".repeat(900) });

    expect(worker.shown[0]?.title).toHaveLength(200);
    expect(String(worker.shown[0]?.options.body)).toHaveLength(500);
  });

  it("suppresses a duplicate when a visible focused app client is present", async () => {
    const visible = client("https://chat.example/channel/general", true);
    const worker = loadWorker([visible]);

    await push(worker, { id: "m-1", title: "Nils mentions you", body: "Look" });

    expect(worker.shown).toHaveLength(0);
  });

  it("still delivers when the app is open only in the background", async () => {
    const hidden = client("https://chat.example/channel/general", false);
    const worker = loadWorker([hidden]);

    await push(worker, { id: "m-1", title: "Nils mentions you", body: "Look" });

    expect(worker.shown).toHaveLength(1);
  });
});

describe("notification worker click handling", () => {
  it("focuses the client already showing the target", async () => {
    const matching = client("https://chat.example/channel/general", true);
    const worker = loadWorker([matching]);
    const { event, close } = clickEvent("/channel/general");

    worker.listener("notificationclick")(event);
    await event.waitUntil.mock.calls[0]?.[0];

    expect(close).toHaveBeenCalledOnce();
    expect(matching.focus).toHaveBeenCalledOnce();
    expect(worker.openWindow).not.toHaveBeenCalled();
  });

  it("brings an existing app window forward for the home page", async () => {
    const elsewhere = client("https://chat.example/login", false);
    const worker = loadWorker([elsewhere]);
    const { event } = clickEvent("/");

    worker.listener("notificationclick")(event);
    await event.waitUntil.mock.calls[0]?.[0];

    expect(elsewhere.focus).toHaveBeenCalledOnce();
    expect(worker.openWindow).not.toHaveBeenCalled();
  });

  it("opens a same-origin target when no client shows it", async () => {
    const worker = loadWorker();
    const { event } = clickEvent("https://chat.example/channel/general?at=1");

    worker.listener("notificationclick")(event);
    await event.waitUntil.mock.calls[0]?.[0];

    expect(worker.openWindow).toHaveBeenCalledExactlyOnceWith("/channel/general?at=1");
  });

  it("never opens an arbitrary cross-origin destination", async () => {
    const worker = loadWorker();
    const { event } = clickEvent("https://evil.example/steal");

    worker.listener("notificationclick")(event);
    await event.waitUntil.mock.calls[0]?.[0];

    expect(worker.openWindow).toHaveBeenCalledExactlyOnceWith("/");
  });
});
