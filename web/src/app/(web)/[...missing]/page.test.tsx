import { describe, expect, it, vi } from "vitest";

import UnknownAddress from "./page";

vi.mock("next/navigation", () => ({
  notFound: () => {
    throw new Error("NEXT_NOT_FOUND");
  },
}));

describe("an address no route claims", () => {
  it("is answered by the web client's not-found page", () => {
    expect(() => UnknownAddress()).toThrow("NEXT_NOT_FOUND");
  });
});
