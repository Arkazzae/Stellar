import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  GTCRN_WASM_URL,
  GTCRN_WORKLET_URL,
  PROCESSOR_NAME,
  RNNOISE_SIMD_WASM_URL,
  RNNOISE_WASM_URL,
  RNNOISE_WORKLET_URL,
  SPEEX_WASM_URL,
  SPEEX_WORKLET_URL,
  attachSuppressor,
  resetSuppressorCacheForTests,
} from "./noiseSuppression";

/** Stands in for the browser's node; records what it was constructed with. */
class FakeWorkletNode {
  static created: FakeWorkletNode[] = [];
  static failWith: Error | null = null;

  connect = vi.fn();
  disconnect = vi.fn();
  port = { postMessage: vi.fn() };

  constructor(
    readonly context: AudioContext,
    readonly name: string,
    readonly options: AudioWorkletNodeOptions,
  ) {
    if (FakeWorkletNode.failWith) throw FakeWorkletNode.failWith;
    FakeWorkletNode.created.push(this);
  }
}

const binaries = new Map<string, ArrayBuffer>();
const fetchBinary = vi.fn(async (url: string) => {
  const body = new ArrayBuffer(8);
  binaries.set(url, body);
  return { ok: true, status: 200, arrayBuffer: async () => body };
});

function graph() {
  const context = {
    audioWorklet: { addModule: vi.fn(async () => undefined) },
  } as unknown as AudioContext;
  const source = {
    connect: vi.fn(),
    disconnect: vi.fn(),
  } as unknown as MediaStreamAudioSourceNode;
  return { context, source };
}

const lastNode = () => FakeWorkletNode.created.at(-1) as FakeWorkletNode;

beforeEach(() => {
  vi.stubGlobal("AudioWorkletNode", FakeWorkletNode);
  vi.stubGlobal("fetch", fetchBinary);
});

afterEach(() => {
  resetSuppressorCacheForTests();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  FakeWorkletNode.created = [];
  FakeWorkletNode.failWith = null;
  binaries.clear();
  fetchBinary.mockClear();
});

describe("suppressor graph lifecycle", () => {
  it("wires the source into the node and releases exactly once", async () => {
    const { context, source } = graph();
    const handle = await attachSuppressor("rnnoise", context, source);

    expect(handle).not.toBeNull();
    expect(handle?.kind).toBe("rnnoise");
    expect(handle?.output).toBe(lastNode());
    expect(context.audioWorklet.addModule).toHaveBeenCalledWith(RNNOISE_WORKLET_URL);
    expect(source.connect).toHaveBeenCalledWith(lastNode());
    handle?.dispose();
    handle?.dispose();

    expect(source.disconnect).toHaveBeenCalledOnce();
    expect(lastNode().disconnect).toHaveBeenCalledOnce();
    // The processors free their WASM state on this one message.
    expect(lastNode().port.postMessage).toHaveBeenCalledOnce();
    expect(lastNode().port.postMessage).toHaveBeenCalledWith("destroy");
  });

  it.each([
    ["rnnoise", RNNOISE_WORKLET_URL, RNNOISE_SIMD_WASM_URL],
    ["gtcrn", GTCRN_WORKLET_URL, GTCRN_WASM_URL],
    ["speex", SPEEX_WORKLET_URL, SPEEX_WASM_URL],
  ] as const)(
    "starts %s's processor with one channel and the binary it was served",
    async (kind, worklet, binary) => {
      const { context, source } = graph();

      await attachSuppressor(kind, context, source);

      expect(context.audioWorklet.addModule).toHaveBeenCalledWith(worklet);
      expect(fetchBinary).toHaveBeenCalledWith(binary);
      expect(lastNode().context).toBe(context);
      expect(lastNode().name).toBe(PROCESSOR_NAME[kind]);
      expect(lastNode().options).toEqual({
        processorOptions: { maxChannels: 1, wasmBinary: binaries.get(binary) },
      });
      expect(lastNode().options.processorOptions.wasmBinary).toBe(binaries.get(binary));
    },
  );

  it("fetches a binary once per page and registers a worklet once per context", async () => {
    const { context, source } = graph();
    const other = graph();

    await attachSuppressor("gtcrn", context, source);
    await attachSuppressor("gtcrn", context, source);
    await attachSuppressor("gtcrn", other.context, other.source);

    expect(fetchBinary).toHaveBeenCalledOnce();
    expect(context.audioWorklet.addModule).toHaveBeenCalledOnce();
    expect(other.context.audioWorklet.addModule).toHaveBeenCalledOnce();
    expect(FakeWorkletNode.created).toHaveLength(3);
  });
});

describe("choosing the RNNoise build", () => {
  it("takes the SIMD build where this engine validates the probe", async () => {
    // Node runs WebAssembly SIMD, so this also proves the probe is a valid module.
    const { context, source } = graph();

    await attachSuppressor("rnnoise", context, source);

    expect(fetchBinary).toHaveBeenCalledWith(RNNOISE_SIMD_WASM_URL);
  });

  it("takes the plain build where SIMD is refused", async () => {
    vi.spyOn(WebAssembly, "validate").mockReturnValue(false);
    const { context, source } = graph();

    await attachSuppressor("rnnoise", context, source);

    expect(fetchBinary).toHaveBeenCalledWith(RNNOISE_WASM_URL);
    expect(fetchBinary).not.toHaveBeenCalledWith(RNNOISE_SIMD_WASM_URL);
  });

  it("takes the plain build where validation itself throws", async () => {
    vi.spyOn(WebAssembly, "validate").mockImplementation(() => {
      throw new Error("no WebAssembly here");
    });
    const { context, source } = graph();

    await attachSuppressor("rnnoise", context, source);

    expect(fetchBinary).toHaveBeenCalledWith(RNNOISE_WASM_URL);
  });
});

describe("falling back to the browser's suppressor", () => {
  it("cleans a partially connected node when construction fails", async () => {
    const { context, source } = graph();
    (source.connect as ReturnType<typeof vi.fn>).mockImplementationOnce(() => {
      throw new Error("cannot connect");
    });

    await expect(attachSuppressor("speex", context, source)).resolves.toBeNull();

    expect(lastNode().disconnect).toHaveBeenCalledOnce();
    expect(lastNode().port.postMessage).toHaveBeenCalledWith("destroy");
  });

  it("returns null rather than throwing when the worklet module is missing", async () => {
    const { context, source } = graph();
    (context.audioWorklet.addModule as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      new Error("404"),
    );
    await expect(attachSuppressor("gtcrn", context, source)).resolves.toBeNull();
    expect(source.connect).not.toHaveBeenCalled();
  });

  it("returns null when the binary is not served, rather than starting a silent node", async () => {
    fetchBinary.mockResolvedValueOnce({
      ok: false,
      status: 404,
      arrayBuffer: async () => new TextEncoder().encode("<!doctype html>").buffer,
    });
    const { context, source } = graph();

    await expect(attachSuppressor("gtcrn", context, source)).resolves.toBeNull();

    expect(FakeWorkletNode.created).toHaveLength(0);
    expect(source.connect).not.toHaveBeenCalled();
  });

  it("returns null when the processor cannot be constructed", async () => {
    FakeWorkletNode.failWith = new DOMException("not registered", "InvalidStateError");
    const { context, source } = graph();

    await expect(attachSuppressor("speex", context, source)).resolves.toBeNull();

    expect(source.connect).not.toHaveBeenCalled();
  });

  it("returns null where the browser has no AudioWorklet", async () => {
    vi.unstubAllGlobals();
    vi.stubGlobal("fetch", fetchBinary);
    const { source } = graph();
    const context = {} as AudioContext;

    await expect(attachSuppressor("rnnoise", context, source)).resolves.toBeNull();

    expect(source.connect).not.toHaveBeenCalled();
  });
});
