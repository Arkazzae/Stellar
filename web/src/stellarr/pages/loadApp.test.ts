import { describe, expect, it, vi } from "vitest";

import { loadApp } from "./loadApp";

const appModule = vi.hoisted(() => ({ AppContainer: () => null }));
const signedInCode = vi.hoisted(() => ({ loadSignedInCode: vi.fn() }));

vi.mock("@/containers/AppContainer", () => appModule);
vi.mock("@/adapters/signedInLoader", () => signedInCode);

describe("loadApp", () => {
  it("resolves to the app module once the signed-in code is there", async () => {
    signedInCode.loadSignedInCode.mockResolvedValueOnce({});

    const app = await loadApp();

    expect(app.AppContainer).toBe(appModule.AppContainer);
  });

  it("fails when the signed-in code does not arrive", async () => {
    const failure = new Error("network down");
    signedInCode.loadSignedInCode.mockImplementationOnce(() => Promise.reject(failure));

    await expect(loadApp()).rejects.toBe(failure);
  });
});
