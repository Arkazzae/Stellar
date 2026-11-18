import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { WorkletSuppressor } from "../audioProfiles";
import type { NoiseSuppressionMode } from "../settings";
import type { SuppressorDownload } from "../types";
import type { SuppressorDownloadEvent } from "./noiseSuppression";
import { DOWNLOAD_SHOWN_AFTER_MS, SuppressorDownloads } from "./suppressorDownload";

const URL_OF: Record<WorkletSuppressor, string> = {
  rnnoise: "/audio/rnnoise_simd.wasm",
  gtcrn: "/audio/gtcrn.wasm",
  speex: "/audio/speex.wasm",
};

function harness(
  initial: NoiseSuppressionMode = "browser",
  sizes: Record<string, number> = { [URL_OF.gtcrn]: 1000, [URL_OF.speex]: 1000 },
) {
  let watcher: ((event: SuppressorDownloadEvent) => void) | null = null;
  const shown: Array<SuppressorDownload | null> = [];
  const preload = vi.fn(async (_kind: WorkletSuppressor) => true);
  const downloads = new SuppressorDownloads(initial, {
    publish: (download) => shown.push(download),
    preload,
    watch: (listener) => {
      watcher = listener;
      return () => {
        watcher = null;
      };
    },
    sizeOf: (url) => sizes[url],
  });
  const emit = (kind: WorkletSuppressor, phase: SuppressorDownloadEvent["phase"], received = 0) =>
    watcher?.({ kind, url: URL_OF[kind], received, phase });
  return { downloads, shown, preload, emit, watching: () => watcher !== null };
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("SuppressorDownloads", () => {
  it("fetches the engine a person settles on, and not the one in effect from the start", () => {
    const { downloads, preload } = harness("rnnoise");

    downloads.choose("rnnoise");
    downloads.choose("browser");
    expect(preload).not.toHaveBeenCalled();

    downloads.choose("gtcrn");
    expect(preload).toHaveBeenCalledExactlyOnceWith("gtcrn");
  });

  it("shows a download only once it has run past the delay, as its share of the recorded size", () => {
    const { downloads, emit } = harness();
    downloads.choose("gtcrn");
    emit("gtcrn", "loading", 0);
    emit("gtcrn", "loading", 250);

    vi.advanceTimersByTime(DOWNLOAD_SHOWN_AFTER_MS - 1);
    expect(downloads.current).toBeNull();

    vi.advanceTimersByTime(1);
    expect(downloads.current).toEqual({ kind: "gtcrn", percent: 25 });

    emit("gtcrn", "loading", 404);
    expect(downloads.current).toEqual({ kind: "gtcrn", percent: 40 });

    emit("gtcrn", "done", 1000);
    expect(downloads.current).toBeNull();
  });

  it("never shows a download that ends within the delay", () => {
    const { downloads, shown, emit } = harness();
    downloads.choose("speex");
    emit("speex", "loading", 0);
    vi.advanceTimersByTime(80);
    emit("speex", "loading", 1000);
    emit("speex", "done", 1000);

    vi.advanceTimersByTime(DOWNLOAD_SHOWN_AFTER_MS * 2);
    expect(shown.every((download) => download === null)).toBe(true);
  });

  it("stops showing a download the moment it fails", () => {
    const { downloads, emit } = harness();
    downloads.choose("gtcrn");
    emit("gtcrn", "loading", 300);
    vi.advanceTimersByTime(DOWNLOAD_SHOWN_AFTER_MS);
    expect(downloads.current).not.toBeNull();

    emit("gtcrn", "failed");
    expect(downloads.current).toBeNull();
  });

  it("never passes 100 or goes back, and gives no percent where the total is not known", () => {
    const { downloads, emit } = harness("browser", { [URL_OF.gtcrn]: 1000 });
    downloads.choose("gtcrn");
    emit("gtcrn", "loading", 600);
    vi.advanceTimersByTime(DOWNLOAD_SHOWN_AFTER_MS);
    expect(downloads.current?.percent).toBe(60);

    // More bytes than recorded: whatever the total is, it is not the manifest's.
    emit("gtcrn", "loading", 1200);
    expect(downloads.current).toEqual({ kind: "gtcrn", percent: null });
    emit("gtcrn", "loading", 1300);
    expect(downloads.current?.percent).toBeNull();

    downloads.choose("rnnoise");
    emit("rnnoise", "loading", 5000);
    vi.advanceTimersByTime(DOWNLOAD_SHOWN_AFTER_MS);
    expect(downloads.current).toEqual({ kind: "rnnoise", percent: null });
  });

  it("shows only the chosen engine, while one left behind finishes unseen", () => {
    const { downloads, emit } = harness();
    downloads.choose("gtcrn");
    emit("gtcrn", "loading", 100);
    vi.advanceTimersByTime(DOWNLOAD_SHOWN_AFTER_MS);

    downloads.choose("speex");
    expect(downloads.current).toBeNull();
    emit("speex", "loading", 0);
    emit("gtcrn", "loading", 900);
    expect(downloads.current).toBeNull();

    vi.advanceTimersByTime(DOWNLOAD_SHOWN_AFTER_MS);
    emit("speex", "loading", 500);
    expect(downloads.current).toEqual({ kind: "speex", percent: 50 });

    // Back to the first, still under way: its ring returns where it got to.
    downloads.choose("gtcrn");
    expect(downloads.current).toEqual({ kind: "gtcrn", percent: 90 });
  });

  it("stops listening and clears what it showed when disposed", () => {
    const { downloads, emit, watching } = harness();
    downloads.choose("gtcrn");
    emit("gtcrn", "loading", 100);
    vi.advanceTimersByTime(DOWNLOAD_SHOWN_AFTER_MS);

    downloads.dispose();
    expect(watching()).toBe(false);
    expect(downloads.current).toBeNull();
  });
});
