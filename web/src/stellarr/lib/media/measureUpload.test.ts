import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(() => {
  vi.doUnmock("./probe");
  vi.resetModules();
});

describe("measureUpload", () => {
  const file = new File(["x"], "a.png", { type: "image/png" });

  it("loads the probe when a file is measured and returns what it found", async () => {
    vi.doMock("./probe", () => ({ probeMedia: async () => ({ width: 5, height: 4, placeholder: "AAAA" }) }));
    const { measureUpload } = await import("./measureUpload");

    expect(await measureUpload(file)).toEqual({ width: 5, height: 4, placeholder: "AAAA" });
  });

  it("measures nothing, and does not fail, when the probe cannot be loaded", async () => {
    vi.doMock("./probe", () => {
      throw new Error("chunk failed to load");
    });
    const { measureUpload } = await import("./measureUpload");

    expect(await measureUpload(file)).toEqual({});
  });
});
