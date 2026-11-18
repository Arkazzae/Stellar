import { describe, expect, it } from "vitest";

import { isValidUsername, suggestUsername } from "./username";

describe("suggestUsername", () => {
  it("makes a plain login of a display name", () => {
    expect(suggestUsername("Ola Nowak")).toBe("ola.nowak");
    expect(suggestUsername("Łukasz Żółć")).toBe("lukasz.zolc");
    expect(suggestUsername("  Zażółć   Gęślą Jaźń ")).toBe("zazolc.gesla.jazn");
  });

  it("leaves out what a login cannot hold and keeps no separator at the ends or doubled", () => {
    expect(suggestUsername("Ada (admin) ✨")).toBe("ada.admin");
    expect(suggestUsername("Ola - Nowak")).toBe("ola.nowak");
    expect(suggestUsername("__x__")).toBe("x");
    expect(suggestUsername("🚀")).toBe("");
  });

  it("stays within the server's 32 characters without ending on a separator", () => {
    const suggestion = suggestUsername("Aleksandra Konstantynopolitańczykowianeczka");
    expect(suggestion).toBe("aleksandra.konstantynopolitanczy");
    expect(suggestUsername("abcdefghijklmnopqrstuvwxyzabcde f")).toBe("abcdefghijklmnopqrstuvwxyzabcde");
  });
});

describe("isValidUsername", () => {
  it("takes what the server takes", () => {
    expect(isValidUsername("ola.nowak")).toBe(true);
    expect(isValidUsername("żaba_1")).toBe(true);
    expect(isValidUsername("ol")).toBe(false);
    expect(isValidUsername("ola nowak")).toBe(false);
    expect(isValidUsername("a".repeat(33))).toBe(false);
  });
});
