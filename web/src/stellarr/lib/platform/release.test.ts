import { afterEach, describe, expect, it, vi } from "vitest";

import type { ConnectionPort, ConnectionStatus } from "@/ports/presence";

import {
  createReleaseWatcher,
  fetchLatestRelease,
  runningRelease,
  VISIBLE_CHECK_INTERVAL_MS,
  watchRelease,
  type ReleaseWatcherDeps,
} from "./release";

function watcherWith(overrides: Partial<ReleaseWatcherDeps> = {}) {
  let reloadedFor: string | null = null;
  const deps = {
    running: "a",
    fetchLatest: vi.fn(async () => "b" as string | null),
    readReloadedFor: vi.fn(() => reloadedFor),
    writeReloadedFor: vi.fn((id: string) => {
      reloadedFor = id;
    }),
    reload: vi.fn(),
    ...overrides,
  };
  return { deps, watcher: createReleaseWatcher(deps) };
}

describe("comparing the running release with the server's", () => {
  it("reloads at once for a different release, and remembers which", async () => {
    const { deps, watcher } = watcherWith();
    await watcher.check();
    expect(deps.writeReloadedFor).toHaveBeenCalledWith("b");
    expect(deps.reload).toHaveBeenCalledTimes(1);
    expect(watcher.state).toBe("updating");
  });

  it("leaves a page on the server's release alone", async () => {
    const { deps, watcher } = watcherWith({ fetchLatest: vi.fn(async () => "a") });
    await watcher.check();
    expect(deps.reload).not.toHaveBeenCalled();
    expect(watcher.state).toBe("current");
  });

  it("never takes an unknown answer for a newer release", async () => {
    const { deps, watcher } = watcherWith({ fetchLatest: vi.fn(async () => null) });
    await watcher.check();
    expect(deps.reload).not.toHaveBeenCalled();
    expect(watcher.state).toBe("current");
  });

  it("has nothing to compare when the page carries no release", async () => {
    const { deps, watcher } = watcherWith({ running: null });
    await watcher.check();
    expect(deps.fetchLatest).not.toHaveBeenCalled();
    expect(deps.reload).not.toHaveBeenCalled();
  });

  it("asks the server once for checks that overlap", async () => {
    const { deps, watcher } = watcherWith();
    await Promise.all([watcher.check(), watcher.check()]);
    expect(deps.fetchLatest).toHaveBeenCalledTimes(1);
  });
});

describe("the one reload per release", () => {
  it("holds still when a reload for this release did not bring it", async () => {
    const { deps, watcher } = watcherWith({ readReloadedFor: vi.fn(() => "b") });
    await watcher.check();
    expect(deps.reload).not.toHaveBeenCalled();
    expect(watcher.state).toBe("stuck");
  });

  it("reloads again for a release newer than the one it was stuck on", async () => {
    let latest = "b";
    const { deps, watcher } = watcherWith({ fetchLatest: vi.fn(async () => latest) });
    await watcher.check();
    await watcher.check();
    expect(deps.reload).toHaveBeenCalledTimes(1);
    expect(watcher.state).toBe("stuck");
    latest = "c";
    await watcher.check();
    expect(deps.reload).toHaveBeenCalledTimes(2);
    expect(watcher.state).toBe("updating");
  });

  it("leaves stuck when the server comes back to the page's release", async () => {
    let latest = "b";
    const { watcher } = watcherWith({ readReloadedFor: vi.fn(() => "b"), fetchLatest: vi.fn(async () => latest) });
    await watcher.check();
    expect(watcher.state).toBe("stuck");
    latest = "a";
    await watcher.check();
    expect(watcher.state).toBe("current");
  });

  it("does not reload when the tab cannot remember what it reloaded for", async () => {
    const { deps, watcher } = watcherWith({
      writeReloadedFor: vi.fn(() => {
        throw new Error("storage is full");
      }),
    });
    await watcher.check();
    expect(deps.reload).not.toHaveBeenCalled();
    expect(watcher.state).toBe("stuck");
  });

  it("tells its subscribers when the state changes", async () => {
    const { watcher } = watcherWith();
    const listener = vi.fn();
    watcher.subscribe(listener);
    await watcher.check();
    expect(listener).toHaveBeenCalledTimes(1);
  });
});

describe("the order of an update with a service worker", () => {
  it("has the worker keep the shell before the guard is written and the page reloads", async () => {
    const order: string[] = [];
    const { watcher } = watcherWith({
      keepShell: vi.fn(async () => {
        order.push("keep");
        return true;
      }),
      writeReloadedFor: vi.fn(() => void order.push("remember")),
      reload: vi.fn(() => void order.push("reload")),
    });
    await watcher.check();
    expect(order).toEqual(["keep", "remember", "reload"]);
  });

  it("stays as it is, without using up its one reload, when the worker could not keep the shell", async () => {
    const { deps, watcher } = watcherWith({ keepShell: vi.fn(async () => false) });
    await watcher.check();
    expect(deps.reload).not.toHaveBeenCalled();
    expect(deps.writeReloadedFor).not.toHaveBeenCalled();
    expect(watcher.state).toBe("current");
    deps.keepShell = vi.fn(async () => true);
    await watcher.check();
    expect(deps.reload).toHaveBeenCalledTimes(1);
  });

  it("asks the worker to keep the shell of the release the page already runs", async () => {
    const { deps, watcher } = watcherWith({ fetchLatest: vi.fn(async () => "a"), keepShell: vi.fn(async () => true) });
    await watcher.check();
    expect(deps.keepShell).toHaveBeenCalledTimes(1);
    expect(deps.reload).not.toHaveBeenCalled();
  });

  it("does not ask the worker for a release it was stuck on", async () => {
    const { deps, watcher } = watcherWith({ readReloadedFor: vi.fn(() => "b"), keepShell: vi.fn(async () => true) });
    await watcher.check();
    expect(deps.keepShell).not.toHaveBeenCalled();
    expect(watcher.state).toBe("stuck");
  });
});

function startWatching() {
  let status: ConnectionStatus = "idle";
  const listeners = new Set<() => void>();
  const connection: ConnectionPort = {
    getStatus: () => status,
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
  const doc = Object.assign(new EventTarget(), { visibilityState: "visible" as DocumentVisibilityState });
  const win = new EventTarget();
  let clock = 1_000_000;
  const watcher = { state: "current" as const, subscribe: vi.fn(() => () => undefined), check: vi.fn(async () => undefined) };
  const stop = watchRelease(watcher, { connection, document: doc, window: win, now: () => clock });
  watcher.check.mockClear();
  return {
    watcher,
    stop,
    setStatus(next: ConnectionStatus) {
      status = next;
      for (const listener of listeners) listener();
    },
    show: () => doc.dispatchEvent(new Event("visibilitychange")),
    hide() {
      doc.visibilityState = "hidden";
      doc.dispatchEvent(new Event("visibilitychange"));
      doc.visibilityState = "visible";
    },
    online: () => win.dispatchEvent(new Event("online")),
    advance(ms: number) {
      clock += ms;
    },
  };
}

describe("when the page checks", () => {
  it("checks as it starts", () => {
    let checks = 0;
    const watcher = { state: "current" as const, subscribe: () => () => undefined, check: async () => void (checks += 1) };
    const connection: ConnectionPort = { getStatus: () => "idle", subscribe: () => () => undefined };
    const stop = watchRelease(watcher, { connection, document, window, now: () => 0 });
    stop();
    expect(checks).toBe(1);
  });

  it("checks when the hub connection comes back, and not when it first connects", () => {
    const run = startWatching();
    run.setStatus("connecting");
    run.setStatus("connected");
    expect(run.watcher.check).not.toHaveBeenCalled();
    run.setStatus("reconnecting");
    expect(run.watcher.check).not.toHaveBeenCalled();
    run.setStatus("connected");
    expect(run.watcher.check).toHaveBeenCalledTimes(1);
  });

  it("checks when the browser comes online", () => {
    const run = startWatching();
    run.online();
    run.online();
    expect(run.watcher.check).toHaveBeenCalledTimes(2);
  });

  it("checks when the tab becomes visible, at most once a minute", () => {
    const run = startWatching();
    run.show();
    run.advance(VISIBLE_CHECK_INTERVAL_MS - 1);
    run.show();
    expect(run.watcher.check).toHaveBeenCalledTimes(1);
    run.advance(1);
    run.show();
    expect(run.watcher.check).toHaveBeenCalledTimes(2);
  });

  it("does not check when the tab is hidden", () => {
    const run = startWatching();
    run.hide();
    expect(run.watcher.check).not.toHaveBeenCalled();
  });

  it("stops listening when it is stopped", () => {
    const run = startWatching();
    run.stop();
    run.online();
    run.show();
    run.setStatus("reconnecting");
    run.setStatus("connected");
    expect(run.watcher.check).not.toHaveBeenCalled();
  });
});

describe("asking the server", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    document.head.innerHTML = "";
  });

  const answer = (response: Response | Error) =>
    vi.fn(async () => {
      if (response instanceof Error) throw response;
      return response;
    });

  it("reads the id from the release file, past every cache", async () => {
    const fetchMock = answer(Response.json({ id: "abc" }));
    vi.stubGlobal("fetch", fetchMock);
    await expect(fetchLatestRelease()).resolves.toBe("abc");
    expect(fetchMock).toHaveBeenCalledWith("/release.json", expect.objectContaining({ cache: "no-store" }));
  });

  it.each([
    ["a missing file", new Response("", { status: 404 })],
    ["a page in its place", new Response("<html></html>", { status: 200 })],
    ["a file with no id", Response.json({})],
    ["a network failure", new Error("offline")],
  ])("takes %s for no answer", async (_name, response) => {
    vi.stubGlobal("fetch", answer(response));
    await expect(fetchLatestRelease()).resolves.toBeNull();
  });

  it("reads which release the page is from its document", () => {
    expect(runningRelease()).toBeNull();
    document.head.innerHTML = `<meta name="stellarr-release" content="abc" />`;
    expect(runningRelease()).toBe("abc");
  });
});
