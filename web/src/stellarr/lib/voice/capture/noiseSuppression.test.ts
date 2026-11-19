import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  GTCRN_WASM_URL,
  GTCRN_WORKLET_URL,
  PROCESSOR_NAME,
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

  /** What RNNoise's processor does after construction: say it is ready, fail, or say nothing. */
  static rnnoise: "ready" | "error" | "silent" = "ready";

  connect = vi.fn();
  disconnect = vi.fn();
  port: { postMessage: ReturnType<typeof vi.fn>; onmessage: ((event: { data: unknown }) => void) | null } = {
    postMessage: vi.fn(),
    onmessage: null,
  };
  onprocessorerror: (() => void) | null = null;

  constructor(
    readonly context: AudioContext,
    readonly name: string,
    readonly options: AudioWorkletNodeOptions,
  ) {
    if (FakeWorkletNode.failWith) throw FakeWorkletNode.failWith;
    FakeWorkletNode.created.push(this);
    if (name === PROCESSOR_NAME.rnnoise && FakeWorkletNode.rnnoise !== "silent") {
      const ready = FakeWorkletNode.rnnoise === "ready";
      queueMicrotask(() => (ready ? this.port.onmessage?.({ data: "ready" }) : this.onprocessorerror?.()));
    }
  }
}

const compiled = {} as WebAssembly.Module;
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
  // The fake binary is eight bytes of nothing; RNNoise's is compiled before the node exists.
  vi.spyOn(WebAssembly, "compile").mockImplementation(async () => compiled);
});

afterEach(() => {
  resetSuppressorCacheForTests();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  FakeWorkletNode.created = [];
  FakeWorkletNode.failWith = null;
  FakeWorkletNode.rnnoise = "ready";
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

describe("starting RNNoise", () => {
  it("hands the processor the binary compiled, one channel's worth of nothing else", async () => {
    const { context, source } = graph();

    const handle = await attachSuppressor("rnnoise", context, source);

    expect(handle?.kind).toBe("rnnoise");
    expect(context.audioWorklet.addModule).toHaveBeenCalledWith(RNNOISE_WORKLET_URL);
    expect(fetchBinary).toHaveBeenCalledWith(RNNOISE_WASM_URL);
    expect(WebAssembly.compile).toHaveBeenCalledWith(binaries.get(RNNOISE_WASM_URL));
    expect(lastNode().name).toBe(PROCESSOR_NAME.rnnoise);
    expect(lastNode().options).toEqual({ processorOptions: { module: compiled } });
    expect(source.connect).toHaveBeenCalledWith(lastNode());
  });

  it("returns null, wiring nothing, when the binary does not compile", async () => {
    vi.spyOn(WebAssembly, "compile").mockRejectedValue(new WebAssembly.CompileError("not a module"));
    const { context, source } = graph();

    await expect(attachSuppressor("rnnoise", context, source)).resolves.toBeNull();

    expect(FakeWorkletNode.created).toHaveLength(0);
    expect(source.connect).not.toHaveBeenCalled();
  });

  it("returns null, and frees the node, when the processor never says it is ready", async () => {
    vi.useFakeTimers();
    try {
      FakeWorkletNode.rnnoise = "silent";
      const { context, source } = graph();

      const attached = attachSuppressor("rnnoise", context, source);
      await vi.advanceTimersByTimeAsync(3000);

      await expect(attached).resolves.toBeNull();
      expect(source.connect).not.toHaveBeenCalled();
      expect(lastNode().port.postMessage).toHaveBeenCalledWith("destroy");
      expect(lastNode().disconnect).toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("returns null, and frees the node, when the processor fails to start", async () => {
    FakeWorkletNode.rnnoise = "error";
    const { context, source } = graph();

    await expect(attachSuppressor("rnnoise", context, source)).resolves.toBeNull();

    expect(source.connect).not.toHaveBeenCalled();
    expect(lastNode().port.postMessage).toHaveBeenCalledWith("destroy");
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
