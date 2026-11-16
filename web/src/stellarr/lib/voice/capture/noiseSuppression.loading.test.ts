import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { attachSuppressor, resetSuppressorCacheForTests } from "./noiseSuppression";

const served = () => ({ ok: true, status: 200, arrayBuffer: async () => new ArrayBuffer(8) });
const fetchBinary = vi.fn(async (_url: string) => served());
let created = 0;

class FakeWorkletNode {
  connect = vi.fn();
  disconnect = vi.fn();
  port = { postMessage: vi.fn() };
  constructor() {
    created += 1;
  }
}

function graph() {
  const context = {
    audioWorklet: { addModule: vi.fn(async () => undefined) },
  } as unknown as AudioContext;
  const source = { connect: vi.fn(), disconnect: vi.fn() } as unknown as AudioNode;
  return { context, source };
}

beforeEach(() => {
  vi.stubGlobal("AudioWorkletNode", FakeWorkletNode);
  vi.stubGlobal("fetch", fetchBinary);
});

afterEach(() => {
  resetSuppressorCacheForTests();
  vi.unstubAllGlobals();
  fetchBinary.mockReset();
  fetchBinary.mockImplementation(async () => served());
  created = 0;
});

describe("suppressor loading", () => {
  it("shares one download and one registration between simultaneous captures", async () => {
    const { context, source } = graph();

    const handles = await Promise.all([
      attachSuppressor("gtcrn", context, source),
      attachSuppressor("gtcrn", context, source),
    ]);

    expect(handles.every((handle) => handle !== null)).toBe(true);
    expect(fetchBinary).toHaveBeenCalledOnce();
    expect(context.audioWorklet.addModule).toHaveBeenCalledOnce();
    expect(created).toBe(2);
  });

  it("retries a failed binary download on the next attach", async () => {
    const { context, source } = graph();
    fetchBinary.mockRejectedValueOnce(new Error("offline"));

    await expect(attachSuppressor("gtcrn", context, source)).resolves.toBeNull();
    expect(source.connect).not.toHaveBeenCalled();

    const handle = await attachSuppressor("gtcrn", context, source);
    expect(handle?.kind).toBe("gtcrn");
    expect(fetchBinary).toHaveBeenCalledTimes(2);
  });

  it("retries a binary the server refused on the next attach", async () => {
    const { context, source } = graph();
    fetchBinary.mockResolvedValueOnce({ ...served(), ok: false, status: 503 });

    await expect(attachSuppressor("speex", context, source)).resolves.toBeNull();
    await expect(attachSuppressor("speex", context, source)).resolves.not.toBeNull();
    expect(fetchBinary).toHaveBeenCalledTimes(2);
  });

  it("registers the worklet again after a failed registration", async () => {
    const { context, source } = graph();
    (context.audioWorklet.addModule as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      new Error("404"),
    );

    await expect(attachSuppressor("gtcrn", context, source)).resolves.toBeNull();
    await expect(attachSuppressor("gtcrn", context, source)).resolves.not.toBeNull();
    expect(context.audioWorklet.addModule).toHaveBeenCalledTimes(2);
  });
});
