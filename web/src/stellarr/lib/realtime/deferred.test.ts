import { describe, expect, it, vi } from "vitest";

import { FakeRealtimeClient } from "@/test/fakes";

import { DeferredRealtimeClient } from "./deferred";
import type { RealtimeClient } from "./types";

/** A load the test settles by hand, as a chunk that arrives later. */
function pendingLoad() {
  const settle = { resolve: (_client: RealtimeClient): void => undefined, reject: (_cause: unknown): void => undefined };
  const load = vi.fn(
    () =>
      new Promise<RealtimeClient>((resolve, reject) => {
        settle.resolve = resolve;
        settle.reject = reject;
      }),
  );
  return { load, settle };
}

describe("a realtime client loaded on first use", () => {
  it("delivers events to a subscription made before the implementation arrived", async () => {
    const pending = pendingLoad();
    const deferred = new DeferredRealtimeClient(pending.load);
    const received = vi.fn();
    const cancelled = vi.fn();
    deferred.on("SocialChanged", received);
    const cancel = deferred.on("EmojiChanged", cancelled);
    cancel();
    const started = deferred.start();
    const real = new FakeRealtimeClient();
    pending.settle.resolve(real);
    await started;
    real.push("SocialChanged", {});
    real.push("EmojiChanged", {});

    expect(received).toHaveBeenCalledTimes(1);
    expect(cancelled).not.toHaveBeenCalled();
    expect(deferred.getStatus()).toBe("connected");
  });

  it("hands the implementation what was said before it loaded", async () => {
    const pending = pendingLoad();
    const deferred = new DeferredRealtimeClient(pending.load);
    const listener = vi.fn();
    const configureRtc = vi.fn();
    const real = Object.assign(new FakeRealtimeClient(), { configureRtc });

    const hub = { signalingHub: "rtc-hub" };
    deferred.subscribeStatus(listener);
    deferred.configureRtc(hub);
    const started = deferred.start();
    pending.settle.resolve(real);
    await started;

    expect(configureRtc).toHaveBeenCalledWith(hub);
    expect(listener).toHaveBeenCalled();
  });

  it("keeps a stop after a start in that order", async () => {
    const pending = pendingLoad();
    const deferred = new DeferredRealtimeClient(pending.load);
    const real = new FakeRealtimeClient();
    const calls: string[] = [];
    const note = (name: string) => async () => {
      calls.splice(calls.length, 0, name);
    };
    real.start = note("first");
    real.stop = note("second");

    const done = Promise.all([deferred.start(), deferred.stop()]);
    pending.settle.resolve(real);
    await done;

    expect(calls).toEqual(["first", "second"]);
  });

  it("does not load the implementation just to stop it", async () => {
    const load = vi.fn();

    await new DeferredRealtimeClient(load).stop();

    expect(load).not.toHaveBeenCalled();
  });

  it("reports a failed load, shows a dead socket and tries again on the next call", async () => {
    const pending = pendingLoad();
    const deferred = new DeferredRealtimeClient(pending.load);
    const listener = vi.fn();
    deferred.subscribeStatus(listener);

    const first = deferred.start();
    pending.settle.reject(new Error("network down"));

    await expect(first).rejects.toThrow("network down");
    expect(deferred.getStatus()).toBe("disconnected");
    expect(listener).toHaveBeenCalled();

    const second = deferred.start();
    pending.settle.resolve(new FakeRealtimeClient());
    await second;
    expect(pending.load).toHaveBeenCalledTimes(2);
    expect(deferred.getStatus()).toBe("connected");
  });
});
