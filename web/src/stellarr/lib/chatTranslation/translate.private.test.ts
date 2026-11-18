import "fake-indexeddb/auto";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./detectLanguage", () => ({ detectLanguage: vi.fn(() => "pl") }));

interface WorkerRequest {
  id: number;
  text: string;
}

/** Answers every request at once, the way public/bergamot-worker.js does. */
class EchoWorker {
  static requests: WorkerRequest[] = [];
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  onmessageerror: (() => void) | null = null;

  postMessage(request: WorkerRequest) {
    EchoWorker.requests.push(request);
    queueMicrotask(() =>
      this.onmessage?.({ data: { type: "result", id: request.id, translation: `EN: ${request.text}` } } as MessageEvent),
    );
  }

  terminate() {}
}

function deleteDatabase(): Promise<void> {
  return new Promise((resolve) => {
    const request = indexedDB.deleteDatabase("stellarr-chat-translations");
    request.onsuccess = () => resolve();
    request.onerror = () => resolve();
  });
}

/** What a page loaded after this one would find on disk. */
async function storedAfterReload(text: string): Promise<string | undefined> {
  vi.resetModules();
  const cache = await import("./translationCache");
  return cache.get(cache.cacheKey(text, "en"));
}

describe("translations of encrypted messages", () => {
  beforeEach(async () => {
    EchoWorker.requests = [];
    vi.stubGlobal("Worker", EchoWorker);
    await deleteDatabase();
    vi.resetModules();
  });
  afterEach(() => vi.unstubAllGlobals());

  it("stay in this tab's memory and never reach the stored cache", async () => {
    const { translate } = await import("./translate");

    await expect(translate("login do klubu", "en", { persist: false })).resolves.toBe("EN: login do klubu");
    await expect(translate("login do klubu", "en", { persist: false })).resolves.toBe("EN: login do klubu");

    expect(EchoWorker.requests).toHaveLength(1);
    expect(await storedAfterReload("login do klubu")).toBeUndefined();
  });

  it("while a plain message's translation is still stored for the next visit", async () => {
    const { translate } = await import("./translate");

    await translate("dzień dobry", "en");

    expect(await storedAfterReload("dzień dobry")).toBe("EN: dzień dobry");
  });

  it("are all forgotten, stored or not, when the cache is cleared", async () => {
    const { translate } = await import("./translate");
    const cache = await import("./translationCache");
    await translate("dzień dobry", "en");
    await translate("login do klubu", "en", { persist: false });

    await cache.clearAll();

    expect(cache.getSync(cache.cacheKey("login do klubu", "en"))).toBeUndefined();
    expect(cache.getSync(cache.cacheKey("dzień dobry", "en"))).toBeUndefined();
    expect(await storedAfterReload("dzień dobry")).toBeUndefined();
  });
});
