import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { runInNewContext } from "node:vm";

import { describe, expect, it, vi } from "vitest";

import { WORKER_PATH } from "./worker";

const SOURCE = readFileSync(resolve(process.cwd(), `public${WORKER_PATH}`), "utf8");
const ORIGIN = "https://chat.example";
const DIRECTORIES = ["audio", "bergamot-models", "codecs", "fonts", "identity", "mascot", "sounds", "static"];

type Listener = (event: Record<string, unknown>) => void;

/** A Cache that logs what is stored in it, so the order of an update can be read back. */
class FakeCache {
  readonly entries = new Map<string, Response>();

  constructor(
    private readonly name: string,
    private readonly log: string[],
    private readonly refuse: (name: string, path: string) => boolean,
  ) {}

  async match(path: string): Promise<Response | undefined> {
    return this.entries.get(path)?.clone();
  }

  async put(path: string, response: Response): Promise<void> {
    if (this.refuse(this.name, path)) throw new Error("The cache is full.");
    this.entries.set(path, response);
    this.log.push(`put ${this.name} ${path}`);
  }
}

class FakeCaches {
  readonly stores = new Map<string, FakeCache>();
  readonly log: string[] = [];
  refuse: (name: string, path: string) => boolean = () => false;

  async open(name: string): Promise<FakeCache> {
    let cache = this.stores.get(name);
    if (cache === undefined) {
      cache = new FakeCache(name, this.log, (cacheName, path) => this.refuse(cacheName, path));
      this.stores.set(name, cache);
    }
    return cache;
  }

  async delete(name: string): Promise<boolean> {
    this.log.push(`delete ${name}`);
    return this.stores.delete(name);
  }

  async keys(): Promise<string[]> {
    return [...this.stores.keys()];
  }
}

/** A release as the server hands it out: its manifest and the files of its shell. */
function release(id: string): { id: string; files: Record<string, string> } {
  return {
    id,
    files: { "/index.html": `document of ${id}`, "/static/app.js": `app of ${id}`, "/fonts/outfit.woff2": `font of ${id}` },
  };
}

function loadWorker(initial = release("r1")) {
  const listeners = new Map<string, Listener>();
  const caches = new FakeCaches();
  const network = {
    release: initial,
    offline: new Set<string>(),
    requests: [] as { path: string; init: RequestInit | undefined }[],
    instance: "window.stellarrInstanceLanguage = 'pl';",
  };
  const fetchMock = vi.fn(async (input: string | { url: string }, init?: RequestInit) => {
    const path = typeof input === "string" ? input : new URL(input.url).pathname;
    network.requests.push({ path, init });
    if (network.offline.has(path)) throw new TypeError("Failed to fetch");
    if (path === "/release.json") {
      const { id, files } = network.release;
      return Response.json({ id, shell: Object.keys(files), directories: DIRECTORIES });
    }
    if (path === "/stellarr-instance.js") return new Response(network.instance);
    const body = network.release.files[path];
    return body === undefined ? new Response("", { status: 404 }) : new Response(body);
  });
  const self = {
    location: { origin: ORIGIN },
    addEventListener: (type: string, listener: Listener) => {
      listeners.set(type, listener);
    },
    skipWaiting: vi.fn(async () => undefined),
    registration: { showNotification: vi.fn(async () => undefined) },
    clients: { matchAll: vi.fn(async () => []), openWindow: vi.fn(), claim: vi.fn(async () => undefined) },
  };
  const context = { self, caches, fetch: fetchMock, Response, URL, JSON, Array, Object, String, Error, Promise, console };
  runInNewContext(SOURCE, context);

  /** What the worker answers a request with, or null when it leaves the request to the network. */
  async function ask(path: string, mode = "navigate", method = "GET"): Promise<string | null> {
    const event = { request: { method, mode, url: `${ORIGIN}${path}` }, respondWith: vi.fn(), waitUntil: vi.fn() };
    listeners.get("fetch")?.(event);
    const answer = event.respondWith.mock.calls[0]?.[0] as Promise<Response> | undefined;
    return answer === undefined ? null : (await answer).text();
  }

  /** Sends the worker the message of a page and gives back what it replies. */
  async function adopt(): Promise<{ ok: boolean; id?: string }> {
    const port = { postMessage: vi.fn() };
    const event = { data: { type: "stellarr-adopt" }, ports: [port], waitUntil: vi.fn() };
    listeners.get("message")?.(event);
    await event.waitUntil.mock.calls[0]?.[0];
    return port.postMessage.mock.calls[0]?.[0] as { ok: boolean; id?: string };
  }

  return { context, listeners, caches, network, self, ask, adopt };
}

type Route = "document" | "file" | "instance" | "network";

describe("which requests the worker answers", () => {
  const { context } = loadWorker();
  const routeOf = context as unknown as {
    routeOf(method: string, mode: string, url: URL, origin: string, directories: string[] | null): Route;
  };
  const route = (path: string, mode = "navigate", directories: string[] | null = DIRECTORIES, method = "GET"): Route =>
    routeOf.routeOf(method, mode, new URL(ORIGIN + path), ORIGIN, directories);

  it.each(["/", "/login", "/invite/abc", "/channels/general/threads", "/login/"])("answers a navigation to %s with the document", (path) => {
    expect(route(path)).toBe("document");
  });

  it.each(["/cockpit", "/cockpit/users", "//cockpit/users"])("leaves a navigation to Cockpit at %s to the network", (path) => {
    expect(route(path)).toBe("network");
  });

  it.each(["/api/auth/me", "/api/auth/oidc/callback", "/hubs/chat", "/assets/avatars/a.png", "/health", "/.well-known/stellarr/client", "//api/x"])(
    "never touches %s, a navigation or not",
    (path) => {
      expect(route(path)).toBe("network");
      expect(route(path, "cors")).toBe("network");
    },
  );

  it.each(["/audio/detector-worklet.js", "/sounds/calling.wav", "/codecs/x.wasm", "/index.html"])("leaves the file %s to the network", (path) => {
    expect(route(path)).toBe("network");
    expect(route(path, "no-cors")).toBe("network");
  });

  it("answers the files of the shell, and only those folders", () => {
    expect(route("/static/app.js", "no-cors")).toBe("file");
    expect(route("/fonts/outfit.woff2", "cors")).toBe("file");
  });

  it("answers nothing that is not a plain read from this origin", () => {
    expect(route("/login", "navigate", DIRECTORIES, "POST")).toBe("network");
    expect(routeOf.routeOf("GET", "navigate", new URL("https://other.example/login"), ORIGIN, DIRECTORIES)).toBe("network");
  });

  it("answers the instance file, and leaves the rest to the network while no shell is kept", () => {
    expect(route("/stellarr-instance.js", "no-cors", null)).toBe("instance");
    expect(route("/login", "navigate", null)).toBe("network");
    expect(route("/static/app.js", "no-cors", null)).toBe("network");
  });
});

describe("answering from the kept shell", () => {
  it("passes every request to the network before a release is taken", async () => {
    const worker = loadWorker();
    await worker.ask("/login");
    expect(worker.network.requests.map(({ path }) => path)).toEqual(["/login"]);
  });

  it("answers a navigation and the files of the shell without asking the network", async () => {
    const worker = loadWorker();
    await worker.adopt();
    worker.network.requests.length = 0;
    await expect(worker.ask("/")).resolves.toBe("document of r1");
    await expect(worker.ask("/invite/abc")).resolves.toBe("document of r1");
    await expect(worker.ask("/static/app.js", "no-cors")).resolves.toBe("app of r1");
    expect(worker.network.requests).toEqual([]);
  });

  it("lets Cockpit, the API, the hub and the files outside the shell go to the network", async () => {
    const worker = loadWorker();
    await worker.adopt();
    for (const path of ["/cockpit", "/cockpit/users", "/api/auth/me", "/hubs/chat", "/audio/detector-worklet.js"]) {
      await expect(worker.ask(path)).resolves.toBeNull();
    }
    await expect(worker.ask("/api/chat/send", "cors", "POST")).resolves.toBeNull();
  });

  it("falls back to the network for a file the copy does not have", async () => {
    const worker = loadWorker();
    await worker.adopt();
    worker.network.release.files["/static/late.js"] = "late";
    await expect(worker.ask("/static/late.js", "no-cors")).resolves.toBe("late");
  });
});

describe("taking a new release", () => {
  const stored = (log: string[], cache: string) => log.filter((entry) => entry.startsWith(`put ${cache} `));

  it("stores the whole new shell before it switches, then forgets the old one", async () => {
    const worker = loadWorker(release("r1"));
    await worker.adopt();
    worker.caches.log.length = 0;
    worker.network.release = release("r2");

    await expect(worker.adopt()).resolves.toEqual({ ok: true, id: "r2" });

    const { log } = worker.caches;
    const switched = log.indexOf("put stellarr-kept /current");
    expect(stored(log, "stellarr-shell:r2")).toHaveLength(3);
    for (const entry of stored(log, "stellarr-shell:r2")) expect(log.indexOf(entry)).toBeLessThan(switched);
    expect(log.indexOf("delete stellarr-shell:r1")).toBeGreaterThan(switched);
    expect([...worker.caches.stores.keys()].sort()).toEqual(["stellarr-kept", "stellarr-shell:r2"]);
    await expect(worker.ask("/")).resolves.toBe("document of r2");
  });

  it("keeps the old shell and deletes the partial one when a file cannot be fetched", async () => {
    const worker = loadWorker(release("r1"));
    await worker.adopt();
    worker.network.release = release("r2");
    worker.network.offline.add("/static/app.js");

    await expect(worker.adopt()).resolves.toEqual({ ok: false });

    expect([...worker.caches.stores.keys()].sort()).toEqual(["stellarr-kept", "stellarr-shell:r1"]);
    await expect(worker.ask("/")).resolves.toBe("document of r1");
  });

  it("keeps the old shell when a file cannot be stored", async () => {
    const worker = loadWorker(release("r1"));
    await worker.adopt();
    worker.network.release = release("r2");
    worker.caches.refuse = (name, path) => name === "stellarr-shell:r2" && path === "/fonts/outfit.woff2";

    await expect(worker.adopt()).resolves.toEqual({ ok: false });

    await expect(worker.ask("/")).resolves.toBe("document of r1");
    expect(worker.caches.stores.has("stellarr-shell:r2")).toBe(false);
  });

  it("refuses a document that belongs to another release than the one it was listed for", async () => {
    const worker = loadWorker(release("r1"));
    await worker.adopt();
    worker.network.release = { ...release("r2"), files: { ...release("r2").files, "/index.html": "document of r3" } };

    await expect(worker.adopt()).resolves.toEqual({ ok: false });
    await expect(worker.ask("/")).resolves.toBe("document of r1");
  });

  it("keeps nothing when the server does not answer", async () => {
    const worker = loadWorker(release("r1"));
    worker.network.offline.add("/release.json");
    await expect(worker.adopt()).resolves.toEqual({ ok: false });
    expect(worker.caches.stores.has("stellarr-shell:r1")).toBe(false);
  });

  it("stores the shell again when the pointer names a cache that is gone or empty", async () => {
    const worker = loadWorker(release("r1"));
    await worker.adopt();
    await worker.caches.delete("stellarr-shell:r1");
    await worker.caches.open("stellarr-shell:r1");
    worker.network.requests.length = 0;

    await expect(worker.adopt()).resolves.toEqual({ ok: true, id: "r1" });

    expect(worker.network.requests.map(({ path }) => path)).toContain("/static/app.js");
    await expect(worker.ask("/")).resolves.toBe("document of r1");
  });

  it("does not fetch the shell again for the release it already keeps", async () => {
    const worker = loadWorker(release("r1"));
    await worker.adopt();
    worker.network.requests.length = 0;
    await expect(worker.adopt()).resolves.toEqual({ ok: true, id: "r1" });
    expect(worker.network.requests.map(({ path }) => path)).toEqual(["/release.json"]);
  });
});

describe("the instance file", () => {
  const instance = (worker: ReturnType<typeof loadWorker>) => worker.ask("/stellarr-instance.js", "no-cors");
  const settle = () => new Promise((done) => setTimeout(done, 0));

  it("is fetched in the ordinary way and kept when there is no copy yet", async () => {
    const worker = loadWorker();
    await expect(instance(worker)).resolves.toBe(worker.network.instance);
    const [request] = worker.network.requests;
    expect(request?.init).toBeUndefined();
    expect(worker.caches.stores.get("stellarr-kept")?.entries.has("/stellarr-instance.js")).toBe(true);
  });

  it("is answered from the copy at once and refreshed behind it", async () => {
    const worker = loadWorker();
    await instance(worker);
    worker.network.instance = "changed";
    worker.network.requests.length = 0;
    await expect(instance(worker)).resolves.toBe("window.stellarrInstanceLanguage = 'pl';");
    await settle();
    expect(worker.network.requests.map(({ path }) => path)).toEqual(["/stellarr-instance.js"]);
    await expect(instance(worker)).resolves.toBe("changed");
  });

  it("is still answered from the copy when the network is gone", async () => {
    const worker = loadWorker();
    await instance(worker);
    worker.network.offline.add("/stellarr-instance.js");
    await expect(instance(worker)).resolves.toBe(worker.network.instance);
  });

  it("fails as the network does when there is no copy", async () => {
    const worker = loadWorker();
    worker.network.offline.add("/stellarr-instance.js");
    await expect(instance(worker)).rejects.toThrow("Failed to fetch");
  });
});

describe("the worker as a whole", () => {
  it("still handles push and clicks, and takes control at once", () => {
    const worker = loadWorker();
    expect([...worker.listeners.keys()].sort()).toEqual(["activate", "fetch", "install", "message", "notificationclick", "push"]);
  });

  it("deletes the copies a crash left behind when it activates", async () => {
    const worker = loadWorker();
    await worker.adopt();
    await worker.caches.open("stellarr-shell:half-stored");
    const event = { waitUntil: vi.fn() };
    worker.listeners.get("activate")?.(event);
    await event.waitUntil.mock.calls[0]?.[0];
    expect([...worker.caches.stores.keys()].sort()).toEqual(["stellarr-kept", "stellarr-shell:r1"]);
    expect(worker.self.clients.claim).toHaveBeenCalled();
  });
});
