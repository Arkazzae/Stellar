import { beforeEach, describe, expect, it, vi } from "vitest";

const appModule = vi.hoisted(() => ({ AppContainer: () => null }));
const signedInCode = vi.hoisted(() => ({ loadSignedInCode: vi.fn() }));

vi.mock("@/containers/AppContainer", () => appModule);
vi.mock("@/adapters/signedInLoader", () => signedInCode);

describe("loadApp", () => {
  beforeEach(() => {
    vi.resetModules();
    signedInCode.loadSignedInCode.mockReset();
  });
  it("resolves to the app module once the signed-in code is there", async () => {
    signedInCode.loadSignedInCode.mockResolvedValueOnce({});

    const { loadApp } = await import("./loadApp");
    const request = loadApp();
    expect(loadApp()).toBe(request);
    const app = await request;
    expect(loadApp()).toBe(request);
    expect(signedInCode.loadSignedInCode).toHaveBeenCalledOnce();

    expect(app.AppContainer).toBe(appModule.AppContainer);
  });

  it("fails when the signed-in code does not arrive", async () => {
    const failure = new Error("network down");
    signedInCode.loadSignedInCode.mockImplementationOnce(() => Promise.reject(failure));

    const { loadApp } = await import("./loadApp");
    await expect(loadApp()).rejects.toBe(failure);
    signedInCode.loadSignedInCode.mockResolvedValueOnce({});
    expect((await loadApp()).AppContainer).toBe(appModule.AppContainer);
  });
});
