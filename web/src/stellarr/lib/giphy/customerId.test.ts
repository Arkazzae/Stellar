import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { GIPHY_CUSTOMER_ID_KEY } from "./customerId";

/** The module keeps the id it made, so each test loads it afresh. */
async function freshModule() {
  vi.resetModules();
  return await import("./customerId");
}

beforeEach(() => {
  window.localStorage.clear();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("giphyCustomerId", () => {
  it("is made up at random and kept in this browser", async () => {
    const { giphyCustomerId } = await freshModule();

    const id = giphyCustomerId();

    expect(id).toMatch(/^[0-9a-f]{32}$/);
    expect(window.localStorage.getItem(GIPHY_CUSTOMER_ID_KEY)).toBe(id);
    expect(giphyCustomerId()).toBe(id);
  });

  it("stays the same across page loads", async () => {
    const first = (await freshModule()).giphyCustomerId();
    const second = (await freshModule()).giphyCustomerId();

    expect(second).toBe(first);
  });

  it("is not the same for two browsers", async () => {
    const first = (await freshModule()).giphyCustomerId();
    window.localStorage.clear();
    const second = (await freshModule()).giphyCustomerId();

    expect(second).not.toBe(first);
  });

  it("replaces a stored value that is not one of its own", async () => {
    window.localStorage.setItem(GIPHY_CUSTOMER_ID_KEY, "u-maja");
    const { giphyCustomerId } = await freshModule();

    expect(giphyCustomerId()).toMatch(/^[0-9a-f]{32}$/);
  });

  it("holds for the page when the browser refuses storage", async () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("denied");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("denied");
    });
    const { giphyCustomerId } = await freshModule();

    const id = giphyCustomerId();
    expect(id).toMatch(/^[0-9a-f]{32}$/);
    expect(giphyCustomerId()).toBe(id);
  });
});
