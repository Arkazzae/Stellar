import { describe, expect, it } from "vitest";

import { passwordStrength } from "./passwordStrength";

describe("passwordStrength", () => {
  it("lights no bar below the minimum length", () => {
    expect(passwordStrength("")).toEqual({ score: 0, hint: null });
    expect(passwordStrength("kawa12!")).toEqual({ score: 0, hint: "short" });
  });

  it("rates length above all, with variety helping a password that is long already", () => {
    expect(passwordStrength("kawarower")).toEqual({ score: 1, hint: "length" });
    expect(passwordStrength("kawa rower 2")).toEqual({ score: 2, hint: null });
    expect(passwordStrength("Kawa-row3r")).toEqual({ score: 2, hint: null });
    expect(passwordStrength("kawa rower 2026")).toEqual({ score: 3, hint: null });
    expect(passwordStrength("zielony.most.42")).toEqual({ score: 3, hint: null });
    expect(passwordStrength("zielonymostnadrzeka")).toEqual({ score: 3, hint: null });
  });

  it("keeps a guessable password weak however long it is", () => {
    expect(passwordStrength("MojeHasło2026!!!")).toEqual({ score: 1, hint: "guessable" });
    expect(passwordStrength("qwertyuiop123456")).toEqual({ score: 1, hint: "guessable" });
    expect(passwordStrength("aaaaaaaaaaaaaaaaaa")).toEqual({ score: 1, hint: "guessable" });
    expect(passwordStrength("ola.nowak.ma.kota", "Ola.Nowak")).toEqual({ score: 1, hint: "guessable" });
    // A login too short to mean anything is not looked for.
    expect(passwordStrength("ola.nowak.ma.kota", "ol")).toEqual({ score: 3, hint: null });
  });
});
