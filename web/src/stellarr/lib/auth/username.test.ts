import { describe, expect, it } from "vitest";

import { accountProblem } from "@/components/auth/AccountFields";
import type { Translator } from "@/i18n";

import { isReservedUsername, isValidUsername, suggestUsername } from "./username";

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

describe("isReservedUsername", () => {
  it("keeps the words that call on a channel, in either language and any case", () => {
    for (const word of ["everyone", "Wszyscy", "HERE", "obecni"]) {
      expect(isReservedUsername(word), word).toBe(true);
      // The form says what the server would answer before it is asked.
      expect(isValidUsername(word), word).toBe(true);
    }
    for (const name of ["everyone2", "hereford", "obecnie", "ola"]) {
      expect(isReservedUsername(name), name).toBe(false);
    }
  });

  it("is answered by the form as a login somebody has", () => {
    const t: Translator = (key) => key;
    const values = { displayName: "Ola", username: "Wszyscy", email: "ola@example.test", password: "long enough password" };

    expect(accountProblem(values, t)).toEqual({ field: "username", message: "auth.register.usernameTaken" });
    expect(accountProblem({ ...values, username: "ola" }, t)).toBeNull();
  });
});
