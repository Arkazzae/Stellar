import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { probeMedia } from "./probe";

const objectUrl = "blob:probe-test";

function fileOf(type: string): File {
  return new File(["content"], "upload", { type });
}

beforeEach(() => {
  vi.spyOn(URL, "createObjectURL").mockReturnValue(objectUrl);
  vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("probeMedia for a file that is neither image, audio nor video", () => {
  it("measures nothing and creates no object URL", async () => {
    await expect(probeMedia(fileOf("application/pdf"))).resolves.toEqual({});
    await expect(probeMedia(fileOf(""))).resolves.toEqual({});

    expect(URL.createObjectURL).not.toHaveBeenCalled();
  });
});

describe("probeMedia for an image", () => {
  it("reports the decoded size and releases the bitmap", async () => {
    const close = vi.fn();
    vi.stubGlobal("createImageBitmap", vi.fn(async () => ({ width: 640, height: 480, close })));

    await expect(probeMedia(fileOf("image/png"))).resolves.toEqual({
      width: 640,
      height: 480,
    });
    expect(close).toHaveBeenCalledTimes(1);
  });

  it("reports nothing when the browser cannot decode the file", async () => {
    vi.stubGlobal("createImageBitmap", vi.fn().mockRejectedValue(new Error("bad image")));

    await expect(probeMedia(fileOf("image/heic"))).resolves.toEqual({});
  });

  it("reports nothing when the browser cannot decode images at all", async () => {
    vi.stubGlobal("createImageBitmap", undefined);

    await expect(probeMedia(fileOf("image/png"))).resolves.toEqual({});
  });
});

describe("probeMedia preview of an image", () => {
  const big = (type: string, size = 1000) => new File([new Uint8Array(size)], "photo", { type });

  /** An offscreen canvas that answers each encode request with the next scripted blob. */
  function encodeAs(...answers: Array<{ type: string; size: number } | null>) {
    const convertToBlob = vi.fn(async (_options: { type: string; quality: number }) => {
      const answer = answers.shift() ?? null;
      return answer === null ? null : new Blob([new Uint8Array(answer.size)], { type: answer.type });
    });
    const created: Array<{ width: number; height: number }> = [];
    class FakeCanvas {
      constructor(width: number, height: number) {
        created.push({ width, height });
      }
      getContext() {
        return { drawImage: vi.fn(), imageSmoothingEnabled: false, imageSmoothingQuality: "low" };
      }
      convertToBlob = convertToBlob;
    }
    vi.stubGlobal("OffscreenCanvas", FakeCanvas);
    return { convertToBlob, created };
  }

  function decode(width: number, height: number) {
    const close = vi.fn();
    const create = vi.fn(async (_file: Blob, _options?: unknown) => ({ width, height, close }));
    vi.stubGlobal("createImageBitmap", create);
    return { close, create };
  }

  it("encodes WebP at 0.8 inside 1280 by 1280 and reports the original's size", async () => {
    const { close, create } = decode(4000, 3000);
    const canvas = encodeAs({ type: "image/webp", size: 300 });

    const metadata = await probeMedia(big("image/png"));

    expect(create).toHaveBeenCalledWith(expect.anything(), { imageOrientation: "from-image" });
    expect(canvas.created).toEqual([{ width: 1280, height: 960 }]);
    expect(canvas.convertToBlob).toHaveBeenCalledWith({ type: "image/webp", quality: 0.8 });
    expect(metadata).toMatchObject({ width: 4000, height: 3000 });
    expect(metadata.preview?.type).toBe("image/webp");
    expect(close).toHaveBeenCalledTimes(1);
  });

  it("falls back to JPEG at 0.82 for a JPEG when the browser answers a PNG", async () => {
    decode(2000, 1000);
    const canvas = encodeAs({ type: "image/png", size: 900 }, { type: "image/jpeg", size: 200 });

    const metadata = await probeMedia(big("image/jpeg"));

    expect(canvas.convertToBlob).toHaveBeenNthCalledWith(2, { type: "image/jpeg", quality: 0.82 });
    expect(metadata.preview?.type).toBe("image/jpeg");
  });

  it("sends no preview for a non-JPEG when WebP cannot be encoded", async () => {
    decode(2000, 1000);
    const canvas = encodeAs({ type: "image/png", size: 100 });

    const metadata = await probeMedia(big("image/png"));

    expect(canvas.convertToBlob).toHaveBeenCalledTimes(1);
    expect(metadata).toEqual({ width: 2000, height: 1000 });
  });

  it("sends none when the preview is over 60% of the original or over 1 MiB", async () => {
    decode(2000, 1000);
    encodeAs({ type: "image/webp", size: 601 });
    expect((await probeMedia(big("image/png"))).preview).toBeUndefined();

    encodeAs({ type: "image/webp", size: 1024 * 1024 + 1 });
    expect((await probeMedia(big("image/png", 4 * 1024 * 1024))).preview).toBeUndefined();
  });

  it("makes no preview of an animation or a vector", async () => {
    decode(100, 100);
    const canvas = encodeAs({ type: "image/webp", size: 10 });
    const gif = new File([new Uint8Array([71, 73, 70, 56, 57, 97, 0, 0, 0, 0])], "a.gif", {
      type: "image/gif",
    });

    expect(await probeMedia(gif)).toEqual({ width: 100, height: 100 });
    expect(await probeMedia(big("image/svg+xml"))).toEqual({ width: 100, height: 100 });
    expect(canvas.convertToBlob).not.toHaveBeenCalled();
  });

  it("uploads without a preview when encoding throws or outlasts the budget", async () => {
    decode(2000, 1000);
    const failing = encodeAs();
    failing.convertToBlob.mockRejectedValueOnce(new Error("no"));
    expect(await probeMedia(big("image/png"))).toEqual({ width: 2000, height: 1000 });

    vi.useFakeTimers();
    const slow = encodeAs();
    slow.convertToBlob.mockReturnValue(new Promise(() => {}));
    const pending = probeMedia(big("image/png"));
    await vi.advanceTimersByTimeAsync(10_000);
    await expect(pending).resolves.toEqual({ width: 2000, height: 1000 });
  });
});

interface Playback {
  duration: number;
  videoWidth: number;
  videoHeight: number;
  readyState: number;
  /** What the element does once given a source. */
  outcome: "metadata" | "error" | "nothing";
}

/**
 * jsdom decodes no media, so the element is told what it found. Returns the
 * elements that were given a source, in order.
 */
function playBack(overrides: Partial<Playback> = {}): HTMLMediaElement[] {
  const playback: Playback = {
    duration: 12.5,
    videoWidth: 1920,
    videoHeight: 1080,
    readyState: 4,
    outcome: "metadata",
    ...overrides,
  };
  vi.spyOn(HTMLMediaElement.prototype, "duration", "get").mockReturnValue(playback.duration);
  vi.spyOn(HTMLMediaElement.prototype, "readyState", "get").mockReturnValue(playback.readyState);
  vi.spyOn(HTMLVideoElement.prototype, "videoWidth", "get").mockReturnValue(playback.videoWidth);
  vi.spyOn(HTMLVideoElement.prototype, "videoHeight", "get").mockReturnValue(playback.videoHeight);

  const loaded: HTMLMediaElement[] = [];
  vi.spyOn(HTMLMediaElement.prototype, "load").mockImplementation(function (
    this: HTMLMediaElement,
  ) {
    if (!this.hasAttribute("src")) return;
    loaded.push(this);
    setTimeout(() => {
      if (playback.outcome === "metadata") this.onloadedmetadata?.(new Event("loadedmetadata"));
      if (playback.outcome === "error") this.onerror?.(new Event("error"));
    }, 0);
  });
  return loaded;
}

describe("probeMedia for audio", () => {
  it("reports the duration", async () => {
    playBack({ duration: 83.2 });

    await expect(probeMedia(fileOf("audio/ogg"))).resolves.toEqual({ durationSeconds: 83.2 });
  });

  it.each([Number.NaN, Number.POSITIVE_INFINITY, 0])(
    "leaves out a duration of %s",
    async (duration) => {
      playBack({ duration });

      await expect(probeMedia(fileOf("audio/ogg"))).resolves.toEqual({});
    },
  );

  it("reports nothing for a file the browser cannot read", async () => {
    playBack({ outcome: "error" });

    await expect(probeMedia(fileOf("audio/x-unknown"))).resolves.toEqual({});
  });

  it("gives up after ten seconds without metadata", async () => {
    vi.useFakeTimers();
    playBack({ outcome: "nothing" });

    const result = probeMedia(fileOf("audio/mpeg"));
    let settled = false;
    void result.then(() => {
      settled = true;
    });

    await vi.advanceTimersByTimeAsync(9_999);
    expect(settled).toBe(false);

    await vi.advanceTimersByTimeAsync(1);
    expect(settled).toBe(true);
    await expect(result).resolves.toEqual({});
  });

  it("detaches the file and revokes its URL afterwards", async () => {
    const loaded = playBack();

    await probeMedia(fileOf("audio/ogg"));

    expect(loaded).toHaveLength(1);
    expect(loaded[0]?.hasAttribute("src")).toBe(false);
    expect(URL.revokeObjectURL).toHaveBeenCalledWith(objectUrl);
  });

  it("revokes the URL when the file cannot be read either", async () => {
    playBack({ outcome: "error" });

    await probeMedia(fileOf("audio/ogg"));

    expect(URL.revokeObjectURL).toHaveBeenCalledWith(objectUrl);
  });
});

describe("probeMedia for video", () => {
  function stubPosterCanvas(context: CanvasRenderingContext2D | null) {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(context as never);
    const toBlob = vi
      .spyOn(HTMLCanvasElement.prototype, "toBlob")
      .mockImplementation((callback, type) => callback(new Blob(["frame"], { type })));
    return toBlob;
  }

  function drawingContext() {
    const drawImage = vi.fn();
    return { drawImage, context: { drawImage } as unknown as CanvasRenderingContext2D };
  }

  it("reports duration and size with a JPEG poster of the first frame", async () => {
    const { drawImage, context } = drawingContext();
    const toBlob = stubPosterCanvas(context);
    const loaded = playBack({ videoWidth: 640, videoHeight: 360 });

    const metadata = await probeMedia(fileOf("video/mp4"));

    expect(metadata).toMatchObject({ durationSeconds: 12.5, width: 640, height: 360 });
    expect(metadata.poster).toBeInstanceOf(Blob);
    expect(metadata.poster?.type).toBe("image/jpeg");
    expect(drawImage).toHaveBeenCalledWith(loaded[0], 0, 0, 640, 360);
    expect(toBlob).toHaveBeenCalledWith(expect.any(Function), "image/jpeg", 0.82);
  });

  it.each([
    { name: "landscape", width: 1920, height: 1080, poster: [960, 540] },
    { name: "portrait", width: 1080, height: 1920, poster: [304, 540] },
    { name: "small", width: 320, height: 180, poster: [320, 180] },
  ])(
    "fits a $name poster inside 960x540 without enlarging it",
    async ({ width, height, poster }) => {
      const { drawImage, context } = drawingContext();
      stubPosterCanvas(context);
      const loaded = playBack({ videoWidth: width, videoHeight: height });

      const metadata = await probeMedia(fileOf("video/webm"));

      expect(metadata).toMatchObject({ width, height });
      expect(drawImage).toHaveBeenCalledWith(loaded[0], 0, 0, ...poster);
    },
  );

  it("still reports duration and size when no poster can be drawn", async () => {
    stubPosterCanvas(null);
    playBack();

    const metadata = await probeMedia(fileOf("video/mp4"));

    expect(metadata).toEqual({ durationSeconds: 12.5, width: 1920, height: 1080 });
  });

  it("omits the size of a stream that reports none", async () => {
    stubPosterCanvas(drawingContext().context);
    playBack({ videoWidth: 0, videoHeight: 0 });

    await expect(probeMedia(fileOf("video/mp4"))).resolves.toEqual({ durationSeconds: 12.5 });
  });

  it("waits for the first frame when the metadata arrives before it", async () => {
    vi.useFakeTimers();
    stubPosterCanvas(drawingContext().context);
    const loaded = playBack({ readyState: 0 });

    const result = probeMedia(fileOf("video/mp4"));
    await vi.advanceTimersByTimeAsync(0);
    loaded[0]?.dispatchEvent(new Event("loadeddata"));

    expect((await result).poster).toBeInstanceOf(Blob);
  });

  it("settles for the metadata when the first frame takes over three seconds", async () => {
    vi.useFakeTimers();
    const toBlob = stubPosterCanvas(drawingContext().context);
    playBack({ readyState: 0 });

    const result = probeMedia(fileOf("video/mp4"));
    await vi.advanceTimersByTimeAsync(3_000);

    await expect(result).resolves.toEqual({ durationSeconds: 12.5, width: 1920, height: 1080 });
    expect(toBlob).not.toHaveBeenCalled();
  });
});
